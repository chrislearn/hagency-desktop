//! This Agent's earlier devices on this computer. Each execution device
//! generation is a new Matrix device whose crypto store sits beside the others
//! in the Agent's private directory, under the same store key. A new device
//! cannot decrypt what an earlier one sent, so on its own it could never tell
//! whether an earlier device's report send landed (ADR 0013). Those messages
//! were encrypted with sessions the earlier device started itself, and their
//! keys stay in its store: the report check reads them there.
//!
//! Nothing is imported into the current device's store, and only a session the
//! earlier device started itself counts, never one it received from another
//! device or imported. Megolm decryption checks the session's signature, and
//! that the event belongs to the session's room.
use matrix_sdk::ruma::{DeviceId, OwnedUserId, RoomId, UserId};
use matrix_sdk::{SqliteCryptoStore, SqliteStoreConfig};
use matrix_sdk_crypto::store::CryptoStore;
use matrix_sdk_crypto::types::events::room::encrypted::EncryptedEvent;
use matrix_sdk_crypto::vodozemac::Curve25519PublicKey;
use serde_json::Value;
use std::path::Path;

/// The SDK's crypto database inside a device's store directory. A directory
/// without one is never opened, so a check never creates a store.
const CRYPTO_DATABASE: &str = "matrix-sdk-crypto.sqlite3";
/// At most this many earlier stores are opened for one check, newest first.
const MAX_EARLIER_DEVICES: usize = 32;

struct Earlier {
    user: OwnedUserId,
    curve25519: Curve25519PublicKey,
    store: SqliteCryptoStore,
}

/// The stores of this Agent's earlier devices, opened for one history check.
pub(super) struct EarlierDevices(Vec<Earlier>);

/// Agent device IDs are `HAGE2EE` and a hash (the server's device grant).
fn device_directory(name: &str) -> bool {
    name.len() > 7
        && name.len() <= 64
        && name.starts_with("HAGE2EE")
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl EarlierDevices {
    pub(super) fn none() -> Self {
        Self(Vec::new())
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Opens the stores under `root` of `user`'s devices other than `current`.
    /// A store whose account is another user or another device is skipped.
    pub(super) async fn open(root: &Path, passphrase: &str, user: &UserId, current: &DeviceId) -> Self {
        let Ok(entries) = std::fs::read_dir(root) else { return Self::none() };
        let mut names: Vec<(std::time::SystemTime, String)> = entries
            .flatten()
            .filter_map(|entry| {
                // Directory entry metadata does not follow links: a link is skipped.
                let metadata = entry.metadata().ok().filter(|metadata| metadata.is_dir())?;
                let name = entry.file_name().into_string().ok()?;
                (device_directory(&name) && name != current.as_str())
                    .then(|| (metadata.modified().unwrap_or(std::time::UNIX_EPOCH), name))
            })
            .collect();
        // The device that ran last is the likeliest sender.
        names.sort_by(|a, b| b.cmp(a));
        let mut found = Vec::new();
        for (_, name) in names.into_iter().take(MAX_EARLIER_DEVICES) {
            let path = root.join(&name);
            if !path.join(CRYPTO_DATABASE).is_file() {
                continue;
            }
            // No `PRAGMA optimize`: the check writes nothing to an earlier store.
            let config = SqliteStoreConfig::new(&path).passphrase(Some(passphrase)).optimize(false);
            let Ok(store) = SqliteCryptoStore::open_with_config(&config).await else { continue };
            let Ok(Some(account)) = store.load_account().await else { continue };
            if account.user_id() != user || account.device_id().as_str() != name {
                continue;
            }
            found.push(Earlier {
                user: account.user_id().to_owned(),
                curve25519: account.identity_keys().curve25519,
                store,
            });
        }
        Self(found)
    }

    /// The decrypted event, when one of these devices sent `raw` in `room`,
    /// encrypted with a session it started itself.
    pub(super) async fn read(&self, room: &RoomId, raw: &Value) -> Option<Value> {
        if self.0.is_empty()
            || raw["type"] != "m.room.encrypted"
            || raw["content"]["algorithm"] != "m.megolm.v1.aes-sha2"
        {
            return None;
        }
        let session_id = raw["content"]["session_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 256)?;
        let event: EncryptedEvent = serde_json::from_value(raw.clone()).ok()?;
        for earlier in &self.0 {
            if event.sender != earlier.user {
                continue;
            }
            let Ok(Some(session)) = earlier.store.get_inbound_group_session(room, session_id).await
            else {
                continue;
            };
            // A received session carries the sending device's key, and a
            // forwarded, restored or exported one is marked imported.
            if session.has_been_imported() || session.sender_key() != earlier.curve25519 {
                continue;
            }
            // Also copies the event's sender, ID, timestamp and `unsigned`.
            let Ok((plain, _)) = session.decrypt(&event).await else { continue };
            return Some(Value::Object(plain));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk_crypto::{EncryptionSettings, OlmMachine, OlmMachineBuilder};
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
    use matrix_sdk::ruma::{OwnedDeviceId, OwnedRoomId};
    use serde_json::json;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn user(id: &str) -> OwnedUserId {
        OwnedUserId::try_from(id).unwrap()
    }
    fn device(id: &str) -> OwnedDeviceId {
        id.into()
    }
    fn room(id: &str) -> OwnedRoomId {
        OwnedRoomId::try_from(id).unwrap()
    }

    async fn machine(root: &Path, user: &UserId, device: &DeviceId) -> OlmMachine {
        let store = SqliteCryptoStore::open(root.join(device.as_str()), Some(KEY)).await.unwrap();
        OlmMachineBuilder::new(user, device)
            .with_crypto_store(store)
            .build()
            .await
            .unwrap()
    }

    /// An event as the homeserver serves it: `machine`'s encrypted message.
    async fn encrypted(machine: &OlmMachine, room: &RoomId, text: &str, event: &str) -> Value {
        // Shared with nobody else: the session is this device's own.
        machine
            .share_room_key(room, std::iter::empty::<&UserId>(), EncryptionSettings::default())
            .await
            .unwrap();
        let result = machine
            .encrypt_room_event(room, RoomMessageEventContent::text_plain(text))
            .await
            .unwrap();
        json!({
            "type": "m.room.encrypted",
            "event_id": event,
            "sender": machine.user_id().as_str(),
            "origin_server_ts": 1_700_000_000_000u64,
            "content": serde_json::from_str::<Value>(result.content.json().get()).unwrap(),
        })
    }

    #[tokio::test]
    async fn an_earlier_device_store_reads_only_messages_that_device_encrypted() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let agent = user("@agent:example.test");
        let room_id = room("!room:example.test");
        let earlier_id = device("HAGE2EE00000000000000001");
        let current = device("HAGE2EE00000000000000002");
        let earlier = machine(root, &agent, &earlier_id).await;
        let raw = encrypted(&earlier, &room_id, "EARLIER_REPORT", "$earlier:example.test").await;
        drop(earlier);
        // A directory with no database is never turned into a store.
        let empty = root.join("HAGE2EE00000000000000003");
        std::fs::create_dir(&empty).unwrap();

        let devices = EarlierDevices::open(root, KEY, &agent, &current).await;
        assert_eq!(devices.len(), 1);
        let plain = devices.read(&room_id, &raw).await.expect("the earlier device's own message");
        assert_eq!(plain["type"], "m.room.message");
        assert_eq!(plain["content"]["body"], "EARLIER_REPORT");
        assert_eq!(plain["event_id"], "$earlier:example.test");
        assert_eq!(plain["sender"], agent.as_str());
        assert!(!empty.join(CRYPTO_DATABASE).exists());
        // Its session belongs to its own room, and the sender must be its account.
        assert!(devices.read(&room("!other:example.test"), &raw).await.is_none());
        let mut forged = raw.clone();
        forged["sender"] = "@mallory:example.test".into();
        assert!(devices.read(&room_id, &forged).await.is_none());
        // One changed ciphertext character fails the session's MAC and signature.
        let mut altered = raw.clone();
        let mut ciphertext: Vec<char> = altered["content"]["ciphertext"].as_str().unwrap().chars().collect();
        let middle = ciphertext.len() / 2;
        ciphertext[middle] = if ciphertext[middle] == 'A' { 'B' } else { 'A' };
        altered["content"]["ciphertext"] = ciphertext.into_iter().collect::<String>().into();
        assert!(devices.read(&room_id, &altered).await.is_none());

        // The current device's own store, another account's and a wrong key are not used.
        assert_eq!(EarlierDevices::open(root, KEY, &agent, &earlier_id).await.len(), 0);
        assert_eq!(EarlierDevices::open(root, KEY, &user("@other:example.test"), &current).await.len(), 0);
        assert_eq!(EarlierDevices::open(root, &"f".repeat(64), &agent, &current).await.len(), 0);
    }

    #[tokio::test]
    async fn a_session_another_device_only_imported_does_not_count() {
        let source = tempfile::tempdir().unwrap();
        let agent = user("@agent:example.test");
        let room_id = room("!room:example.test");
        let original = machine(source.path(), &agent, &device("HAGE2EE0000000000000000A")).await;
        let raw = encrypted(&original, &room_id, "ORIGINAL_REPORT", "$original:example.test").await;
        let session_id = raw["content"]["session_id"].as_str().unwrap().to_owned();
        let exported = original
            .store()
            .get_inbound_group_session(&room_id, &session_id)
            .await
            .unwrap()
            .unwrap()
            .export()
            .await;
        drop(original);

        // Beside the current device is only a device that imported that key.
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let importer = machine(root, &agent, &device("HAGE2EE0000000000000000B")).await;
        importer
            .store()
            .import_exported_room_keys(vec![exported], |_, _| {})
            .await
            .unwrap();
        drop(importer);
        let devices = EarlierDevices::open(root, KEY, &agent, &device("HAGE2EE0000000000000000C")).await;
        assert_eq!(devices.len(), 1);
        assert!(devices.read(&room_id, &raw).await.is_none());
    }
}
