//! Task #13: the project-side appservice registration ISSUE route — the
//! retained `POST /api/project-sides/:id/registration-file`
//! (`backend-v2.js:10031-10168`) on the native console API, plus the CLI
//! runner that shares it. One store method (`DomainRepository::
//! issue_side_registration`) serves both; this file adds no second
//! generation path, no guard of its own beyond the console's.
//!
//! WHAT THE CALLER GETS, matching the TS response key for key: the YAML
//! file's path, the two eight-hex fingerprints, and the facts needed to
//! finish the install. What it NEVER gets is `as_token` or `hs_token` —
//! the response is `IssueSideRegistration` serialized, and that type has
//! no token field (the ADR-132 projection's negative guarantee, kept
//! whole here). The tokens travel only into the private state directory:
//! the YAML for the operator's install, `<state>/matrix.appservice_token`
//! for the service the appservice profile already reads
//! (`bootstrap/config.rs:646`) — no hand-placing left between issuing
//! and serving.
//!
//! STAGING, like TS: a side that already holds a credential keeps it live
//! and this issue is held as `pending` until a verify proves the
//! homeserver accepts the new one — `staged: true` and the verbatim
//! `stagedNote` say so. Native has no access verdicts to compute TS's
//! `liveIsBroken`, so every existing credential stages; promotion is the
//! verify task's, owed separately (see the report).
use super::engagements::check_lifecycle;
use super::{Error, body, failed, recheck, usage::query};
use crate::{refusal, resources::domain};
use hagency_store::{DomainRepository, IssueSideRegistrationRequest, Repository, private};
use salvo::prelude::*;
use serde::Deserialize;
use std::path::Path;

pub(super) fn router() -> Router {
    Router::with_path("project-sides/{side}/registration-file").post(issue)
}

/// The TS route body (`backend-v2.js:10035-10085`): `url` required;
/// `registration_id`, `sender_localpart`, `user_namespace` and
/// `exclusive` optional with the store's defaults. Closed key set, like
/// every console body.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct IssueBody {
    url: String,
    registration_id: Option<String>,
    sender_localpart: Option<String>,
    user_namespace: Option<String>,
    exclusive: Option<bool>,
}

/// The TS failure shapes, as the console's refusal convention carries
/// them: statuses match (404 side, 400 body), the code words are the
/// native console's.
fn store_failure(res: &mut Response, error: hagency_store::Error) {
    let (status, code) = match error {
        hagency_store::Error::NotFound => (StatusCode::NOT_FOUND, "project_side_not_found"),
        hagency_store::Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_registration_body"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "registration_unavailable"),
    };
    refusal(res, status, code);
}

#[handler]
async fn issue(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // The side id from the path, validated like every path segment the
    // console accepts: bounded, no control bytes.
    let Some(side) = req.param::<String>("side") else {
        failed(res, Error::Invalid);
        return;
    };
    if side.is_empty() || side.len() > 256 || !side.bytes().all(|b| b.is_ascii_graphic()) {
        failed(res, Error::Invalid);
        return;
    }
    // The `-file` form takes no query: TS's `?replace=true` belongs to
    // the token-returning endpoint this port deliberately omits.
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    // A credential issue is a project-side write: the same finite
    // lifecycle scope `POST /console/api/project-sides` requires, refused
    // before any body parse or store read.
    if !check_lifecycle(depot, res) {
        return;
    }
    let raw = match body(req, 4 * 1024).await {
        Ok(raw) => raw,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let input: IssueBody = match serde_json::from_slice(&raw) {
        Ok(input) => input,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    // TS: an empty url is the 400 `bad_request`, never a guess — the url
    // is the address the homeserver pushes to and cannot be derived.
    if input.url.trim().is_empty() {
        failed(res, Error::Invalid);
        return;
    }
    let request = IssueSideRegistrationRequest {
        side,
        url: input.url,
        registration_id: input.registration_id,
        sender_localpart: input.sender_localpart,
        user_namespace: input.user_namespace,
        exclusive: input.exclusive,
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.issue_side_registration(request).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(issued) => {
            // `ok: true` plus the camelCase TS body — the struct's own
            // serialization, so no key can drift from the store's.
            let mut value = serde_json::to_value(&issued).expect("plain JSON fields");
            value["ok"] = serde_json::Value::Bool(true);
            res.render(Json(value));
        }
        Err(error) => store_failure(res, error),
    }
}

/// The CLI arm of the same operation: `hagency side-registration --side
/// <server> --url <url>` — offline, like `registration register`, against
/// the same store method the route serves. Prints the route's body (no
/// tokens; the YAML and the token file are already on disk). Requires an
/// initialized private state, never manufactures one.
pub fn run_cli(
    state: &Path,
    request: IssueSideRegistrationRequest,
) -> Result<(), hagency_store::Error> {
    private::read_secret(&state.join("operator.token"))?;
    let _custody = Repository::open(state)?;
    let mut domain = DomainRepository::open(state)?;
    let issued = domain.issue_side_registration(&request, now_ms()?)?;
    let mut value = serde_json::to_value(&issued).expect("plain JSON fields");
    value["ok"] = serde_json::Value::Bool(true);
    println!("{value}");
    Ok(())
}

fn now_ms() -> Result<u64, hagency_store::Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or(hagency_store::Error::Unavailable)
}
