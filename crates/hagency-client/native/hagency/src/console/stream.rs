//! Console live stream (#26): `GET /console/api/stream` — TS parity with the
//! retained `/api/stream` (lib/backend/sse-adapter.js:20 installRoute): a
//! `text/event-stream` response that writes the `:\n\n` comment heartbeat on
//! connect, broadcasts `event: <name>\ndata: <JSON>\n\n` frames, and keeps the
//! client alive with `: ` comments every 30 s (startKeepalive, default 30000).
//!
//! The retained backend had ONE process and broadcast in-memory at each write
//! site (`broadcastSSE('task_updated', task)`, backend-v2.js:8577 et al.).
//! Native serves from a shared DomainStore whose writers are separate tasks,
//! so the route polls the store's bounded change feed (`console_feed`) — the
//! same freshness question `routerStore.snapshot()` answered (:3655) — and
//! emits a category event when a fingerprint changes. The page then refetches
//! its own bounded read, exactly the division the retained dashboard used.
//!
//! The connection holds NO console permit for its lifetime: the
//! `browser_boundary` semaphore (8) is released before streaming starts, so
//! one live tab cannot starve the console's bounded request budget.
use super::{Error, Session, failed, usage::query};
use crate::refusal;
use crate::resources::domain;
use salvo::prelude::*;
use serde_json::json;
use std::collections::BTreeMap;

/// One entity row of a `console_entities` snapshot, keyed for diffing.
type EntityRow = (String, String, serde_json::Value);

fn rows(feed: &serde_json::Value, category: &str) -> BTreeMap<String, EntityRow> {
    feed[category]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    (
                        row["key"].as_str().unwrap_or_default().to_owned(),
                        (
                            row["key"].as_str().unwrap_or_default().to_owned(),
                            row["state"].as_str().unwrap_or_default().to_owned(),
                            row["entity"].clone(),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// #59: diff two entity snapshots into the retained TS event vocabulary
/// (backend-v2.js broadcastSSE sites): the payload IS the entity —
/// `task_updated` broadcast the task row (:8577), `alert_created` the alert
/// (lib/alert-store.js:332), `approval_requested` `{request_id, agent}`
/// (:2518), `approval_verdict` the verdict fields (:10945),
/// `agent_blocked`/`agent_recovered` the runtime fields (:5497/:5511),
/// `message` the message row (:4554). Events with no native entity are not
/// emitted (see report-59 for the list).
fn diff_events(
    previous: &serde_json::Value,
    current: &serde_json::Value,
) -> Vec<(String, serde_json::Value)> {
    let mut out: Vec<(String, serde_json::Value)> = Vec::new();
    let task_before = rows(previous, "tasks");
    let task_now = rows(current, "tasks");
    for (key, (_, state, entity)) in &task_now {
        match task_before.get(key) {
            None => out.push(("task_created".into(), entity.clone())),
            Some((_, old, _)) if old != state => out.push(("task_updated".into(), entity.clone())),
            _ => {}
        }
    }
    for key in task_before.keys() {
        if !task_now.contains_key(key) {
            out.push(("task_deleted".into(), json!({"id": key, "deleted": true})));
        }
    }
    let alert_before = rows(previous, "alerts");
    let alert_now = rows(current, "alerts");
    for (key, (_, state, entity)) in &alert_now {
        let was = alert_before.get(key).map(|(_, state, _)| state.as_str());
        match was {
            None => out.push(("alert_created".into(), entity.clone())),
            Some(old) if old == "open" && state == "resolved" => {
                out.push(("alert_resolved".into(), entity.clone()))
            }
            Some(old) if old != state => out.push(("alert_updated".into(), entity.clone())),
            _ => {}
        }
    }
    for key in alert_before.keys() {
        if !alert_now.contains_key(key) {
            out.push(("alert_deleted".into(), json!({"dedupe_key": key})));
        }
    }
    let approval_before = rows(previous, "approvals");
    let approval_now = rows(current, "approvals");
    for (key, (_, state, entity)) in &approval_now {
        match approval_before.get(key) {
            None => out.push(("approval_requested".into(), entity.clone())),
            Some((_, old, _)) if old != state => {
                // The verdict fields TS broadcast: request_id, agent, status.
                // The native approval names its engagement, not an agent
                // word — `agent` carries the engagement id.
                let mut payload = entity.clone();
                if let Some(object) = payload.as_object_mut() {
                    object.insert("agent".into(), json!(key));
                }
                out.push(("approval_verdict".into(), payload));
            }
            _ => {}
        }
    }
    let fence_before = rows(previous, "fences");
    let fence_now = rows(current, "fences");
    for (key, (_, state, entity)) in &fence_now {
        let was = fence_before.get(key).map(|(_, state, _)| state.as_str());
        // Emit only on the transition — a still-blocked agent is not news.
        match (was, state.as_str()) {
            (None, "blocked") | (Some("recovered"), "blocked") => {
                out.push(("agent_blocked".into(), entity.clone()))
            }
            (Some("blocked"), "recovered") => out.push(("agent_recovered".into(), entity.clone())),
            _ => {}
        }
    }
    let message_before = rows(previous, "messages");
    let message_now = rows(current, "messages");
    for (key, (_, _, entity)) in &message_now {
        if !message_before.contains_key(key) {
            out.push(("message".into(), entity.clone()));
        }
    }
    // #59 task-graph events (lib/task-graph.js:285-390): node state moves
    // name node_dispatched/node_completed; a graph row appearing names
    // task_graph_created; its head reaching a terminal state names
    // task_graph_completed. Payloads mirror the TS queueEvent shapes.
    let graph_before = rows(previous, "graphs");
    let graph_now = rows(current, "graphs");
    for (key, (_, state, entity)) in &graph_now {
        let was = graph_before.get(key).map(|(_, old, _)| old.as_str());
        let moved = was != Some(state.as_str());
        match state.as_str() {
            "dispatched" if moved => {
                out.push(("task_graph_node_dispatched".into(), entity.clone()))
            }
            "complete" if moved => out.push(("task_graph_node_completed".into(), entity.clone())),
            _ => {}
        }
    }
    let head_before = rows(previous, "graph_heads");
    let head_now = rows(current, "graph_heads");
    for (key, (_, state, entity)) in &head_now {
        let was = head_before.get(key).map(|(_, old, _)| old.as_str());
        match (was, state.as_str()) {
            (None, _) => out.push(("task_graph_created".into(), entity.clone())),
            (Some(old), "complete") if old != "complete" => {
                out.push(("task_graph_completed".into(), entity.clone()))
            }
            _ => {}
        }
    }
    out
}

pub(super) fn router() -> Router {
    Router::with_path("stream")
        .get(stream)
        .push(Router::with_path("snapshot").get(snapshot))
        .push(Router::with_path("events").get(events))
}

/// `GET /console/api/stream` — the SSE wire itself. Snapshot-first so a
/// reconnecting page learns the current fingerprints without a separate
/// round trip; then change events; then keepalive comments.
#[handler]
async fn stream(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // The connection is long-lived; a permit would be held forever. The
    // boundary already authenticated the browser; this handler only reads
    // the store.
    depot.remove("console_permit");
    // The retained route ignores the query string entirely (sse-adapter
    // installRoute): refuse any parameters rather than inventing semantics.
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    // A session is still required — the stream is same-origin console data.
    if depot.get_typed::<Session>().is_err() {
        failed(res, Error::Unauthorized);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let mut last = match store.console_feed().await {
        Ok(feed) => feed["version"].as_str().unwrap_or_default().to_owned(),
        Err(_) => String::new(),
    };
    let headers = [
        ("content-type", "text/event-stream; charset=utf-8"),
        ("cache-control", "no-cache"),
    ];
    for (name, value) in headers {
        if let Ok(parsed) = value.parse() {
            res.headers_mut().insert(name, parsed);
        }
    }
    // The response is only SENT once this handler returns; the stream body
    // is the channel below, written by a spawned task that outlives the
    // request. TS parity: the retained route wrote `:\n\n` on connect and
    // kept the socket open via the clients set (sse-adapter.js:20-31).
    let mut sender = res.channel();
    tokio::spawn(async move {
        let _ = sender.send_data(":\n\n".to_owned()).await;
        let _ = sender
            .send_data(format!(
                "event: hello\ndata: {}\n\n",
                json!({"version": last})
            ))
            .await;
        // #59: the entity baseline. The first poll diffs against it, so a
        // connect between two writes replays nothing and the next write
        // emits its named event — the retained clients-set behaviour, where
        // only live subscribers saw broadcasts.
        let mut previous = match store.console_entities().await {
            Ok(entities) => entities,
            Err(_) => serde_json::Value::Null,
        };
        // Poll cadence: the console page polled at 15 s; the feed read is
        // one bounded query, so 1 s keeps a live page tight without touching
        // any writer. The first interval tick fires immediately.
        let mut poll = tokio::time::interval(std::time::Duration::from_secs(1));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(30));
        keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = keepalive.tick() => {
                    if sender.send_data(":\n\n".to_owned()).await.is_err() {
                        return;
                    }
                }
                _ = poll.tick() => {
                    // #59 named events first: the TS vocabulary with the
                    // entity as the payload (task_updated :8577, message
                    // :4554, approval_verdict :10945, agent_blocked :5497,
                    // alert_created lib/alert-store.js:332, …). One bounded
                    // entity read; the diff is in-memory.
                    if let Ok(current) = store.console_entities().await
                        && current != previous {
                            for (name, payload) in diff_events(&previous, &current) {
                                let _ = sender
                                    .send_data(format!(
                                        "event: {name}\ndata: {}\n\n",
                                        payload
                                    ))
                                    .await;
                            }
                            previous = current;
                        }
                    let Ok(feed) = store.console_feed().await else {
                        continue;
                    };
                    let version = feed["version"].as_str().unwrap_or_default().to_owned();
                    if !version.is_empty() && version != last {
                        for category in ["agents", "tasks", "alerts"] {
                            let _ = sender
                                .send_data(format!(
                                    "event: {}\ndata: {}\n\n",
                                    category,
                                    json!({
                                        "version": feed[category]["version"],
                                        "count": feed[category]["count"],
                                        "feed_version": version,
                                    })
                                ))
                                .await;
                        }
                        last = version;
                    }
                }
            }
        }
    });
}

/// `GET /console/api/stream/snapshot` — the bounded one-shot feed read
/// (router snapshot parity, store.ts:3655): the page's cursor baseline.
#[handler]
async fn snapshot(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    if depot.get_typed::<Session>().is_err() {
        failed(res, Error::Unauthorized);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    match store.console_feed().await {
        Ok(feed) => res.render(Json(feed)),
        Err(error) => failure(res, error),
    }
}

/// `GET /console/api/stream/events?after=` — router events parity
/// (store.ts:3811 `eventsAfter`): the categories whose version changed
/// since the cursor, plus the new cursor. Bounded to one page.
#[handler]
async fn events(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let prepared = async {
        query(req, &["after"], 160)?;
        let after = req.query::<String>("after").unwrap_or_default();
        if !after.is_empty()
            && !after
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::Invalid);
        }
        Ok::<_, Error>(after)
    }
    .await;
    let after = match prepared {
        Ok(value) => value,
        Err(error) => {
            failed(res, error);
            return;
        }
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let current = match store.console_feed().await {
        Ok(feed) => feed,
        Err(error) => {
            failure(res, error);
            return;
        }
    };
    let version = current["version"].as_str().unwrap_or_default().to_owned();
    let mut changed: Vec<&str> = Vec::new();
    if after.is_empty() || after != version {
        // One page: every category that differs from the cursor's feed. The
        // cursor is the whole-feed version, so a mismatch means at least one
        // category changed; report all three with their own versions so the
        // page can diff per-category.
        for category in ["agents", "tasks", "alerts"] {
            changed.push(category);
        }
    }
    res.render(Json(json!({
        "low_watermark": 0,
        "high_watermark": version,
        "gap": false,
        "events": changed
            .into_iter()
            .map(|kind| json!({
                "kind": format!("{kind}_changed"),
                "version": current[kind]["version"],
                "count": current[kind]["count"],
            }))
            .collect::<Vec<_>>(),
    })));
}

fn failure(res: &mut Response, error: hagency_store::Error) {
    let (status, code) = match error {
        hagency_store::Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_stream_query"),
        hagency_store::Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        hagency_store::Error::OutcomeUnknown => (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "stream_unavailable"),
    };
    refusal(res, status, code);
}
