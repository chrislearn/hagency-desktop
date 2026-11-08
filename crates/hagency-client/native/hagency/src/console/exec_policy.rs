//! #27 — release a dirty workspace, and the per-agent execution policy.
//!
//! Both routes port a retained operator surface:
//!
//! * `POST clear-dirty` — backend-v2.js:8897-8902 (`requireRouterBearer`).
//!   TS replies `{ok:true,result:cleared}` and maps a refusal through
//!   `routerRefusalStatus` (backend-v2.js:1923): `not_found` 404,
//!   `inspection_required` 409.
//! * `GET`/`PUT execution-policy` — backend-v2.js:10865-10885
//!   (`requireBearer`). GET is `{executionPolicy,grants,appliesTo}` and 404s
//!   when the agent does not exist; PUT normalizes (`400` on refusal), 404s
//!   the same way, and answers `{ok:true,executionPolicy,appliesTo}`.
//!
//! Both are operator mutations, so both take the console's existing
//! `Scope::AgentLifecycle` — the same scope the agent lifecycle and the
//! stopped-dispatch inspection routes already require.
use super::resources::failure;
use super::{Error, Session, body, console, failed, recheck, usage::query};
use crate::{refusal, resources::domain};
use salvo::prelude::*;
use serde_json::{Value, json};

pub(super) fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("agents/{id}/execution-policy")
                .get(policy)
                .put(edit_policy),
        )
        .push(Router::with_path("resources/{id}/clear-dirty").post(clear_dirty))
}

/// A valid session whose grant is `Scope::AgentLifecycle`. A read-only or
/// other-scoped session is refused before any store work, exactly as the
/// agent lifecycle routes do.
fn check_lifecycle(depot: &Depot, res: &mut Response) -> bool {
    let session = match depot.get_typed::<Session>() {
        Ok(session) => session,
        Err(_) => {
            failed(res, Error::Unauthorized);
            return false;
        }
    };
    match console(depot).and_then(|c| c.0.authority.can_lifecycle(session)) {
        Ok(true) => true,
        Ok(false) => {
            failed(res, Error::LifecycleForbidden);
            false
        }
        Err(error) => {
            failed(res, error);
            false
        }
    }
}

fn agent_id(req: &Request) -> Result<String, Error> {
    let id = req.param::<String>("id").ok_or(Error::Invalid)?;
    hagency_core::project::identifier(&id, 128).map_err(|_| Error::Invalid)?;
    Ok(id)
}

/// `GET /api/agents/:name/execution-policy`: 404 when the agent does not
/// exist, else the retained three-key object verbatim.
#[handler]
async fn policy(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let id = match agent_id(req) {
        Ok(id) => id,
        Err(error) => {
            failed(res, error);
            return;
        }
    };
    if !check_lifecycle(depot, res) {
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.execution_policy(id).await;
    if result.is_ok() && recheck(depot).is_err() {
        failure(res, hagency_store::Error::Unavailable);
        return;
    }
    match result {
        Ok(value) => res.render(Json(value)),
        // TS answers a bare `{error:'agent not found'}` at 404 here, not the
        // router refusal shape; the code word is what the console keys on.
        Err(error) => failure(res, error),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PolicyEdit {
    execution_policy: Option<Value>,
}

/// `PUT /api/agents/:name/execution-policy`: the store normalizes with the
/// engagement's framework (`400` on refusal), persists, and answers
/// `{ok:true,executionPolicy,appliesTo}`.
#[handler]
async fn edit_policy(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let id = match agent_id(req) {
        Ok(id) => id,
        Err(error) => {
            failed(res, error);
            return;
        }
    };
    if !check_lifecycle(depot, res) {
        return;
    }
    if req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        != Some("application/json")
    {
        failed(res, Error::Invalid);
        return;
    }
    let input: PolicyEdit = match body(req, 1024).await {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(input) => input,
            Err(_) => {
                failed(res, Error::Invalid);
                return;
            }
        },
        Err(error) => {
            failed(res, error);
            return;
        }
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or_default();
    let result = store
        .set_execution_policy(id, input.execution_policy, now)
        .await;
    if result.is_ok() && recheck(depot).is_err() {
        failure(res, hagency_store::Error::Unavailable);
        return;
    }
    match result {
        Ok(yolo) => res.render(Json(json!({
            "ok": true,
            "executionPolicy": { "yolo": yolo },
            "appliesTo": "next_dispatch",
        }))),
        Err(error) => failure(res, error),
    }
}

/// `POST /api/router/resources/:id/clear-dirty`: release the workspace, or
/// refuse `inspection_required` (409) when it is quarantined.
#[handler]
async fn clear_dirty(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let id = match agent_id(req) {
        Ok(id) => id,
        Err(error) => {
            failed(res, error);
            return;
        }
    };
    if !check_lifecycle(depot, res) {
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or_default();
    let result = store.clear_workspace_dirty(id, now).await;
    if result.is_ok() && recheck(depot).is_err() {
        failure(res, hagency_store::Error::Unavailable);
        return;
    }
    match result {
        // TS replies `res.json({ok:true, result: cleared})` where `cleared` is
        // the store's own `{ok:true}` (backend-v2.js:8899-8901) — the exact
        // shape, no extra key.
        Ok(_) => res.render(Json(json!({"ok": true, "result": {"ok": true}}))),
        // TS maps the quarantine refusal to `inspection_required` at 409
        // (backend-v2.js:1929); native's store names the same fact
        // `Quarantined`, so that is the one code this route maps itself.
        Err(hagency_store::Error::Quarantined) => {
            refusal(res, StatusCode::CONFLICT, "inspection_required")
        }
        Err(error) => failure(res, error),
    }
}
