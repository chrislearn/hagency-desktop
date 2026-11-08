//! Agent invitation intake (task #12): the wire half of the retained
//! `bridge-matrix.js:7894-8131` invite poll and `:9060-9128` accept —
//! plus the DM invite `lib/matrix-direct-chat.js:163-183` observes.
//!
//! This module is WIRE ONLY, like the rest of the crate: it reads
//! `rooms.invite` from a lightweight sync (`timeline limit 0`, the TS
//! poll's exact filter) and performs joins/leaves. The trust decision,
//! the pending record and the console answer live in
//! `hagency::bootstrap::invites` — the store's owner.
//!
//! THE INVITER IS READ FROM THE AGENT'S OWN MEMBER EVENT, lowercased by
//! the homeserver (`bridge-matrix.js:7986-7998`): the state_key must equal
//! the configured sender mxid EXACTLY, so a `@ac_BigLittle:…` lookup
//! against a homeserver-lowercased `@ac_biglittle:…` finds nothing and the
//! inviter reads as null — untrusted, a pending decision, never a guess
//! (the inviter IS the owner under ADR-002).
use crate::{CancellationToken, Error, http::Http};
use serde::Deserialize;
use serde_json::Value;

/// One observed invitation from a sync round: the room, the inviter (NULL
/// when the invite state names no sender — surfaced, never guessed), the
/// DM flag that decides `mode` (`direct` vs `group`), and the member
/// event's timestamp the direct-rooms admission records as `sinceTs`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedInvite {
    pub room_id: String,
    pub inviter: Option<String>,
    pub is_direct: bool,
    pub origin_server_ts: Option<i64>,
}

impl ObservedInvite {
    /// The binding mode the TS direct-rooms POST carries
    /// (`backend-v2.js:16287-16302`): `is_direct === true` is `direct`,
    /// everything else `group`.
    pub fn mode(&self) -> &'static str {
        if self.is_direct { "direct" } else { "group" }
    }
}

/// The joined room a successful join reports: Matrix may answer with a
/// different (canonical) room id than the one invited to, and the accept
/// path records what the SERVER said, not what was asked.
#[derive(Debug, Deserialize)]
struct Joined {
    room_id: String,
}

/// The lightweight sync filter the TS invite poll uses
/// (`bridge-matrix.js:7903`): no timeline, no ephemeral, no account data —
/// only the room set, because membership is all this path decides.
fn invite_filter() -> String {
    serde_json::json!({
        "room": {"timeline": {"limit": 0}, "ephemeral": {"types": []}, "account_data": {"types": []}},
        "presence": {"types": []},
        "account_data": {"types": []}
    })
    .to_string()
}

/// One lightweight sync round: `timeout=0`, the invite filter above, and
/// the cursor when the caller has one. The response is the caller's to
/// parse (`parse_invites`) — no state lives here.
pub async fn invite_sync(
    http: &Http,
    since: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Value, Error> {
    let filter = invite_filter();
    let mut query = vec![("timeout", "0"), ("filter", filter.as_str())];
    if let Some(since) = since {
        query.push(("since", since));
    }
    http.request(&["_matrix", "client", "v3", "sync"], Some(&query), cancel)
        .await?
        .success()
}

/// Parse `rooms.invite` of a sync response into the invitations addressed
/// to THIS identity. TS processes EVERY room key in `rooms.invite`
/// (`bridge-matrix.js:7990-7998`): the section itself is the homeserver's
/// word that this identity is invited, so each key yields exactly one
/// invite. Within a room, the inviter is the FIRST `m.room.member` event
/// whose `state_key` equals `own_mxid` — TS's `find` matches type and
/// state_key ONLY (membership content is not consulted: the stripped
/// state of an invited room carries the invite membership by
/// construction); no match or no readable state means `inviter: null`,
/// surfaced, never guessed. `origin_server_ts` and `content.is_direct`
/// ride along when present.
pub fn parse_invites(sync: &Value, own_mxid: &str) -> Vec<ObservedInvite> {
    let Some(invited) = sync.pointer("/rooms/invite").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (room_id, room) in invited {
        let mut observed = ObservedInvite {
            room_id: room_id.clone(),
            inviter: None,
            is_direct: false,
            origin_server_ts: None,
        };
        for event in room
            .pointer("/invite_state/events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if event.get("type").and_then(Value::as_str) != Some("m.room.member")
                || event.get("state_key").and_then(Value::as_str) != Some(own_mxid)
            {
                continue;
            }
            observed.inviter = event
                .get("sender")
                .and_then(Value::as_str)
                .map(str::to_owned);
            observed.is_direct = event.pointer("/content/is_direct") == Some(&Value::Bool(true));
            observed.origin_server_ts = event.get("origin_server_ts").and_then(Value::as_i64);
            break;
        }
        out.push(observed);
    }
    out
}

/// Accept an invitation by joining: `POST /_matrix/client/v3/join/{roomId}`
/// with an empty body (`bridge-matrix.js:9073-9080`), returning the room id
/// the SERVER reports — the canonical form, which may differ from the one
/// invited to.
pub async fn join_room(
    http: &Http,
    room_id: &str,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let joined: Joined = serde_json::from_value(
        http.post(
            &["_matrix", "client", "v3", "join", room_id],
            "{}".to_owned(),
            cancel,
        )
        .await?
        .success()?,
    )
    .map_err(|_| Error::InvalidJson)?;
    if joined.room_id.is_empty() {
        return Err(Error::InvalidJson);
    }
    Ok(joined.room_id)
}

/// Decline an invitation by leaving: best-effort by design
/// (`bridge-matrix.js:9135-9147`) — the decision is the record, so a
/// failed leave must not leave the contributor unable to say no. Errors
/// are the caller's to log, never to act on.
pub async fn leave_room(
    http: &Http,
    room_id: &str,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    http.post(
        &["_matrix", "client", "v3", "rooms", room_id, "leave"],
        "{}".to_owned(),
        cancel,
    )
    .await?
    .success()
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sync_with(invite: Value) -> Value {
        json!({"next_batch":"cursor","rooms":{"invite":invite}})
    }

    /// The TS-visible parse: the member event addressed to the agent names
    /// the inviter; `is_direct` and `origin_server_ts` ride along.
    #[test]
    fn parse_reads_the_agent_member_event() {
        let sync = sync_with(json!({
            "!dm:example.test": {"invite_state":{"events":[
                {"type":"m.room.create","state_key":"","sender":"@owner:example.test","content":{}},
                {"type":"m.room.member","state_key":"@worker:example.test","sender":"@owner:example.test",
                 "origin_server_ts":42,"content":{"membership":"invite","is_direct":true}}
            ]}}
        }));
        let invites = parse_invites(&sync, "@worker:example.test");
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].room_id, "!dm:example.test");
        assert_eq!(invites[0].inviter.as_deref(), Some("@owner:example.test"));
        assert!(invites[0].is_direct);
        assert_eq!(invites[0].origin_server_ts, Some(42));
        assert_eq!(invites[0].mode(), "direct");
    }

    /// The state_key must match the agent's own mxid EXACTLY — the TS bug
    /// this guards is the homeserver's lowercasing (`:7986-7998`): a
    /// mismatched case finds nothing and the inviter reads as null, which
    /// is a pending decision, never a guess.
    #[test]
    fn parse_ignores_a_member_event_for_another_user() {
        let sync = sync_with(json!({
            "!room:example.test": {"invite_state":{"events":[
                {"type":"m.room.member","state_key":"@someone-else:example.test","sender":"@owner:example.test",
                 "content":{"membership":"invite"}}
            ]}}
        }));
        let invites = parse_invites(&sync, "@worker:example.test");
        assert_eq!(invites.len(), 1);
        assert!(invites[0].inviter.is_none());
        assert!(!invites[0].is_direct);
        assert_eq!(invites[0].mode(), "group");
    }

    /// An invite with no readable state is still a room this identity is
    /// invited to — recorded with a NULL inviter, surfaced (`:2394-2396`).
    #[test]
    fn parse_records_an_unreadable_invite_state_as_null_inviter() {
        let sync = sync_with(json!({"!bare:example.test": {}}));
        let invites = parse_invites(&sync, "@worker:example.test");
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].room_id, "!bare:example.test");
        assert_eq!(invites[0].inviter, None);
    }

    /// The membership CONTENT is not consulted (TS `find` matches type +
    /// state_key only, `bridge-matrix.js:7990-7992`): the room's presence
    /// in `rooms.invite` is the homeserver's word that this identity is
    /// invited, and the member event names the inviter whatever its
    /// membership says.
    #[test]
    fn parse_does_not_consult_membership_content() {
        let sync = sync_with(json!({
            "!odd:example.test": {"invite_state":{"events":[
                {"type":"m.room.member","state_key":"@worker:example.test","sender":"@owner:example.test",
                 "content":{"membership":"leave"}}
            ]}}
        }));
        let invites = parse_invites(&sync, "@worker:example.test");
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].inviter.as_deref(), Some("@owner:example.test"));
    }

    /// A sync with no invite section observes nothing — not an error.
    #[test]
    fn parse_of_an_empty_sync_is_empty() {
        assert!(parse_invites(&json!({"next_batch":"c"}), "@w:e.t").is_empty());
    }
}
