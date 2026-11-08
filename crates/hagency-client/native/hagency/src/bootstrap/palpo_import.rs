//! Import the fleet configuration a Palpo HAgency owner downloads (TS parity:
//! `mockup/lib/fleet-credential-import.js` `parseFleetCredentialImport` and
//! `lib/fleet-outbound-config.js` `normalizeOutboundTransport`).
//!
//! The download is `{fleetId, serverName, credentialVersion: 1, registration,
//! transport}`: the App Service registration Palpo installed for this fleet and
//! the outbound machine credential. The import writes three things and nothing
//! else:
//! - the six-field fleet `registrations` row through the store's sole writer,
//!   with the reception left UNBOUND — Palpo's connection probe binds it later
//!   (`lib/fleet-protocol.js` sets `receptionRoomId` only there);
//! - `palpo-transport.json` + `palpo.machine_token`, the inputs of
//!   `serve --palpo-transport`;
//! - `palpo-appservice.json`, the App Service tokens the fleet's identities
//!   act with.
//!
//! Every rule below is the TS rule; a refusal names the field, never a value.
use hagency_core::authority::Registration;
use hagency_store::{DomainRepository, Repository, private};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the configuration is not the owner download from Palpo ({0})")]
    Invalid(&'static str),
    #[error("the state directory refused the import: {0}")]
    Store(#[from] hagency_store::Error),
}

/// Local part of the fleet's private approval bot inside its exclusive
/// namespace `^@<fleetId>_[a-z0-9_]+:<server>$`.
const APPROVAL_LOCALPART: &str = "approval";

pub struct Imported {
    pub fleet_id: String,
    pub server_name: String,
    pub representative: String,
    pub approval_bot: String,
    pub endpoint: String,
    /// The bound reception room, empty until Palpo's connection probe.
    pub reception: String,
}

fn text<'a>(value: &'a Value, key: &str, field: &'static str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error::Invalid(field))
}

fn token(value: &str, field: &'static str) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > 4096 {
        return Err(Error::Invalid(field));
    }
    Ok(())
}

pub fn parse(raw: &str) -> Result<(Registration, Value, Value, String, u64), Error> {
    if raw.len() > 65536 {
        return Err(Error::Invalid("size"));
    }
    let envelope: Value = serde_json::from_str(raw).map_err(|_| Error::Invalid("json"))?;
    if !envelope.is_object() {
        return Err(Error::Invalid("json"));
    }
    if envelope.get("credentialVersion").is_some_and(|v| v != 1) {
        return Err(Error::Invalid("credentialVersion"));
    }
    let registration = envelope
        .get("registration")
        .ok_or(Error::Invalid("registration"))?;
    let fleet_id = text(registration, "id", "registration.id")?;
    let suffix = fleet_id
        .strip_prefix("hf_")
        .ok_or(Error::Invalid("registration.id"))?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid("registration.id"));
    }
    if envelope.get("fleetId").is_some_and(|v| v != fleet_id) {
        return Err(Error::Invalid("fleetId"));
    }
    let server_name = text(&envelope, "serverName", "serverName")?;
    let sender = text(
        registration,
        "sender_localpart",
        "registration.sender_localpart",
    )?;
    if sender != format!("{fleet_id}_representative") {
        return Err(Error::Invalid("registration.sender_localpart"));
    }
    let escaped: String = server_name
        .chars()
        .flat_map(|c| {
            let special = ".*+?^${}()|[]\\".contains(c);
            special
                .then_some('\\')
                .into_iter()
                .chain(std::iter::once(c))
        })
        .collect();
    let namespace = format!("^@{fleet_id}_[a-z0-9_]+:{escaped}$");
    let namespaces = registration
        .get("namespaces")
        .ok_or(Error::Invalid("registration.namespaces"))?;
    let users = namespaces.get("users").and_then(Value::as_array);
    let empty = |key| {
        namespaces
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    };
    if !users.is_some_and(|u| {
        u.len() == 1
            && u[0].get("exclusive") == Some(&json!(true))
            && u[0].get("regex") == Some(&json!(namespace))
    }) || !empty("rooms")
        || !empty("aliases")
    {
        return Err(Error::Invalid("registration.namespaces"));
    }
    let as_token = text(registration, "as_token", "registration.as_token")?;
    let hs_token = text(registration, "hs_token", "registration.hs_token")?;
    token(as_token, "registration.as_token")?;
    token(hs_token, "registration.hs_token")?;
    if as_token == hs_token {
        return Err(Error::Invalid("registration tokens"));
    }
    let url = text(registration, "url", "registration.url")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| Error::Invalid("registration.url"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::Invalid("registration.url"));
    }
    // The outbound machine credential (normalizeOutboundTransport).
    let transport = envelope.get("transport").ok_or(Error::Invalid(
        "transport: only an outbound fleet can be imported",
    ))?;
    let keys_ok = transport.as_object().is_some_and(|o| {
        o.keys()
            .all(|k| ["mode", "url", "token", "generation"].contains(&k.as_str()))
    });
    let machine = text(transport, "token", "transport.token")?;
    let generation = transport
        .get("generation")
        .and_then(Value::as_u64)
        .ok_or(Error::Invalid("transport.generation"))?;
    if !keys_ok
        || transport.get("mode") != Some(&json!("outbound"))
        || machine.len() < 16
        || machine.len() > 4096
        || machine.chars().any(char::is_whitespace)
        || machine == as_token
        || machine == hs_token
        || !(1..=hagency_core::JSON_SAFE_MAX).contains(&generation)
    {
        return Err(Error::Invalid("transport"));
    }
    let endpoint = reqwest::Url::parse(text(transport, "url", "transport.url")?)
        .map_err(|_| Error::Invalid("transport.url"))?;
    let local = matches!(
        endpoint.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    );
    if !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.path().trim_end_matches('/') != format!("/api/fleet/v2/{fleet_id}")
        || !(endpoint.scheme() == "https" || endpoint.scheme() == "http" && local)
    {
        return Err(Error::Invalid("transport.url"));
    }
    let endpoint = endpoint.as_str().trim_end_matches('/').to_owned();
    let row = Registration {
        fleet_id: fleet_id.to_owned(),
        generation: 1,
        server_name: server_name.to_owned(),
        reception_room_id: String::new(),
        representative_mxid: format!("@{sender}:{server_name}"),
        approval_bot_mxid: format!("@{fleet_id}_{APPROVAL_LOCALPART}:{server_name}"),
    };
    row.validate().map_err(|_| Error::Invalid("serverName"))?;
    let appservice = json!({
        "as_token": as_token, "hs_token": hs_token, "sender_localpart": sender,
        "namespace": namespace, "url": url,
    });
    Ok((row, appservice, json!(machine), endpoint, generation))
}

/// The Matrix client API the fleet's App Service identities act through.
pub(crate) fn homeserver(homeserver: &str) -> Result<String, Error> {
    let origin = reqwest::Url::parse(homeserver).map_err(|_| Error::Invalid("homeserver"))?;
    if origin.scheme() != "https" && !matches!(origin.host_str(), Some("127.0.0.1" | "localhost")) {
        return Err(Error::Invalid("homeserver must be https"));
    }
    Ok(origin.as_str().trim_end_matches('/').to_owned())
}

/// The three private files `serve --palpo-transport` reads; one writer for the
/// CLI and the console import.
pub(crate) fn write(
    state: &Path,
    registration: &Registration,
    appservice: &Value,
    machine: &Value,
    endpoint: &str,
    generation: u64,
) -> Result<(), Error> {
    let transport = json!({
        "profile": "palpo_v2_resources_v1", "endpoint": endpoint,
        "registration": registration, "machine_generation": generation,
    });
    let encode =
        |value: &Value| serde_json::to_vec_pretty(value).map_err(|_| Error::Invalid("encode"));
    private::replace(&state.join("palpo-transport.json"), &encode(&transport)?)?;
    private::replace(
        &state.join("palpo.machine_token"),
        machine.as_str().unwrap_or_default().as_bytes(),
    )?;
    private::replace(&state.join("palpo-appservice.json"), &encode(appservice)?)?;
    Ok(())
}

/// Import into an initialized private state (service stopped or not yet run).
pub fn run(
    state: &Path,
    file: &Path,
    homeserver: &str,
    reception: Option<&str>,
) -> Result<Imported, Error> {
    let origin =
        self::homeserver(homeserver).map_err(|_| Error::Invalid("--homeserver must be https"))?;
    private::read_secret(&state.join("operator.token"))?;
    let raw = std::fs::read_to_string(file).map_err(|_| Error::Invalid("file unreadable"))?;
    let (registration, mut appservice, machine, endpoint, generation) = parse(&raw)?;
    appservice["homeserver"] = json!(origin);
    let _custody = Repository::open(state)?;
    let mut domain = DomainRepository::open(state)?;
    // A re-import of the same fleet keeps a reception an earlier probe bound.
    let current = domain
        .provisioning_registration(&registration.fleet_id)
        .ok();
    let mut registration = registration;
    if let Some(current) = current {
        registration.reception_room_id = current.reception_room_id;
    } else if let Some(room) = reception {
        registration.reception_room_id = room.to_owned();
    }
    domain.register(&registration)?;
    write(
        state,
        &registration,
        &appservice,
        &machine,
        &endpoint,
        generation,
    )?;
    Ok(Imported {
        fleet_id: registration.fleet_id.clone(),
        server_name: registration.server_name.clone(),
        representative: registration.representative_mxid.clone(),
        approval_bot: registration.approval_bot_mxid.clone(),
        endpoint,
        reception: registration.reception_room_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const FLEET: &str = "hf_0123456789abcdef0123456789abcdef";
    fn download() -> Value {
        json!({"fleetId": FLEET, "serverName": "example.test", "credentialVersion": 1,
            "registration": {"id": FLEET, "url": "http://relay:8090/api/relay/v2/x", "as_token": "as-token-value",
                "hs_token": "hs-token-value", "sender_localpart": format!("{FLEET}_representative"),
                "namespaces": {"users": [{"exclusive": true, "regex": format!("^@{FLEET}_[a-z0-9_]+:example\\.test$")}],
                    "aliases": [], "rooms": []}, "rate_limited": true, "receive_ephemeral": false},
            "transport": {"mode": "outbound", "url": format!("https://palpo.example/api/fleet/v2/{FLEET}"),
                "token": "machine-token-0123456789", "generation": 1}})
    }
    #[test]
    fn native_palpo_import_accepts_the_owner_download_unbound() {
        let (row, appservice, _, endpoint, generation) = parse(&download().to_string()).unwrap();
        assert_eq!(
            row.reception_room_id, "",
            "reception stays unbound until the probe"
        );
        assert_eq!(
            row.representative_mxid,
            format!("@{FLEET}_representative:example.test")
        );
        assert_eq!(
            row.approval_bot_mxid,
            format!("@{FLEET}_approval:example.test")
        );
        assert_eq!(
            endpoint,
            format!("https://palpo.example/api/fleet/v2/{FLEET}")
        );
        assert_eq!(generation, 1);
        assert_eq!(
            appservice["sender_localpart"],
            format!("{FLEET}_representative")
        );
    }
    #[test]
    fn native_palpo_import_refuses_what_ts_refuses() {
        #[allow(clippy::type_complexity)]
        let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
            (
                "callback fleet",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("transport");
                }),
            ),
            (
                "broad namespace",
                Box::new(|v| v["registration"]["namespaces"]["users"][0]["regex"] = json!("^@.*$")),
            ),
            (
                "non-exclusive",
                Box::new(|v| {
                    v["registration"]["namespaces"]["users"][0]["exclusive"] = json!(false)
                }),
            ),
            (
                "other fleet endpoint",
                Box::new(|v| {
                    v["transport"]["url"] = json!(
                        "https://palpo.example/api/fleet/v2/hf_ffffffffffffffffffffffffffffffff"
                    )
                }),
            ),
            (
                "plain http remote",
                Box::new(|v| {
                    v["transport"]["url"] =
                        json!(format!("http://palpo.example/api/fleet/v2/{FLEET}"))
                }),
            ),
            (
                "machine token reuses as_token",
                Box::new(|v| v["transport"]["token"] = json!("as-token-value")),
            ),
            (
                "wrong sender",
                Box::new(|v| v["registration"]["sender_localpart"] = json!("someone")),
            ),
            ("version 2", Box::new(|v| v["credentialVersion"] = json!(2))),
        ];
        for (name, change) in cases {
            let mut value = download();
            change(&mut value);
            assert!(parse(&value.to_string()).is_err(), "{name}");
        }
    }
}
