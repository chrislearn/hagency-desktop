//! The read-only agent detail (board #22, TS `backend-v2.js:12155`
//! `GET /api/agents/:name`): one agent the service knows, served from the
//! same agent-keyed store read (`DomainRepository::agent_detail`) the
//! roster uses — one transaction-consistent projection, never a second
//! arithmetic path. Same read class as the roster: mounted under the API
//! sub-router's `authenticate` hoop with NO scope. The wire item carries
//! the agent-keyed identity plus the resource it works from, the rooms
//! its sessions bind, its current live dispatch and its recent tasks —
//! scalar keys and two bounded lists of flat objects, so no credential
//! home, workspace path or tmux target can travel inside one. The client
//! validator refuses any key outside the declared set, so a future
//! widening fails the whole read instead of leaking silently.
//!
//! The TS route 404s on an unknown agent; so does this one — the store's
//! `None` (no engagement names the agent) maps to NOT_FOUND, and an
//! invalid name shape is refused as BAD_REQUEST before any store work.
use super::{Error, failed, recheck, usage::query};
use crate::resources::domain;
use hagency_core::project::AgentName;
use salvo::prelude::*;

/// The agent-name path parameter: validated as an AgentName (the same
/// shape the engagement projection carries), never treated as an
/// identifier that could address a session, authority or filesystem.
fn agent_name(req: &Request) -> Result<String, Error> {
    let name = req.param::<String>("name").ok_or(Error::Invalid)?;
    AgentName::try_from(name.clone()).map_err(|_| Error::Invalid)?;
    Ok(name)
}

#[handler]
pub(super) async fn detail(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // The detail takes no selection: any query parameter is refused, the
    // same hygiene the roster read applies to its own allowlist.
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
    let result = store.agent_detail(&name).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(Some(agent)) => res.render(Json(agent)),
        Ok(None) => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(serde_json::json!({
                "error": "agent not found",
                "code": "agent_not_found",
            })));
        }
        Err(error) => super::resources::failure(res, error),
    }
}
