//! A room the Matrix library records as unencrypted is confirmed with the
//! server before anything is sent to it.
//!
//! The library takes a room's encryption state as known as soon as a sync
//! response that asked for `m.room.encryption` arrives without it, and it
//! never asks again. A new room that reaches this client before its first
//! state events do (seen with rooms this client had just created) is then
//! recorded as unencrypted for good, and messages to it went out in plain
//! text. So the first send to such a room in a session asks the server for
//! the room's `m.room.encryption`. If the server has it, the record is
//! corrected and the members are read again, which makes the library track
//! their devices so the room key reaches them. Rooms are also checked in the
//! background when they are first listed.

use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

use makepad_widgets::{log, warning};
use matrix_sdk::{
    EncryptionState, Room,
    config::RequestConfig,
    ruma::{
        OwnedRoomId, OwnedUserId,
        api::{client::state::get_state_event_for_key, error::ErrorKind},
        events::{StateEventType, room::encryption::PossiblyRedactedRoomEncryptionEventContent},
    },
};
use matrix_sdk_base::RoomInfoNotableUpdateReasons;
use tokio::sync::Semaphore;

/// Shown when a message is not sent because the check failed.
pub(crate) const UNCONFIRMED: &str =
    "Could not check whether this room is encrypted. Your message was not sent. Try again.";

/// Rooms whose "not encrypted" record the server confirmed in this session,
/// by account.
static CONFIRMED: LazyLock<Mutex<HashSet<(OwnedUserId, OwnedRoomId)>>> =
    LazyLock::new(Default::default);

/// Background checks run two at a time.
static BACKGROUND: Semaphore = Semaphore::const_new(2);

fn key(room: &Room) -> (OwnedUserId, OwnedRoomId) {
    (room.own_user_id().to_owned(), room.room_id().to_owned())
}

fn confirmed(room: &Room) -> bool {
    CONFIRMED.lock().is_ok_and(|rooms| rooms.contains(&key(room)))
}

fn remember(room: &Room) {
    if let Ok(mut rooms) = CONFIRMED.lock() {
        rooms.insert(key(room));
    }
}

/// The room's encryption state to send by. A "not encrypted" record is
/// confirmed with the server once per session; an unknown state is asked for
/// as the library does. An error means the state is not confirmed, so
/// nothing may be sent.
pub(crate) async fn confirmed_state(room: &Room) -> matrix_sdk::Result<EncryptionState> {
    let state = room.encryption_state();
    if state.is_unknown() {
        let state = room.latest_encryption_state().await?;
        if matches!(state, EncryptionState::NotEncrypted) {
            remember(room);
        }
        return Ok(state);
    }
    if !matches!(state, EncryptionState::NotEncrypted) || confirmed(room) {
        return Ok(state);
    }
    let request = get_state_event_for_key::v3::Request::new(
        room.room_id().to_owned(),
        StateEventType::RoomEncryption,
        String::new(),
    );
    // A sender waits for this answer, so a failing server is retried only a
    // few times (the library's default retries without end).
    let sent = room.client().send(request).with_request_config(RequestConfig::short_retry());
    let content = match sent.await {
        Ok(response) => response
            .into_content()
            .deserialize_as_unchecked::<PossiblyRedactedRoomEncryptionEventContent>()?,
        Err(error) if error.client_api_error_kind() == Some(&ErrorKind::NotFound) => {
            remember(room);
            return Ok(EncryptionState::NotEncrypted);
        }
        Err(error) => return Err(error.into()),
    };
    warning!(
        "Room {} was recorded as unencrypted but is encrypted on the server; correcting the record.",
        room.room_id()
    );
    // Written under the store lock that sync processing holds, so a sync in
    // progress cannot put the old record back. The event stays: later syncs
    // only mark the state as known.
    room.update_and_save_room_info(|mut info| {
        info.set_encryption_event(Some(content));
        info.mark_encryption_state_synced();
        info.mark_members_missing();
        (info, RoomInfoNotableUpdateReasons::empty())
    })
    .await?;
    // Now that the room counts as encrypted, reading its members makes the
    // library track their devices. The message is encrypted either way.
    if let Err(error) = room.sync_members().await {
        warning!(
            "Could not read the members of room {} after correcting its encryption: {error}",
            room.room_id()
        );
    }
    Ok(room.encryption_state())
}

/// Checks a newly listed room that is recorded as unencrypted, so it shows
/// and behaves as encrypted before anyone sends to it.
pub(crate) fn check_in_background(room: Room) {
    if !matches!(room.encryption_state(), EncryptionState::NotEncrypted) || confirmed(&room) {
        return;
    }
    crate::account_session::spawn(async move {
        let Ok(_permit) = BACKGROUND.acquire().await else {
            return;
        };
        if let Err(error) = confirmed_state(&room).await {
            log!(
                "Could not check the encryption of room {}; the next send asks again: {error}",
                room.room_id()
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::Client;
    use std::time::Duration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, path_regex},
    };

    const USER: &str = "@alice:example.org";

    /// A client with one joined room that the library records as unencrypted,
    /// the way a sliding sync response without the room's state leaves it.
    async fn recorded_unencrypted(server: &MockServer, room: &str) -> (Client, Room) {
        let joined = serde_json::json!({ room: {
            "state": {"events": [
                {"type":"m.room.create","state_key":"","event_id":"$create","sender":USER,"origin_server_ts":1,"content":{"creator":USER,"room_version":"10"}},
                {"type":"m.room.member","state_key":USER,"event_id":"$join","sender":USER,"origin_server_ts":2,"content":{"membership":"join"}}
            ]},
            "timeline": {"events": [], "limited": false, "prev_batch": "older"}
        }});
        Mock::given(method("GET"))
            .and(path_regex(".*/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"next_batch": "initial", "rooms": {"join": joined}}),
            ))
            .mount(server)
            .await;
        let client = Client::builder()
            .homeserver_url(server.uri())
            .server_versions([matrix_sdk::ruma::api::MatrixVersion::V1_11])
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .restore_session(
                serde_json::from_value(serde_json::json!({
                    "access_token": "fixture", "user_id": USER, "device_id": "ENCRYPTIONCHECK"
                }))
                .unwrap(),
                Default::default(),
            )
            .await
            .unwrap();
        client
            .sync_once(matrix_sdk::config::SyncSettings::default().timeout(Duration::ZERO))
            .await
            .unwrap();
        let room = client.get_room(&OwnedRoomId::try_from(room).unwrap()).unwrap();
        room.update_and_save_room_info(|mut info| {
            info.mark_encryption_state_synced();
            (info, RoomInfoNotableUpdateReasons::empty())
        })
        .await
        .unwrap();
        assert!(matches!(room.encryption_state(), EncryptionState::NotEncrypted));
        (client, room)
    }

    fn state_path(room: &str) -> String {
        format!("/_matrix/client/v3/rooms/{room}/state/m.room.encryption/")
    }

    fn run(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(test);
    }

    #[test]
    fn an_encrypted_room_recorded_as_unencrypted_is_corrected_and_its_members_tracked() {
        run(async {
            let server = MockServer::start().await;
            let room_id = "!corrected:example.org";
            let (client, room) = recorded_unencrypted(&server, room_id).await;
            Mock::given(method("GET"))
                .and(path(state_path(room_id)))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"algorithm": "m.megolm.v1.aes-sha2"}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            let bob = "@bob:example.org";
            Mock::given(method("GET"))
                .and(path(format!("/_matrix/client/v3/rooms/{room_id}/members")))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"chunk": [
                    {"type":"m.room.member","room_id":room_id,"state_key":USER,"event_id":"$join","sender":USER,"origin_server_ts":2,"content":{"membership":"join"}},
                    {"type":"m.room.member","room_id":room_id,"state_key":bob,"event_id":"$bob","sender":bob,"origin_server_ts":3,"content":{"membership":"join"}}
                ]})))
                .expect(1)
                .mount(&server)
                .await;

            // The fault: the library alone keeps the record and never asks the
            // server (the state mock allows one question, the one below).
            assert!(matches!(
                room.latest_encryption_state().await.unwrap(),
                EncryptionState::NotEncrypted
            ));
            let state = confirmed_state(&room).await.unwrap();
            assert!(state.is_encrypted());
            assert!(room.encryption_state().is_encrypted());
            // The corrected record is final: no second server question.
            assert!(confirmed_state(&room).await.unwrap().is_encrypted());
            let tracked = client.encryption().tracked_users().await.unwrap();
            assert!(tracked.iter().any(|user| user.as_str() == bob), "{tracked:?}");
        });
    }

    #[test]
    fn a_room_the_server_confirms_unencrypted_is_asked_once_per_session() {
        run(async {
            let server = MockServer::start().await;
            let room_id = "!plain:example.org";
            let (_client, room) = recorded_unencrypted(&server, room_id).await;
            Mock::given(method("GET"))
                .and(path(state_path(room_id)))
                .respond_with(ResponseTemplate::new(404).set_body_json(
                    serde_json::json!({"errcode": "M_NOT_FOUND", "error": "Event not found."}),
                ))
                .expect(1)
                .mount(&server)
                .await;

            assert!(matches!(confirmed_state(&room).await.unwrap(), EncryptionState::NotEncrypted));
            assert!(matches!(confirmed_state(&room).await.unwrap(), EncryptionState::NotEncrypted));
        });
    }

    #[test]
    fn a_refused_check_stops_the_send_and_is_asked_again() {
        run(async {
            let server = MockServer::start().await;
            let room_id = "!refused:example.org";
            let (_client, room) = recorded_unencrypted(&server, room_id).await;
            Mock::given(method("GET"))
                .and(path(state_path(room_id)))
                .respond_with(ResponseTemplate::new(403).set_body_json(
                    serde_json::json!({"errcode": "M_FORBIDDEN", "error": "Not allowed."}),
                ))
                .expect(2)
                .mount(&server)
                .await;

            assert!(confirmed_state(&room).await.is_err());
            assert!(!confirmed(&room));
            assert!(confirmed_state(&room).await.is_err());
            assert!(matches!(room.encryption_state(), EncryptionState::NotEncrypted));
        });
    }
}
