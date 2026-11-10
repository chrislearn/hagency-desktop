//! Operator account/fleet mutations on the running service's own store owner
//! (task #28). These are the trusted-local writers the offline CLI verbs
//! (`bootstrap::accounts`, `bootstrap::registration`) performed by opening the
//! state directory a second time — which forced the service to be stopped.
//! Now the running service performs them through its `DomainStore`, so the
//! operator commands work while the service runs.
//!
//! The `authorize` hoop (loopback + operator bearer token) already gates every
//! route mounted here; these handlers add only the store's own contract and
//! render the store's own fields, never the operator token.

use crate::{App, refusal};
use hagency_core::authority::Registration;
use hagency_core::project::identifier;
use hagency_store::{AccountReadinessMode, DomainStore, Error, LoginAttempt, LoginVerdict};
use salvo::prelude::*;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) fn router() -> Router {
    Router::new()
        .push(Router::with_path("project-sides").post(register))
        .push(Router::with_path("accounts").get(list).post(prepare))
        .push(Router::with_path("accounts/{id}/retire").post(retire))
        .push(Router::with_path("accounts/{id}/login-begin").post(login_begin))
        .push(Router::with_path("accounts/{id}/login-settle").post(login_settle))
}

fn domain(depot: &Depot, res: &mut Response) -> Option<DomainStore> {
    match depot.get_typed::<App>().ok().and_then(|a| a.domain.clone()) {
        Some(store) => Some(store),
        None => {
            refusal(res, StatusCode::SERVICE_UNAVAILABLE, "domain_unavailable");
            None
        }
    }
}

/// The route's own refusal classes, one word each — never a guessed figure.
fn store_error(res: &mut Response, error: Error) {
    let (status, code) = match error {
        Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_domain_command"),
        Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        Error::State => (StatusCode::CONFLICT, "state_conflict"),
        Error::Conflict => (StatusCode::CONFLICT, "revision_conflict"),
        Error::Generation => (StatusCode::CONFLICT, "stale_generation"),
        Error::Capacity => (StatusCode::SERVICE_UNAVAILABLE, "capacity"),
        Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        Error::OutcomeUnknown => (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "native_unavailable"),
    };
    refusal(res, status, code);
}

fn account_id(req: &Request) -> Result<String, ()> {
    let id = req.param::<String>("id").ok_or(())?;
    identifier(&id, 128).map_err(|_| ())?;
    Ok(id)
}

async fn read_body(req: &mut Request, res: &mut Response) -> Option<Vec<u8>> {
    match tokio::time::timeout(Duration::from_secs(2), req.payload_with_max_size(64 * 1024)).await {
        Ok(Ok(bytes)) => Some(bytes.to_vec()),
        Ok(Err(_)) => {
            refusal(res, StatusCode::PAYLOAD_TOO_LARGE, "body_rejected");
            None
        }
        Err(_) => {
            refusal(res, StatusCode::REQUEST_TIMEOUT, "body_timeout");
            None
        }
    }
}

fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
}

/// The operator's fleet registration, mirroring the offline
/// `registration register --file` writer and the console `POST
/// /console/api/project-sides` route: the store owns every guarantee.
#[handler]
async fn register(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Some(raw) = read_body(req, res).await else {
        return;
    };
    let registration: Registration = match serde_json::from_slice(&raw) {
        Ok(registration) => registration,
        Err(_) => {
            refusal(res, StatusCode::BAD_REQUEST, "invalid_domain_command");
            return;
        }
    };
    match store.register(registration.clone()).await {
        Ok(()) => res.render(Json(serde_json::json!({
            "ok": true,
            "side": {
                "id": registration.fleet_id,
                "generation": registration.generation,
            },
        }))),
        Err(error) => store_error(res, error),
    }
}

/// List every account choice — the read the offline `account inspect` served.
#[handler]
async fn list(_req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    match store.account_choices().await {
        Ok(choices) => res.render(Json(serde_json::to_value(choices).unwrap_or_default())),
        Err(error) => store_error(res, error),
    }
}

/// Reserve and materialize one fresh namespace — the offline `account prepare`.
#[handler]
async fn prepare(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PrepareInput {
        profile: String,
    }
    let Some(raw) = read_body(req, res).await else {
        return;
    };
    let input: PrepareInput = match serde_json::from_slice(&raw) {
        Ok(input) => input,
        Err(_) => {
            refusal(res, StatusCode::BAD_REQUEST, "invalid_domain_command");
            return;
        }
    };
    let profile = input.profile;
    let reserved = match store.reserve_account(profile).await {
        Ok(choice) => choice,
        Err(error) => {
            store_error(res, error);
            return;
        }
    };
    match store.materialize_account(reserved.id).await {
        Ok(choice) => res.render(Json(serde_json::to_value(choice).unwrap_or_default())),
        Err(error) => store_error(res, error),
    }
}

/// Permanently fence a binding — the offline `account retire`.
#[handler]
async fn retire(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Ok(id) = account_id(req) else {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_domain_command");
        return;
    };
    match store.retire_account(id).await {
        Ok(choice) => res.render(Json(serde_json::to_value(choice).unwrap_or_default())),
        Err(error) => store_error(res, error),
    }
}

/// Allocate a login attempt and hand back the launch environment the CLI must
/// spawn the provider child under — the service owns the binding, so only it
/// can resolve the retained HOME/CODEX_HOME. The attempt carries no secret.
#[handler]
async fn login_begin(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Ok(id) = account_id(req) else {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_domain_command");
        return;
    };
    let Some(now) = now_ms() else {
        refusal(res, StatusCode::SERVICE_UNAVAILABLE, "clock_unavailable");
        return;
    };
    let attempt = match store.begin_account_login(id.clone(), now).await {
        Ok(attempt) => attempt,
        Err(error) => {
            store_error(res, error);
            return;
        }
    };
    // The login-shaped launch gate + the trusted HOME/CODEX_HOME resolution.
    let mut environment: BTreeMap<std::ffi::OsString, std::ffi::OsString> = BTreeMap::new();
    let env = match store.managed_account(id).await {
        Ok(account) => match account.prepare_login() {
            Ok(launch) => launch.apply_codex_environment(&mut environment),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    if let Err(error) = env {
        store_error(res, error);
        return;
    }
    let home = environment
        .get(&std::ffi::OsString::from("HOME"))
        .and_then(|v| v.to_str())
        .map(str::to_owned);
    let codex_home = environment
        .get(&std::ffi::OsString::from("CODEX_HOME"))
        .and_then(|v| v.to_str())
        .map(str::to_owned);
    res.render(Json(serde_json::json!({
        "ok": true,
        "attempt": attempt,
        "home": home,
        "codexHome": codex_home,
    })));
}

/// Settle the attempt with the parent's classification of the child's exit.
/// The path id must match the attempt's own account id, so a mismatched
/// attempt never settles a different account's row.
#[handler]
async fn login_settle(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Ok(id) = account_id(req) else {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_domain_command");
        return;
    };
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Settle {
        attempt: LoginAttempt,
        verdict: LoginVerdict,
    }
    let Some(raw) = read_body(req, res).await else {
        return;
    };
    let Settle { attempt, verdict } = match serde_json::from_slice(&raw) {
        Ok(settle) => settle,
        Err(_) => {
            refusal(res, StatusCode::BAD_REQUEST, "invalid_domain_command");
            return;
        }
    };
    if attempt.account_id() != id {
        refusal(res, StatusCode::BAD_REQUEST, "attempt_account_mismatch");
        return;
    }
    let Some(now) = now_ms() else {
        refusal(res, StatusCode::SERVICE_UNAVAILABLE, "clock_unavailable");
        return;
    };
    match store.settle_account_login(attempt, verdict, now).await {
        Ok(readiness) => {
            let mode = match readiness.mode {
                AccountReadinessMode::Subscription => "subscription",
                AccountReadinessMode::ApiKey => "api_key",
                AccountReadinessMode::Unknown => "unknown",
            };
            res.render(Json(serde_json::json!({ "ok": true, "mode": mode })));
        }
        Err(error) => store_error(res, error),
    }
}
