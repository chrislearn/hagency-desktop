//! Operator task management (TS parity: `backend-v2.js:13194-13332`,
//! `lib/task-store.js`, `GET /api/project-board` at `:16025-16051` over
//! `lib/project-board.js`).
//
//! The retained routes are operator-authority routes: reads take
//! `requireTaskReadAccess` with no finer scope, and the mutations take the
//! operator bearer (`requireBearer`) plus the same read access. The console
//! session IS that operator authority — it is exchanged from the operator
//! bearer at `/console/access` — so the reads here take no scope, the same
//! read class as the roster, alerts and project-sides reads, and the writes
//! sit behind the CONFIGURE scope the alerts triage already uses (brief 28),
//! because every console mutation in this facade consults a scope and a task
//! write is an operator triage act. The scope fact travels back on the reads
//! as `permissions.configureResource`, so the page renders no control it
//! would be refused.
//
//! What the retained routes carry that native cannot source is NAMED, never
//! zeroed: `health` on the task reads (the retained supervisor snapshot
//! store has no native counterpart) and the board's Matrix-derived columns.
use super::{Error, Session, body, console, failed, recheck, usage::query};
use crate::{refusal, resources::domain};
use hagency_store::{OperatorTask, TaskFilters, operator_transitions};
use salvo::prelude::*;
use serde_json::{Value, json};

/// Every retained task field native has no source for in this slice.
/// `health` is the supervisor snapshot read-through (`backend-v2.js:13225-13232`,
/// `:13237-13249`): native has no supervisor snapshot store, so the field is
/// absent rather than invented, and the page renders it as unknown.
const UNAVAILABLE: [&str; 1] = ["health"];

pub(super) fn router() -> Router {
    Router::with_path("tasks")
        .get(list)
        // Express serves HEAD from a GET route (`app.get` registers both); the
        // retained server therefore answers a HEAD on every task read, and the
        // console's own `<Link>` prefetch sends one. Salvo's `MethodFilter(GET)`
        // does not fold HEAD onto GET, so the read routes register it explicitly
        // (board #92 — a HEAD fell past the GET child and answered 405).
        .head(list)
        .post(create)
        .push(
            Router::with_path("{id}")
                .get(get)
                .head(get)
                .patch(patch)
                .delete(delete),
        )
        .push(Router::with_path("{id}/accept").post(accept))
        .push(Router::with_path("{id}/transition").post(transition))
        .push(Router::with_path("{id}/comments").post(comment))
}

/// The per-agent task list and the project board live on their own paths, as
/// the retained routes do (`/api/agents/:name/tasks`, `/api/project-board`).
pub(super) fn extra_router() -> Router {
    Router::new()
        .push(
            Router::with_path("agents/{name}/tasks")
                .get(agent_tasks)
                .head(agent_tasks),
        )
        .push(
            Router::with_path("project-board")
                .get(project_board)
                .head(project_board),
        )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

/// The store-error mapping, the same shape the alerts and usage reads use.
fn store_error(res: &mut Response, error: hagency_store::Error) {
    let (status, code) = match error {
        hagency_store::Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_task_command"),
        hagency_store::Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        hagency_store::Error::Capacity => (StatusCode::SERVICE_UNAVAILABLE, "task_capacity"),
        hagency_store::Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        hagency_store::Error::OutcomeUnknown => (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "tasks_unavailable"),
    };
    refusal(res, status, code);
}

/// The write gate: the CONFIGURE scope, refused with the console's missing
/// scope word BEFORE any store job, so a read-only session changes nothing.
fn check_configure(depot: &Depot, res: &mut Response) -> bool {
    let session = match depot.get_typed::<Session>() {
        Ok(session) => session,
        Err(_) => {
            failed(res, Error::Unauthorized);
            return false;
        }
    };
    match console(depot).and_then(|c| c.0.authority.can_configure(session)) {
        Ok(true) => true,
        Ok(false) => {
            failed(res, Error::ConfigurationForbidden);
            false
        }
        Err(error) => {
            failed(res, error);
            false
        }
    }
}

/// The session's write capability as a payload fact, the way the alerts read
/// serves it: the page hides its controls without the configure scope.
fn permissions(depot: &Depot) -> Value {
    let configure = depot
        .get_typed::<Session>()
        .ok()
        .and_then(|session| console(depot).ok()?.0.authority.can_configure(session).ok())
        .unwrap_or(false);
    json!({"configureResource": configure})
}

/// One task on the wire: the store's row plus `next`, the statuses the SERVER
/// allows from its current one. The page renders a transition control only
/// from this array, so a client-side map can never disagree with the store's
/// (`lib/task-store.js:8-13` is the one map, served from
/// `hagency_store::operator_transitions`) — the same contract the alerts read
/// uses for its own `next`.
fn wire(task: OperatorTask) -> Value {
    let mut value = serde_json::to_value(&task).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "next".to_owned(),
            serde_json::to_value(operator_transitions(&task.status)).unwrap_or(Value::Null),
        );
    }
    value
}

/// A JSON body, exactly one `content-type: application/json` header, bounded.
async fn json_body(req: &mut Request, res: &mut Response, maximum: usize) -> Option<Value> {
    if req.headers().get_all("content-type").iter().count() != 1
        || req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::trim)
            != Some("application/json")
    {
        failed(res, Error::Invalid);
        return None;
    }
    match serde_json::from_slice::<Value>(&body(req, maximum).await.unwrap_or_default()) {
        Ok(value @ Value::Object(_)) => Some(value),
        // The retained routes pass `req.body || {}` straight through, and
        // every normalizer tolerates a missing field; a JSON object is the
        // only shape that cannot mean something else.
        _ => {
            failed(res, Error::Invalid);
            None
        }
    }
}

/// One query parameter, digits only, or the default — the retained
/// `parseTaskPageInt`/`parseTaskPageLimit` accept a non-negative integer and
/// fall back on anything else. A page beyond the store's bound is refused
/// rather than clamped, the way every other bounded native read behaves.
fn page(req: &Request, key: &str) -> Option<u64> {
    req.queries()
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
}

/// The filters the retained list route accepts (`backend-v2.js:13213-13224`).
fn filters(req: &Request) -> Result<TaskFilters, Error> {
    query(
        req,
        &["assignee", "status", "priority", "label", "offset", "limit"],
        512,
    )?;
    let limit = match page(req, "limit") {
        None => None,
        Some(0) => None,
        Some(value) => Some(usize::try_from(value).map_err(|_| Error::Invalid)?),
    };
    Ok(TaskFilters {
        assignee: req.query::<String>("assignee").filter(|v| !v.is_empty()),
        status: req.query::<String>("status").filter(|v| !v.is_empty()),
        priority: req.query::<String>("priority").filter(|v| !v.is_empty()),
        label: req.query::<String>("label").filter(|v| !v.is_empty()),
        offset: page(req, "offset").unwrap_or(0),
        limit,
    })
}

#[handler]
async fn list(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let filters = match filters(req) {
        Ok(filters) => filters,
        Err(error) => {
            failed(res, error);
            return;
        }
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.operator_tasks(filters).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(tasks) => res.render(Json(json!({
            "at_ms": now_ms(),
            "permissions": permissions(depot),
            "unavailable": UNAVAILABLE,
            "tasks": tasks.into_iter().map(wire).collect::<Vec<_>>(),
        }))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn create(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !check_configure(depot, res) {
        return;
    }
    let Some(input) = json_body(req, res, 16 * 1024).await else {
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.create_operator_task(input, now_ms()).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(task) => res.render(Json(json!({"ok": true, "task": wire(task)}))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn get(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.operator_task(id).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(task) => res.render(Json(wire(task))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn patch(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !check_configure(depot, res) {
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(input) = json_body(req, res, 16 * 1024).await else {
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.update_operator_task(id, input).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(task) => res.render(Json(json!({"ok": true, "task": wire(task)}))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn delete(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !check_configure(depot, res) {
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    if req.uri().query().is_some() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.delete_operator_task(id).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(Some(task)) => res.render(Json(json!({"ok": true, "task": wire(task)}))),
        // The retained route answers its own 404 for a missing id
        // (`backend-v2.js:13288-13289`).
        Ok(None) => refusal(res, StatusCode::NOT_FOUND, "not_found"),
        Err(error) => store_error(res, error),
    }
}

/// `POST /api/tasks/:id/accept` (`backend-v2.js:13298-13306`) — the one
/// transition the operator route names with the agent vocabulary; it is the
/// same `transitionTask(id, 'accepted')` call the generic route makes.
#[handler]
async fn accept(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !check_configure(depot, res) {
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store
        .transition_operator_task(id, "accepted".to_owned(), json!({}), now_ms())
        .await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(task) => res.render(Json(json!({"ok": true, "task": wire(task)}))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn transition(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !check_configure(depot, res) {
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(input) = json_body(req, res, 16 * 1024).await else {
        return;
    };
    // `status is required` is the retained route's own 400
    // (`backend-v2.js:13310-13311`), before the store is consulted.
    let Some(status) = input
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
    else {
        refusal(res, StatusCode::BAD_REQUEST, "status_required");
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store
        .transition_operator_task(id, status, input, now_ms())
        .await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(task) => res.render(Json(json!({"ok": true, "task": wire(task)}))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn comment(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if !check_configure(depot, res) {
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(input) = json_body(req, res, 16 * 1024).await else {
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.comment_operator_task(id, input, now_ms()).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(task) => res.render(Json(json!({"ok": true, "task": wire(task)}))),
        Err(error) => store_error(res, error),
    }
}

/// `GET /api/agents/:name/tasks` (`backend-v2.js:13331-13344`): the assignee
/// filter under its own path, `at_ms` + the list envelope.
#[handler]
async fn agent_tasks(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(name) = req
        .param::<String>("name")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && value.len() <= 128)
    else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store
        .operator_tasks(TaskFilters {
            assignee: Some(name),
            ..TaskFilters::default()
        })
        .await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(tasks) => res.render(Json(json!({
            "at_ms": now_ms(),
            "permissions": permissions(depot),
            "unavailable": UNAVAILABLE,
            "tasks": tasks.into_iter().map(wire).collect::<Vec<_>>(),
        }))),
        Err(error) => store_error(res, error),
    }
}

/// `GET /api/project-board` (`backend-v2.js:16025-16051`).
#[handler]
async fn project_board(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &["activity_limit"], 64).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let activity_limit = page(req, "activity_limit").unwrap_or(20);
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.operator_project_board(now_ms(), activity_limit).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(board) => res.render(Json(board)),
        Err(error) => store_error(res, error),
    }
}
