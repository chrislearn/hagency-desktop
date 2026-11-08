//! Matrix onboarding diagnostics (task #51): the three TS routes that tell an
//! operator whether a homeserver is reachable, and whether it can reach us.
//!
//! TS source of truth: `backend-v2.js:9643` (callback-check), `:9666` (probe),
//! `:9698` (reach); the underlying logic in `lib/matrix-candidates.js`
//! (`originFor`:40, `discoverBaseUrl`:73, `probeHomeserver`:115,
//! `describeMatrixReach`:310, `verifyCallbackFromHomeserver`:470). Read-only:
//! no credential is read, no project side is mutated, nothing is cached across
//! requests (TS probes on every visit — a remembered "reachable" misleads).
use super::{Error, body, failed, recheck, usage::query};
use crate::resources::domain;
use salvo::prelude::*;
use serde::Deserialize;
use std::time::Duration;

/// TS `PROBE_TIMEOUT_MS` (lib/matrix-candidates.js:30).
const PROBE_TIMEOUT_MS: u64 = 4000;
/// A response-body bound for the versions/well-known reads.
const PROBE_BODY_MAX: usize = 64 * 1024;

pub(super) fn router() -> Router {
    Router::with_path("matrix")
        .push(Router::with_path("probe").post(probe))
        .push(Router::with_path("reach").get(reach))
        .push(Router::with_path("callback-check").post(callback_check))
}

// --- Pure helpers (the TS `lib/matrix-candidates.js` surface) --------------

/// TS `originFor` (:40): `scheme://host[:port]` — the host KEEPS its explicit
/// port (the mock tests probe an ephemeral port, and TS `new URL().host`
/// includes it). No path, no query, no invented default.
fn origin_for(value: &str) -> Option<String> {
    let raw = value.trim();
    if raw.is_empty() {
        return None;
    }
    if !raw.starts_with("http://") && !raw.starts_with("https://") {
        return None;
    }
    let url = reqwest::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let origin = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    match url.scheme() {
        "http" => Some(format!("http://{origin}")),
        "https" => Some(format!("https://{origin}")),
        _ => None,
    }
}

/// TS `discoverBaseUrl` (:73): the name is enough; well-known is tried; a
/// port-bearing name skips discovery; every outcome carries its `via`.
async fn discover_base_url(server_name: &str, client: &ProbeClient) -> Discovery {
    let name = server_name.trim().trim_end_matches('/');
    if name.is_empty() {
        return Discovery {
            url: None,
            via: "no server name given".into(),
        };
    }
    if name.starts_with("http://") || name.starts_with("https://") {
        return Discovery {
            url: origin_for(name),
            via: "you gave a URL, not a name".into(),
        };
    }
    if name.contains(':') {
        return Discovery {
            url: Some(format!("https://{name}")),
            via: format!("{name} already carries a port, so well-known does not apply"),
        };
    }
    let fallback = format!("https://{name}");
    let well_known = format!("{fallback}/.well-known/matrix/client");
    match client.get_json(&well_known, None).await {
        // TS: res.ok AND the body has a usable base_url -> "declares it";
        // res.ok but no usable base_url (or non-JSON body) -> "serves a
        // well-known with no usable base_url; fell back".
        Ok(Some(body)) => {
            let declared = body
                .get("m.homeserver")
                .and_then(|v| v.get("base_url"))
                .and_then(|v| v.as_str())
                .and_then(origin_for);
            match declared {
                Some(url) => Discovery {
                    url: Some(url),
                    via: format!("{name} declares it in /.well-known/matrix/client"),
                },
                None => Discovery {
                    url: Some(fallback),
                    via: format!("{name} serves a well-known with no usable base_url; fell back"),
                },
            }
        }
        Ok(None) => Discovery {
            url: Some(fallback),
            via: format!("{name} serves a well-known with no usable base_url; fell back"),
        },
        Err(ProbeError::Http(status)) => Discovery {
            url: Some(fallback),
            via: format!("{name} has no well-known (HTTP {status}); fell back to the name itself"),
        },
        // TS catch arm: a failed lookup is a fallback, not an error.
        Err(_) => Discovery {
            url: Some(fallback),
            via: format!("could not read {name}'s well-known; fell back to the name itself"),
        },
    }
}

struct Discovery {
    url: Option<String>,
    via: String,
}

/// TS `probeHomeserver` (:115): `/_matrix/client/versions`, 200 with a
/// `versions` array is the ONLY thing counted reachable.
async fn probe_homeserver(origin: &str, client: &ProbeClient) -> serde_json::Value {
    if origin.trim().is_empty() {
        return serde_json::json!({
            "reachable": false,
            "reason": "no probeable URL: a bare server name is not an address",
        });
    }
    let versions_url = format!("{origin}/_matrix/client/versions");
    match client.get_json(&versions_url, None).await {
        Ok(Some(body)) => match body.get("versions").and_then(|v| v.as_array()) {
            Some(versions) => {
                let last_three: Vec<&serde_json::Value> =
                    versions.iter().rev().take(3).rev().collect();
                serde_json::json!({
                    "reachable": true,
                    "status": 200,
                    "versions": last_three,
                })
            }
            None => serde_json::json!({
                "reachable": false,
                "status": 200,
                "reason": "answered 200 with no `versions` array — not a Matrix homeserver",
            }),
        },
        Ok(None) => serde_json::json!({
            "reachable": false,
            "status": 200,
            "reason": "answered 200 but not with JSON — probably a proxy, not a homeserver",
        }),
        Err(ProbeError::Http(status)) => serde_json::json!({
            "reachable": false,
            "status": status,
            "reason": format!("homeserver answered HTTP {status}"),
        }),
        Err(ProbeError::Timeout) => serde_json::json!({
            "reachable": false,
            "reason": format!("no answer within {PROBE_TIMEOUT_MS}ms"),
        }),
        Err(ProbeError::Connect) => serde_json::json!({
            "reachable": false,
            "reason": "could not connect to the homeserver",
        }),
    }
}

// --- Minimal HTTP probe client (reqwest, no credential) ---------------------

/// Read-only GET returning parsed JSON, or a classified failure. Deliberately
/// separate from `hagency-matrix`'s authenticated client: these routes make
/// unauthenticated spec calls (`/versions`, `.well-known`) to arbitrary
/// operator-supplied origins, so a credentialed client is the wrong tool.
struct ProbeClient {
    client: reqwest::Client,
    timeout: Duration,
}

#[derive(Debug)]
enum ProbeError {
    Http(u16),
    Timeout,
    Connect,
}

impl ProbeClient {
    fn new() -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("static reqwest builder");
        Self {
            client,
            timeout: Duration::from_millis(PROBE_TIMEOUT_MS),
        }
    }

    async fn get_json(
        &self,
        url: &str,
        authorization: Option<&str>,
    ) -> Result<Option<serde_json::Value>, ProbeError> {
        let mut request = self.client.get(url).timeout(self.timeout);
        if let Some(token) = authorization {
            request = request.bearer_auth(token);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Err(ProbeError::Timeout),
            Err(_) => return Err(ProbeError::Connect),
        };
        let status = response.status().as_u16();
        if status != 200 {
            return Err(ProbeError::Http(status));
        }
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(_) => return Err(ProbeError::Connect),
        };
        if body.len() > PROBE_BODY_MAX {
            // A 200 answer larger than a homeserver's versions/well-known
            // document is not one; treat it as "not usable JSON" (TS's
            // res.json() catch arm), not a reachable homeserver.
            return Ok(None);
        }
        match serde_json::from_slice(&body) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        }
    }
}

// --- Handlers ---------------------------------------------------------------

/// TS `POST /api/matrix/probe` (backend-v2.js:9666).
#[handler]
async fn probe(req: &mut Request, _depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let raw = match body(req, 4 * 1024).await {
        Ok(raw) => raw,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let input: ProbeBody = match serde_json::from_slice(&raw) {
        Ok(input) => input,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let server_name = input.server_name.as_deref().unwrap_or("").trim();
    let url = input.url.as_deref().unwrap_or("").trim();
    if server_name.is_empty() && url.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(serde_json::json!({
            "error": "server_name or url is required",
        })));
        return;
    }
    let client = ProbeClient::new();
    // TS: an explicit url wins; the name alone is enough otherwise.
    let mut origin = origin_for(url);
    let mut via = if origin.is_some() {
        Some("you gave the address explicitly".to_string())
    } else {
        None
    };
    if origin.is_none() {
        let discovered =
            discover_base_url(if url.is_empty() { server_name } else { url }, &client).await;
        via = Some(discovered.via.clone());
        match discovered.url {
            Some(url) => origin = Some(url),
            None => {
                res.status_code(StatusCode::BAD_REQUEST);
                res.render(Json(serde_json::json!({
                    "error": "could not turn that into an address",
                    "code": "not_an_origin",
                    "via": discovered.via,
                })));
                return;
            }
        }
    }
    let probe_result = probe_homeserver(origin.as_deref().unwrap_or_default(), &client).await;
    res.render(Json(serde_json::json!({
        "origin": origin,
        "via": via,
        "probe": probe_result,
    })));
}

/// TS `GET /api/matrix/reach` (backend-v2.js:9698).
#[handler]
async fn reach(_req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let sides = match store.project_sides().await {
        Ok(sides) => sides,
        Err(_) => {
            failed(res, Error::Unavailable);
            return;
        }
    };
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    let client = ProbeClient::new();
    let mut homeservers = Vec::with_capacity(sides.len());
    for side in sides {
        // `ProjectSide.id` IS the server name (ADR-016). TS probes
        // `originFor(side.apiBaseUrl)`; native's ProjectSide carries no
        // apiBaseUrl, so the address is null and the probe is TS's
        // "no probeable URL" arm — honest, and no outbound DNS to a stored
        // name (which would also violate the sandbox's localhost-only rule).
        let has_credential = match store.side_credential_for_transport(side.id.clone()).await {
            Ok(credential) => credential.is_some(),
            Err(_) => false,
        };
        let probe_result = probe_homeserver("", &client).await;
        homeservers.push(serde_json::json!({
            "serverName": side.id,
            "url": null,
            "source": "already recorded as a project side",
            "alreadyASide": true,
            "hasCredential": has_credential,
            "sideId": side.id,
            "probe": probe_result,
        }));
    }
    // Native configures no appservice port / edge / sync intake, so report
    // TS's honest no-config arm verbatim rather than guess an inbound path.
    res.render(Json(serde_json::json!({
        "homeservers": homeservers,
        "appservice": {
            "listening": false,
            "port": null,
            "inboundVia": null,
            "reason": "HAGENCY_APPSERVICE_PORT is not set, no co-located edge is configured and no sync intake is configured, so nothing will receive your homeserver's events. Set the port, run bin/hagency-appservice-edge beside the homeserver and set HAGENCY_EDGE_URL, or set HAGENCY_APPSERVICE_SYNC_SIDE and HAGENCY_APPSERVICE_SYNC_URL.",
            "callbackCandidates": [],
        },
    })));
}

/// TS `POST /api/matrix/callback-check` (backend-v2.js:9643).
///
/// TS asks, from inside the homeserver's own container, which of our addresses
/// it can reach. Native configures no appservice port / co-located edge, so the
/// honest TS answer is its "nothing to reach" arm — `verifyCallbackFromHomeserver`
/// returns `{applicable:false, reason:'neither an appservice port nor a
/// co-located edge is configured, so there is nothing for your homeserver to
/// reach'}` before it even looks at the homeserver address.
#[handler]
async fn callback_check(req: &mut Request, _depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    // The body is accepted (TS accepts an optional homeserver_url) but native
    // never reaches the docker arm: there is no port/edge to verify against.
    let _raw = match body(req, 4 * 1024).await {
        Ok(raw) => raw,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let _: Option<CallbackCheckBody> = serde_json::from_slice(&_raw).ok();
    res.render(Json(serde_json::json!({
        "applicable": false,
        "reason": "neither an appservice port nor a co-located edge is configured, so there is nothing for your homeserver to reach",
    })));
}

#[derive(Deserialize)]
struct ProbeBody {
    server_name: Option<String>,
    url: Option<String>,
}

#[derive(Deserialize)]
struct CallbackCheckBody {
    #[allow(dead_code)]
    homeserver_url: Option<String>,
}
