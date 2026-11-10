//! Host-initiated context compaction (board #53, TS `POST /api/runtime/compact`,
//! `backend-v2.js:13061-13077`).
//!
//! The TS route is a bearer-authenticated operator call: 400 when `agent` is
//! missing, `{ok:true, ignored:'agent-not-found', agent}` when the agent record
//! is absent, else `emitRuntimeCompactEvent(…, {mode,marker,source,summary})`.
//!
//! Native has no host-side compaction *initiator*: the owned Codex runtime only
//! OBSERVES compaction (`hagency-runtime/src/codex/session/state.rs:212,324`
//! `thread/compacted` / `contextCompaction`, `hooks.rs:13-14` `preCompact` /
//! `postCompact`), and the warm host's `Command` enum
//! (`hagency-execution/src/warm.rs:198`) is `{Dispatch, Observe, Activate}`.
//! So the honest port is the TS shape with the unsupported answer for a known
//! agent — never a fabricated "compacted" claim about state the runtime did not
//! actually compact.
use crate::{refusal, resources::domain};
use hagency_core::project::AgentName;
use salvo::prelude::*;
use serde::Deserialize;

pub(crate) fn router() -> Router {
    Router::with_path("runtime/compact").post(compact)
}

#[derive(Deserialize)]
struct CompactBody {
    agent: Option<String>,
}

#[handler]
async fn compact(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(store) = domain(depot, res) else {
        return;
    };
    let Some(body) = crate::resources::body::<CompactBody>(req, depot, res).await else {
        return;
    };
    // TS `normalizeAgentName(req.body?.agent)` then 400 when empty.
    let Some(agent) = body.agent else {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(serde_json::json!({"error": "agent required"})));
        return;
    };
    let Ok(agent) = AgentName::try_from(agent.clone()) else {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(serde_json::json!({"error": "agent required"})));
        return;
    };
    // Agent-not-found is `{ok:true, ignored:'agent-not-found', agent}` — the
    // same shape the TS route returns for an absent agent record.
    match store.agent_detail(agent.as_str()).await {
        Ok(None) => {
            res.render(Json(serde_json::json!({
                "ok": true,
                "ignored": "agent-not-found",
                "agent": agent.as_str(),
            })));
        }
        Ok(Some(_)) => {
            // The agent exists; native has no compaction initiator to reach its
            // runtime, so this is honestly reported rather than invented.
            res.render(Json(serde_json::json!({
                "ok": true,
                "ignored": "unsupported",
                "agent": agent.as_str(),
            })));
        }
        Err(error) => {
            let (status, code) = match error {
                hagency_store::Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
                hagency_store::Error::OutcomeUnknown => {
                    (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown")
                }
                _ => (StatusCode::SERVICE_UNAVAILABLE, "domain_unavailable"),
            };
            refusal(res, status, code);
        }
    }
}
