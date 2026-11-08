//! Open ceiling overrun alerts behind the existing operator authentication
//! boundary (ADR-124 slice b). Publication only: an alert is diagnostic and
//! never confers authority; the sweep that writes the rows is in the store.
//! The transition route (ADR-124 amendment) mutates DISPLAY STATE only.
use crate::{
    refusal,
    resources::{body, domain},
};
use hagency_store::{ALERT_STATUSES, AlertTransition, Error, MAX_OPEN_CEILING_ALERTS};
use salvo::prelude::*;
use serde::Deserialize;

pub(crate) fn router() -> Router {
    Router::with_path("alerts")
        .get(list)
        // The retained `/api/alerts/stats` BEFORE `{key}` — salvo matches
        // the literal segment first, so `stats` never shadows an id.
        .push(Router::with_path("stats").get(stats))
        .push(
            Router::with_path("{key}")
                .get(get)
                .patch(patch)
                .delete(delete),
        )
        .push(Router::with_path("{key}/transition").post(transition))
        .push(Router::with_path("{key}/notes").post(add_note))
}

/// The operator transition body: the retained `status` plus optional
/// display provenance (`actor`, bounded at the store), the assignee an
/// `assigned` transition adopts, and the suppression window
/// (`backend-v2.js:16118-16122`: actor, assignee, suppressUntil). Notes
/// carry operator text only, bounded at the store.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransitionBody {
    #[serde(alias = "status")]
    to: String,
    actor: Option<String>,
    note: Option<String>,
    assignee: Option<String>,
    suppress_until_ms: Option<u64>,
}

/// One operator display-state transition (ADR-124 amendment): the SAME
/// boundary and authority as the read (this router mounts under the
/// `authorize` hoop). A transition never enforces anything — display state
/// only; the refusal words match the list route's vocabulary.
#[handler]
async fn transition(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(key) = req.param::<String>("key") else {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_alert_transition");
        return;
    };
    if key.is_empty() || key.len() > 256 {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_alert_transition");
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Some(value) = body::<serde_json::Value>(req, depot, res).await else {
        return;
    };
    let input = match serde_json::from_value::<TransitionBody>(value) {
        Ok(input) => input,
        Err(_) => {
            refusal(res, StatusCode::BAD_REQUEST, "invalid_alert_transition");
            return;
        }
    };
    // `to` must be one of the four states; legality of the PAIR is the
    // store's to refuse (bad_transition) — one map, one owner.
    let Some(to) = ALERT_STATUSES.iter().find(|state| **state == input.to) else {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_alert_transition");
        return;
    };
    // Statement time; a clock fault is the service's own unavailability,
    // never `busy` (the brief-20 E2 rule).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    let Some(now) = now else {
        refusal(res, StatusCode::SERVICE_UNAVAILABLE, "alerts_unavailable");
        return;
    };
    let command = AlertTransition {
        key,
        to,
        actor: input.actor.unwrap_or_default(),
        note: input.note,
        assignee: input.assignee,
        suppress_until_ms: input.suppress_until_ms,
        now,
    };
    match store.transition_ceiling_alert(command).await {
        Ok(alert) => res.render(Json(alert)),
        Err(error) => match error {
            Error::Invalid(_) => refusal(res, StatusCode::BAD_REQUEST, "bad_transition"),
            Error::NotFound => refusal(res, StatusCode::NOT_FOUND, "not_found"),
            Error::Busy => refusal(res, StatusCode::SERVICE_UNAVAILABLE, "busy"),
            Error::OutcomeUnknown => refusal(res, StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
            _ => refusal(res, StatusCode::SERVICE_UNAVAILABLE, "alerts_unavailable"),
        },
    }
}

fn limit_query(req: &Request) -> Result<u32, ()> {
    if req.uri().query().is_some_and(|q| q.len() > 64) {
        return Err(());
    }
    let fields = req.queries();
    if fields.is_empty() {
        return Ok(100);
    }
    if fields.len() != 1 {
        return Err(());
    }
    let values = fields.get_vec("limit").ok_or(())?;
    if values.len() != 1 || values[0].is_empty() || !values[0].bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    let limit: u32 = values[0].parse().map_err(|_| ())?;
    // Deliberate divergence from the retained route's clamp
    // (`Math.min(parseInt(limit) || 100, 500)`, alert-store.js:410): an
    // out-of-range limit is refused, not silently clamped, matching every
    // other bounded read behind this boundary.
    if limit == 0 || limit > MAX_OPEN_CEILING_ALERTS as u32 {
        return Err(());
    }
    Ok(limit)
}

#[handler]
async fn list(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Ok(limit) = limit_query(req) else {
        refusal(res, StatusCode::BAD_REQUEST, "invalid_alerts_query");
        return;
    };
    // The read clock is statement time, mirroring the retained GET /api/alerts
    // (backend-v2.js:16069-16079): no at_ms parameter exists to honor.
    let at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or_default();
    match store.open_ceiling_alerts(limit).await {
        Ok(alerts) => res.render(Json(serde_json::json!({"at_ms": at_ms, "alerts": alerts}))),
        Err(error) => {
            let (status, code) = match error {
                Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_alerts_query"),
                Error::Schema => (StatusCode::SERVICE_UNAVAILABLE, "alerts_corrupt"),
                Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
                Error::OutcomeUnknown => (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "alerts_unavailable"),
            };
            refusal(res, status, code);
        }
    }
}

/// The store-error mapping every alert route shares
/// (`respondAlertStoreError`, `backend-v2.js:16071-16077`): a bad request or
/// transition is 400, a missing alert is 404, a locked store is 503.
fn alert_error(res: &mut Response, error: Error) {
    let (status, code) = match error {
        Error::Invalid(_) => (StatusCode::BAD_REQUEST, "bad_request"),
        Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        Error::OutcomeUnknown => (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "alerts_unavailable"),
    };
    refusal(res, status, code);
}

/// Statement time for the mutating routes; a clock fault is the service's
/// own unavailability (the brief-20 E2 rule), never `busy`.
fn statement_time(res: &mut Response) -> Option<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    if now.is_none() {
        refusal(res, StatusCode::SERVICE_UNAVAILABLE, "alerts_unavailable");
    }
    now
}

/// The retained stats read (`backend-v2.js:16084-16086`): every status
/// bucket zero-seeded, severity buckets over non-resolved rows only.
#[handler]
async fn stats(_req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    match store.alert_stats().await {
        // `stats` cannot be the binding: `#[handler]` emits a unit struct of
        // that name, so the pattern would be read as the struct, not a new
        // binding (E0308/E0530).
        Ok(counts) => res.render(Json(counts)),
        Err(error) => alert_error(res, error),
    }
}

/// The retained single-alert read (`backend-v2.js:16088-16091`): the alert
/// record with its notes history embedded, exactly like the retained
/// record's `notes` array.
#[handler]
async fn get(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(key) = req.param::<String>("key") else {
        refusal(res, StatusCode::NOT_FOUND, "not_found");
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    match store.get_alert(key).await {
        Ok((alert, notes)) => {
            let mut value = serde_json::to_value(&alert).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "notes".to_owned(),
                    serde_json::to_value(&notes).unwrap_or_default(),
                );
            }
            res.render(Json(value));
        }
        Err(error) => alert_error(res, error),
    }
}

/// The retained note append (`backend-v2.js:16129-16138`): `author`
/// defaults to 'operator', empty text is the retained 400
/// 'note text required'.
#[handler]
async fn add_note(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(key) = req.param::<String>("key") else {
        refusal(res, StatusCode::NOT_FOUND, "not_found");
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Some(value) = body::<serde_json::Value>(req, depot, res).await else {
        return;
    };
    let author = value
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("operator")
        .to_owned();
    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let Some(now) = statement_time(res) else {
        return;
    };
    match store.add_alert_note(key, author, text, now).await {
        Ok((alert, notes)) => {
            let mut value = serde_json::to_value(&alert).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "notes".to_owned(),
                    serde_json::to_value(&notes).unwrap_or_default(),
                );
            }
            res.render(Json(serde_json::json!({"ok": true, "alert": value})));
        }
        Err(error) => alert_error(res, error),
    }
}

/// The retained PATCH (`backend-v2.js:16140-16147`): an absent field is
/// untouched; an explicit `null` clears it — both like the retained
/// `fields.x !== undefined` check feeding `normalizeText`, which maps
/// non-strings to null. Empty-after-trim strings clear the same way.
#[handler]
async fn patch(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(key) = req.param::<String>("key") else {
        refusal(res, StatusCode::NOT_FOUND, "not_found");
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Some(value) = body::<serde_json::Value>(req, depot, res).await else {
        return;
    };
    // `patch` cannot be the binding name: `#[handler]` emits a unit struct of
    // that name for this function, so `let mut patch` shadows it (E0530).
    let mut fields = hagency_store::AlertPatch::default();
    if let Some(object) = value.as_object() {
        // A present-but-null field clears (the retained `normalizeText`
        // yields null); the store's trim-empty-to-None rule carries both.
        let take = |name: &str, slot: &mut Option<String>| {
            if let Some(v) = object.get(name) {
                *slot = Some(v.as_str().unwrap_or_default().to_owned());
            }
        };
        take("linkedTaskId", &mut fields.linked_task_id);
        take("owner", &mut fields.owner);
        take("assignee", &mut fields.assignee);
        take("sourceAgent", &mut fields.source_agent);
        take("runbook", &mut fields.runbook);
        take("impact", &mut fields.impact);
        take("recoveryCondition", &mut fields.recovery_condition);
        take("exitCondition", &mut fields.recovery_condition);
        if let Some(tags) = object.get("tags").and_then(|v| v.as_array()) {
            fields.tags = Some(
                tags.iter()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect(),
            );
        }
    }
    match store.update_alert(key, fields).await {
        Ok(alert) => res.render(Json(serde_json::json!({"ok": true, "alert": alert}))),
        Err(error) => alert_error(res, error),
    }
}

/// The retained DELETE (`backend-v2.js:16149-16157`): the deleted row
/// rides the reply like the retained `{ok: true, alert}`.
#[handler]
async fn delete(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(key) = req.param::<String>("key") else {
        refusal(res, StatusCode::NOT_FOUND, "not_found");
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    match store.delete_alert(key).await {
        Ok(alert) => res.render(Json(serde_json::json!({"ok": true, "alert": alert}))),
        Err(error) => alert_error(res, error),
    }
}
