//! The agent and message leftover routes (board #49, migration 067). Six TS
//! routes, ported 1:1 where the port can source them and documented where it
//! cannot:
//!
//!   * `GET /agents/{name}/launch-env` (`backend-v2.js:12344`) — done.
//!   * `POST /agents/{name}/undelete` (`:12308`) — the tombstone the retained
//!     force-delete wrote (`deleted_agents.json`) now has a table
//!     (`agent_tombstones`), so the route has the one thing it acts on.
//!   * `POST /agents/{name}/avatar` (`:16370`) — the retained route handed the
//!     request to the bridge over SSE; a durable `avatar_requests` row is that
//!     hand-off made restart-safe.
//!   * `GET /agents/{name}/delivery-events` (`:16988`) — the retained
//!     `message-delivery-events.jsonl` log is `delivery_events` rows.
//!   * `GET /messages/{id}` (`:16900`) and `POST /messages/{id}/suppress`
//!     (`:17002`) — the operator board `messages.json` is `operator_messages`.
//!
//! Every TS route in this group is behind an AGENT token (`requireAgentToken`)
//! or the bearer token. The native console has one session gate for `/console/api`
//! and no agent-token tier, so these mount behind `authenticate` exactly as the
//! roster and detail reads do — the same substitution #49's launch-env records.
//!
//! Substrate the port does NOT have, named rather than faked:
//!   * `isGroupMember` (`:4429`) — no project-group membership table, so a group
//!     message targets an agent only via an explicit mention or the recorded
//!     default/room recipient.
//!   * the unread index (`getUnreadInboxMessages`, `:4251`) — an in-memory index
//!     in the retained process, not a stored fact; the native board keeps no
//!     reader state, so "unread for this target" is "not suppressed for it".
use super::{Error, body, failed, recheck, usage::query};
use crate::resources::domain;
use hagency_core::project::AgentName;
use salvo::prelude::*;
use serde::Deserialize;
use serde_json::json;

pub(super) fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("agents/{name}")
                .push(Router::with_path("launch-env").get(launch_env))
                .push(Router::with_path("undelete").post(undelete))
                .push(Router::with_path("avatar").post(avatar))
                .push(Router::with_path("delivery-events").get(delivery_events)),
        )
        .push(
            Router::with_path("messages/{id}")
                .get(message_detail)
                .push(Router::with_path("suppress").post(suppress)),
        )
}

/// The agent-name path parameter: the same `AgentName` shape `agent_detail`
/// validates, never an identifier that could address a session or filesystem.
fn agent_name(req: &Request) -> Result<String, Error> {
    let name = req.param::<String>("name").ok_or(Error::Invalid)?;
    AgentName::try_from(name.clone()).map_err(|_| Error::Invalid)?;
    Ok(name)
}

/// The message-id path parameter: the retained board's own id (`msg_0001`,
/// `backend-v2.js:3249`) — bounded text, never a path component.
fn message_id(req: &Request) -> Result<String, Error> {
    let id = req.param::<String>("id").ok_or(Error::Invalid)?;
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) || id.contains('/') {
        return Err(Error::Invalid);
    }
    Ok(id)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

/// The TS name shape `/^[\w\-]+$/` (`backend-v2.js:16372`).
fn agent_not_found(res: &mut Response) {
    res.status_code(StatusCode::NOT_FOUND);
    res.render(Json(
        json!({"error": "agent not found", "code": "agent_not_found"}),
    ));
}

#[handler]
async fn launch_env(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Ok(name) = agent_name(req) else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.agent_launch_env(&name).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(Some(profile)) => res.render(Json(json!({ "runtimeProfile": profile }))),
        Ok(None) => agent_not_found(res),
        Err(error) => super::resources::failure(res, error),
    }
}

/// `POST /api/agents/:name/undelete` (`backend-v2.js:12308-12316`): 400 invalid
/// name, 404 `{error:'no tombstone found'}` when none exists, else the tombstone
/// is removed and `{ok:true, undeleted:true, name}` is answered.
#[handler]
async fn undelete(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Ok(name) = agent_name(req) else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.undelete_agent(name.clone()).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(true) => res.render(Json(json!({"ok": true, "undeleted": true, "name": name}))),
        Ok(false) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"error": "no tombstone found"})));
        }
        Err(error) => super::resources::failure(res, error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Avatar {
    #[serde(default)]
    generate: bool,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    mime: Option<String>,
}

/// `POST /api/agents/:name/avatar` (`backend-v2.js:16370-16377`): 400 invalid
/// name; `force` is `body.generate === true || query.force === 'true'`; `custom`
/// is `!!body.image`; the request is enqueued and `{ok:true, queued:true, name,
/// force, custom}` answered. The retained route broadcast the SSE frame the
/// bridge consumed; the durable `avatar_requests` row is that hand-off.
#[handler]
async fn avatar(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &["force"], 16).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Ok(name) = agent_name(req) else {
        failed(res, Error::Invalid);
        return;
    };
    let input: Avatar =
        match serde_json::from_slice(&body(req, 10 * 1024 * 1024).await.unwrap_or_default()) {
            Ok(input) => input,
            Err(_) => Avatar {
                generate: false,
                image: None,
                mime: None,
            },
        };
    let force = input.generate || req.query::<String>("force").as_deref() == Some("true");
    let custom = input.image.is_some();
    let mime = input.mime.unwrap_or_else(|| "image/png".into());
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store
        .record_avatar_request(name.clone(), force, custom, Some(mime), now_ms())
        .await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(()) => res.render(Json(json!({
            "ok": true, "queued": true, "name": name, "force": force, "custom": custom,
        }))),
        Err(error) => super::resources::failure(res, error),
    }
}

/// `GET /api/agents/:name/delivery-events` (`backend-v2.js:16988-17000`): 400
/// invalid name, 404 unknown agent, else `{agent, events}` — the agent's
/// delivery events, newest first, bounded by `limit` (default 100).
#[handler]
async fn delivery_events(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &["limit"], 32).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Ok(name) = agent_name(req) else {
        failed(res, Error::Invalid);
        return;
    };
    let limit = req
        .query::<String>("limit")
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(100);
    let Some(store) = domain(depot, res) else {
        return;
    };
    let known = store.agent_detail(&name).await;
    match known {
        Ok(Some(_)) => {}
        Ok(None) => {
            agent_not_found(res);
            return;
        }
        Err(error) => {
            super::resources::failure(res, error);
            return;
        }
    }
    let result = store.delivery_events(name.clone(), limit).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(events) => res.render(Json(json!({"agent": name, "events": events}))),
        Err(error) => super::resources::failure(res, error),
    }
}

/// `GET /api/messages/:id` (`backend-v2.js:16900-16914`): 404
/// `{error:'message not found'}`, else the record with `priority` normalized,
/// `schema` reduced and `time` the retained `relativeTime`.
#[handler]
async fn message_detail(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Ok(id) = message_id(req) else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.operator_message(id, now_ms()).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(Some(record)) => res.render(Json(record)),
        Ok(None) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"error": "message not found"})));
        }
        Err(error) => super::resources::failure(res, error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Suppress {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// `POST /api/messages/:id/suppress` (`backend-v2.js:17002-17040`): 400 when no
/// agent is named, 404 unknown agent, 404 `message not found`, 400 when the
/// message is not deliverable to the agent, else the agent is added to
/// `suppressedRecipients` idempotently and
/// `{ok:true,id,agent,suppressed:true,was_unread,is_unread_now,suppressedRecipients}`
/// is answered.
#[handler]
async fn suppress(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Ok(id) = message_id(req) else {
        failed(res, Error::Invalid);
        return;
    };
    let input: Suppress = match serde_json::from_slice(&body(req, 1024).await.unwrap_or_default()) {
        Ok(input) => input,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let Some(agent) = input.agent else {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(json!({"error": "agent required"})));
        return;
    };
    // The TS route normalizes then looks the agent up; a name the service never
    // engaged is its 404.
    let Ok(agent) = AgentName::try_from(agent).map(String::from) else {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(json!({"error": "agent required"})));
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    match store.agent_detail(&agent).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            agent_not_found(res);
            return;
        }
        Err(error) => {
            super::resources::failure(res, error);
            return;
        }
    }
    let reason = input
        .reason
        .map(|value| value.chars().take(128).collect::<String>())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "explicit-suppress".into());
    let result = store
        .suppress_message(id.clone(), agent.clone(), reason, now_ms())
        .await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(hagency_store::SuppressOutcome::Recorded(suppression)) => res.render(Json(json!({
            "ok": true,
            "id": id,
            "agent": agent,
            "suppressed": true,
            "was_unread": suppression.was_unread,
            "is_unread_now": suppression.is_unread_now,
            "suppressedRecipients": suppression.suppressed_recipients,
        }))),
        Ok(hagency_store::SuppressOutcome::Unknown) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"error": "message not found"})));
        }
        Ok(hagency_store::SuppressOutcome::NotDeliverable) => {
            res.status_code(StatusCode::BAD_REQUEST);
            res.render(Json(json!({
                "error": format!("message {id} is not deliverable to {agent}"),
            })));
        }
        Err(error) => super::resources::failure(res, error),
    }
}
