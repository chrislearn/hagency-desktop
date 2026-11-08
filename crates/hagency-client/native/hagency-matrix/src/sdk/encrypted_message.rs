//! Crypto leaf called only while the same SDK worker exclusively owns its machine.
//! Callers retain their own purpose, authority, journal and admission boundaries.
use crate::{Error, outgoing::state::Write};
use matrix_sdk_crypto::{CollectStrategy, EncryptionSettings, OlmMachine};
use ruma::{OwnedUserId, RoomId};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) struct Input<'a> {
    pub users: &'a [OwnedUserId],
    pub room: &'a RoomId,
    pub transaction_id: &'a str,
    pub content: &'a Value,
    pub query_id: &'a str,
    pub response: &'a Value,
    /// ADR-185: an agent's own message goes to every consistent device of
    /// its owner; an approval card (`false`) only to the anchor-signed ones.
    pub all_devices: bool,
}
/// The prepared writes and the recipient set they were shared with: the
/// owner's devices the accepted identity has signed (ADR-183 B-1). The
/// caller records that set on its attempt, because it — not the server's
/// device list — is what the attempt's own validator measures the sent
/// messages against.
pub(super) async fn prepare(
    machine: &OlmMachine,
    input: Input<'_>,
) -> Result<(Vec<Write>, BTreeSet<(String, String)>), Error> {
    let Input {
        users,
        room,
        transaction_id,
        content,
        query_id,
        response,
        all_devices,
    } = input;
    let recipients = if all_devices {
        super::keys::accept_all_devices(machine, users, query_id, response).await?
    } else {
        super::keys::accept(machine, users, query_id, response).await?
    };
    if machine
        .get_missing_sessions(users.iter().map(|u| u.as_ref()))
        .await
        .map_err(|_| Error::OutcomeUnknown)?
        .is_some()
    {
        return Err(Error::Unsupported);
    }
    machine
        .discard_room_key(room)
        .await
        .map_err(|_| Error::OutcomeUnknown)?;
    let settings = EncryptionSettings {
        sharing_strategy: if all_devices {
            CollectStrategy::AllDevices
        } else {
            CollectStrategy::OnlyTrustedDevices
        },
        ..EncryptionSettings::default()
    };
    let shares = machine
        .share_room_key(room, users.iter().map(|u| u.as_ref()), settings)
        .await
        .map_err(|_| Error::OutcomeUnknown)?;
    if shares.len() > 16 {
        return Err(Error::Capacity);
    }
    let mut actual = BTreeSet::new();
    let mut writes = Vec::new();
    for share in shares {
        // ADR-183 B: for every device the accepted identity has not signed,
        // the SDK's trusted-only strategy withholds the key and drafts an
        // `m.room_key.withheld` notice. An excluded device gets neither the
        // key nor the notice: the share is skipped, never a refusal.
        if share.event_type.to_string() == "m.room_key.withheld" {
            continue;
        }
        if share.event_type.to_string() != "m.room.encrypted" {
            return Err(Error::Recipients);
        }
        let messages = serde_json::to_value(&share.messages).map_err(|_| Error::Storage)?;
        for (user, devices) in messages.as_object().ok_or(Error::Storage)? {
            for device in devices.as_object().ok_or(Error::Storage)?.keys() {
                if !actual.insert((user.clone(), device.clone())) {
                    return Err(Error::Recipients);
                }
            }
        }
        writes.push(Write::new(
            share.event_type.to_string(),
            share.txn_id.to_string(),
            json!({"messages":messages}),
            false,
        )?);
    }
    if actual != recipients {
        return Err(Error::Recipients);
    }
    let content =
        ruma::serde::Raw::from_json_string(content.to_string()).map_err(|_| Error::Storage)?;
    // A fresh nonexpired session was created immediately above under this
    // owned worker. No other SDK operation can rotate it between calls.
    let encrypted = machine
        .encrypt_room_event_raw(room, "m.room.message", &content)
        .await
        .map_err(|_| Error::OutcomeUnknown)?;
    writes.push(Write::new(
        "m.room.encrypted".into(),
        transaction_id.to_owned(),
        serde_json::to_value(encrypted.content).map_err(|_| Error::Storage)?,
        true,
    )?);
    Ok((writes, recipients))
}
