//! Fresh authenticated key-query validation shared by outgoing and approval intake.
use crate::Error;
use matrix_sdk_crypto::{Device, OlmMachine, UserIdentity};
use ruma::{
    OwnedUserId,
    api::{IncomingResponseExt, client::keys::get_keys},
};
use serde_json::Value;
use std::collections::BTreeSet;

/// What a fresh `/keys/query` response was accepted as (ADR-183 B). The
/// recipients are the devices the recipient's cross-signing identity — the
/// identity the SDK accepted, whose master key `check_anchors` pins — has
/// signed. Every other device the response listed is excluded and counted,
/// never a refusal: an unverified device is not an outage. A recipient user
/// with no signed device at all refuses (`Recipients`, fail-closed).
pub(super) struct Accepted {
    pub recipients: BTreeSet<(String, String)>,
    pub unverified: BTreeSet<(String, String)>,
    /// The `unverified` devices whose keys ARE consistent (self-signed and
    /// served for that user), only not signed by the owner's identity. An
    /// agent's own messages reach these too (ADR-185); approval cards do not.
    pub unsigned: BTreeSet<(String, String)>,
}

pub(super) async fn accept(
    machine: &OlmMachine,
    users: &[OwnedUserId],
    query_id: &str,
    response: &Value,
) -> Result<BTreeSet<(String, String)>, Error> {
    Ok(accept_counted(machine, users, query_id, response, true)
        .await?
        .recipients)
}

/// ADR-185: an agent's recipients are every consistent device of its users,
/// signed by the owner's identity or not. The identity checks of
/// `accept_counted` (anchor, master and self-signing keys, own device) apply
/// unchanged.
pub(super) async fn accept_all_devices(
    machine: &OlmMachine,
    users: &[OwnedUserId],
    query_id: &str,
    response: &Value,
) -> Result<BTreeSet<(String, String)>, Error> {
    let accepted = accept_counted(machine, users, query_id, response, false).await?;
    Ok(accepted
        .recipients
        .union(&accepted.unsigned)
        .cloned()
        .collect())
}

/// `signed_only`: each recipient user must have at least one device signed by
/// its identity (ADR-183 B, approval cards). An agent (`false`, ADR-185) needs
/// at least one consistent device, signed or not.
pub(super) async fn accept_counted(
    machine: &OlmMachine,
    users: &[OwnedUserId],
    query_id: &str,
    response: &Value,
    signed_only: bool,
) -> Result<Accepted, Error> {
    validate_keys_shape(users, response)?;
    let query = get_keys::v3::Response::try_from_http_response(http::Response::new(
        response.to_string().as_bytes(),
    ))
    .map_err(|_| Error::Wire)?;
    let own = machine.identity_keys();
    let own_keys =
        &response["device_keys"][machine.user_id().as_str()][machine.device_id().as_str()]["keys"];
    if own_keys
        .get(format!("curve25519:{}", machine.device_id()))
        .and_then(Value::as_str)
        != Some(own.curve25519.to_base64().as_str())
        || own_keys
            .get(format!("ed25519:{}", machine.device_id()))
            .and_then(Value::as_str)
            != Some(own.ed25519.to_base64().as_str())
    {
        return Err(Error::Identity);
    }
    machine
        .mark_request_as_sent(query_id.into(), &query)
        .await
        .map_err(|_| Error::OutcomeUnknown)?;
    let mut recipients = BTreeSet::new();
    let mut unverified = BTreeSet::new();
    let mut unsigned = BTreeSet::new();
    let mut own_seen = false;
    for user in users {
        let identity = machine
            .get_identity(user, None)
            .await
            .map_err(|_| Error::OutcomeUnknown)?
            .ok_or(Error::Recipients)?;
        if !identity.is_verified() {
            return Err(Error::Recipients);
        }
        // A malformed fresh entry may be ignored by the SDK while a cached
        // verified identity survives. Require the supplied authority fields
        // to be exactly the identity the SDK accepted, not just present.
        let (master, signing) = match &identity {
            UserIdentity::Own(identity) => (
                serde_json::to_value(identity.master_key().as_ref()),
                serde_json::to_value(identity.self_signing_key().as_ref()),
            ),
            UserIdentity::Other(identity) => (
                serde_json::to_value(identity.master_key().as_ref()),
                serde_json::to_value(identity.self_signing_key().as_ref()),
            ),
        };
        let master = master.map_err(|_| Error::Storage)?;
        let signing = signing.map_err(|_| Error::Storage)?;
        for (fresh, accepted) in [
            (&response["master_keys"][user.as_str()], &master),
            (&response["self_signing_keys"][user.as_str()], &signing),
        ] {
            if !same_fields(fresh, accepted, &["user_id", "usage", "keys", "signatures"]) {
                return Err(Error::Recipients);
            }
        }
        let devices = machine
            .get_user_devices(user, None)
            .await
            .map_err(|_| Error::OutcomeUnknown)?;
        let fresh = response["device_keys"][user.as_str()]
            .as_object()
            .ok_or(Error::Wire)?;
        let mut seen = BTreeSet::new();
        let mut verified = 0usize;
        for device in devices.devices() {
            let id = device.device_id().as_str();
            seen.insert(id.to_string());
            let consistent = match fresh.get(id) {
                Some(raw) => consistent_device(raw, &device, user.as_str(), id)?,
                None => false,
            };
            let signed = device.is_verified() && device.is_cross_signed_by_owner();
            if user == machine.user_id() && device.device_id() == machine.device_id() {
                // The SDK's own device is identity, not a recipient: anything
                // other than the exact accepted keys is an identity refusal.
                let own = machine.identity_keys();
                if !consistent
                    || !signed
                    || device.curve25519_key() != Some(own.curve25519)
                    || device.ed25519_key() != Some(own.ed25519)
                {
                    return Err(Error::Identity);
                }
                own_seen = true;
            } else if consistent && signed {
                recipients.insert((user.to_string(), id.to_string()));
                verified += 1;
            } else {
                if consistent {
                    unsigned.insert((user.to_string(), id.to_string()));
                    if !signed_only {
                        verified += 1;
                    }
                }
                unverified.insert((user.to_string(), id.to_string()));
            }
        }
        // A fresh entry the SDK did not keep (malformed, badly signed) is a
        // device the identity has not signed either: excluded and counted.
        for id in fresh.keys().filter(|id| !seen.contains(*id)) {
            unverified.insert((user.to_string(), id.clone()));
        }
        if verified == 0 && user != machine.user_id() {
            return Err(Error::Recipients);
        }
    }
    if !own_seen
        || response["device_keys"][machine.user_id().as_str()]
            .get(machine.device_id().as_str())
            .is_none()
    {
        return Err(Error::Identity);
    }
    Ok(Accepted {
        recipients,
        unverified,
        unsigned,
    })
}
fn consistent_device(raw: &Value, device: &Device, user: &str, id: &str) -> Result<bool, Error> {
    let accepted = serde_json::to_value(device.as_device_keys()).map_err(|_| Error::Storage)?;
    Ok(same_fields(
        raw,
        &accepted,
        &["user_id", "device_id", "algorithms", "keys", "signatures"],
    ) && raw["user_id"].as_str() == Some(user)
        && raw["device_id"].as_str() == Some(id))
}
fn validate_keys_shape(users: &[OwnedUserId], value: &Value) -> Result<(), Error> {
    if value
        .get("failures")
        .is_some_and(|v| v.as_object().is_none_or(|o| !o.is_empty()))
    {
        return Err(Error::Recipients);
    }
    let keys = value
        .get("device_keys")
        .and_then(Value::as_object)
        .ok_or(Error::Wire)?;
    if keys.keys().cloned().collect::<BTreeSet<_>>()
        != users
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>()
    {
        return Err(Error::Recipients);
    }
    let mut count = 0;
    for (user, devices) in keys {
        count += devices.as_object().ok_or(Error::Wire)?.len();
        if value.get("master_keys").and_then(|v| v.get(user)).is_none()
            || value
                .get("self_signing_keys")
                .and_then(|v| v.get(user))
                .is_none()
        {
            return Err(Error::Recipients);
        }
    }
    if count > 64 {
        return Err(Error::Capacity);
    }
    Ok(())
}

pub(super) fn same_fields(fresh: &Value, accepted: &Value, fields: &[&str]) -> bool {
    fresh.is_object()
        && accepted.is_object()
        && fields
            .iter()
            .all(|key| fresh.get(*key).is_some() && fresh.get(*key) == accepted.get(*key))
}

/// The trust anchor of two `/keys/query` responses is the same (ADR-183 B):
/// for every named user the master and self-signing keys are equal. Device
/// keys and user-signing keys are allowed to differ — the device list may
/// change without a restart.
pub(crate) fn same_anchor<'a>(
    recorded: &Value,
    fresh: &Value,
    users: impl IntoIterator<Item = &'a str>,
) -> bool {
    users.into_iter().all(|user| {
        ["master_keys", "self_signing_keys"].iter().all(|field| {
            same_fields(
                &fresh[*field][user],
                &recorded[*field][user],
                &["user_id", "usage", "keys"],
            )
        })
    })
}

/// Only the explicit fresh-account enrollment calls this pre-verification
/// validator. It does not replace `accept` at any encryption boundary.
pub(super) async fn anchored_initial(
    machine: &OlmMachine,
    users: &[OwnedUserId],
    query_id: &str,
    response: &Value,
    anchors: &std::collections::BTreeMap<String, String>,
) -> Result<(), Error> {
    let own = machine.user_id().as_str();
    if response["device_keys"][own]
        .as_object()
        .is_none_or(|v| !v.is_empty())
        || ["master_keys", "self_signing_keys", "user_signing_keys"]
            .iter()
            .any(|field| response.get(*field).and_then(|m| m.get(own)).is_some())
    {
        return Err(Error::Identity);
    }
    let peers = users
        .iter()
        .filter(|u| u.as_str() != own)
        .cloned()
        .collect::<Vec<_>>();
    let mut peer_response = response.clone();
    peer_response["device_keys"]
        .as_object_mut()
        .ok_or(Error::Wire)?
        .remove(own);
    validate_keys_shape(&peers, &peer_response)?;
    for field in ["master_keys", "self_signing_keys", "user_signing_keys"] {
        if response.get(field).is_some_and(|m| {
            m.as_object().is_none_or(|m| {
                m.keys()
                    .any(|u| !users.iter().any(|expected| expected.as_str() == u))
            })
        }) {
            return Err(Error::Recipients);
        }
    }
    let query = get_keys::v3::Response::try_from_http_response(http::Response::new(
        response.to_string().as_bytes(),
    ))
    .map_err(|_| Error::Wire)?;
    machine
        .mark_request_as_sent(query_id.into(), &query)
        .await
        .map_err(|_| Error::OutcomeUnknown)?;
    for user in &peers {
        let Some(UserIdentity::Other(identity)) = machine
            .get_identity(user, None)
            .await
            .map_err(|_| Error::OutcomeUnknown)?
        else {
            return Err(Error::Recipients);
        };
        let master =
            serde_json::to_value(identity.master_key().as_ref()).map_err(|_| Error::Storage)?;
        let signing = serde_json::to_value(identity.self_signing_key().as_ref())
            .map_err(|_| Error::Storage)?;
        let anchor = anchors.get(user.as_str()).ok_or(Error::Recipients)?;
        if master["keys"] != serde_json::json!({format!("ed25519:{anchor}"):anchor}) {
            return Err(Error::Recipients);
        }
        for (field, accepted) in [("master_keys", &master), ("self_signing_keys", &signing)] {
            if !same_fields(
                &response[field][user.as_str()],
                accepted,
                &["user_id", "usage", "keys", "signatures"],
            ) {
                return Err(Error::Recipients);
            }
        }
        let devices = machine
            .get_user_devices(user, None)
            .await
            .map_err(|_| Error::OutcomeUnknown)?;
        let fresh = response["device_keys"][user.as_str()]
            .as_object()
            .ok_or(Error::Recipients)?;
        let mut signed = 0usize;
        for device in devices.devices() {
            let id = device.device_id().as_str();
            // Identity material that names a device id is malformed, not an
            // unsigned device: still a refusal.
            if [&master, &signing].iter().any(|key| {
                key["keys"]
                    .as_object()
                    .is_none_or(|keys| keys.values().any(|v| v.as_str() == Some(id)))
            }) {
                return Err(Error::Recipients);
            }
            let Some(raw) = fresh.get(id) else {
                continue;
            };
            let algorithms = raw["algorithms"].as_array();
            let usable = algorithms.is_some_and(|algorithms| {
                algorithms.len() == 2
                    && algorithms.contains(&serde_json::json!("m.olm.v1.curve25519-aes-sha2"))
                    && algorithms.contains(&serde_json::json!("m.megolm.v1.aes-sha2"))
            });
            // ADR-183 B: a device the anchor has not signed is excluded, never
            // a refusal; the enrollment needs at least one signed device.
            if device.is_cross_signed_by_owner()
                && usable
                && consistent_device(raw, &device, user.as_str(), id)?
            {
                signed += 1;
            }
        }
        if signed == 0 {
            return Err(Error::Recipients);
        }
    }
    Ok(())
}
