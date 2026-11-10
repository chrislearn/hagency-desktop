//! Task #12's console surface: the pending-invitations list and the
//! operator's accept/decline — the retained `GET /api/matrix/
//! pending-invites` and `POST /api/matrix/pending-invites/decide`
//! (`backend-v2.js:10819-10855`) on the native console API.
//!
//! The list is PENDING-ONLY by construction: the operator's question is
//! "what needs me" (`lib/pending-invite-store.js` list, pending default).
//! The decide route records the decision and says `queued: true` —
//! deliberately never that the agent has joined, which this surface
//! cannot know; the invite poller consumes its worklist afterwards and
//! retries refusals (ADR-183).
//!
//! TS error statuses are kept whole (the store's own two refusals are
//! distinct): 400 for a missing room/agent, 404 for an unknown
//! invitation, 409 `already_{state}` for one already answered; the bodies
//! use the console's refusal convention.
use super::engagements::check_lifecycle;
use super::{Error, body, failed, recheck, usage::query};
use crate::{refusal, resources::domain};
use salvo::prelude::*;
use serde::Deserialize;

pub(super) fn router() -> Router {
    Router::with_path("matrix/pending-invites")
        .get(list)
        .push(Router::with_path("decide").post(decide))
}

/// The list: `{ok:true, invites:[…], pending:N}` — every row exactly the
/// TS backend's eight camelCase keys, newest `seenAt` first.
#[handler]
async fn list(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // A list observation takes no selection: any query parameter is
    // refused, the same hygiene the roster and project-sides apply. The
    // TS `?state=` arm is history browsing this console has no surface
    // for; pending is the actionable set.
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.pending_invites().await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(invites) => {
            let pending = invites.len();
            let mut value = serde_json::json!({ "ok": true, "pending": pending });
            value["invites"] = serde_json::to_value(&invites).expect("plain JSON fields");
            res.render(Json(value));
        }
        Err(_) => refusal(res, StatusCode::SERVICE_UNAVAILABLE, "invites_unavailable"),
    }
}

/// The TS decide body (`backend-v2.js:10835-10840`): `projectRoomId`
/// (camelCase, the TS store's own key) or `project_room_id` (the bridge's
/// snake_case spelling), `agent`, `accept === true`. Closed key set.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decide {
    /// The TS backend's own camelCase key is primary; the bridge's
    /// snake_case spelling is accepted too (`backend-v2.js:10837`).
    #[serde(rename = "projectRoomId", alias = "project_room_id")]
    project_room_id: Option<String>,
    agent: Option<String>,
    accept: Option<bool>,
}

/// Accept or decline one invitation. A decision is an operator act behind
/// the console session — the same sign-in the project-side write requires —
/// refused before any store read.
#[handler]
async fn decide(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // The decide route takes no query.
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    if !check_lifecycle(depot, res) {
        return;
    }
    let raw = match body(req, 1024).await {
        Ok(raw) => raw,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let input: Decide = match serde_json::from_slice(&raw) {
        Ok(input) => input,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    // TS: 400 'projectRoomId and agent are required'.
    let Some(room_id) = input.project_room_id else {
        refusal(res, StatusCode::BAD_REQUEST, "bad_request");
        return;
    };
    let Some(agent) = input.agent else {
        refusal(res, StatusCode::BAD_REQUEST, "bad_request");
        return;
    };
    let room_id = room_id.trim().to_owned();
    let agent = agent.trim().to_owned();
    if room_id.is_empty()
        || room_id.len() > 256
        || agent.is_empty()
        || agent.len() > 64
        || !agent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        refusal(res, StatusCode::BAD_REQUEST, "bad_request");
        return;
    }
    let accepted = input.accept == Some(true);
    let Some(store) = domain(depot, res) else {
        return;
    };
    // TS's two refusals are distinct: 404 for an unknown invitation, 409
    // `already_{state}` for one already answered. Read the prior state
    // first — the store's settle is an overwrite (the bridge's own
    // settle is), so the console owns the conflict refusal.
    let prior = match store.pending_invite(room_id.clone(), agent.clone()).await {
        Ok(prior) => prior,
        Err(_) => {
            refusal(res, StatusCode::SERVICE_UNAVAILABLE, "invites_unavailable");
            return;
        }
    };
    let Some(prior) = prior else {
        refusal(res, StatusCode::NOT_FOUND, "not_found");
        return;
    };
    if prior.state != "pending" {
        refusal(
            res,
            StatusCode::CONFLICT,
            Box::leak(format!("already_{}", prior.state).into_boxed_str()),
        );
        return;
    }
    // `joined: false` — the console records the decision; the join is the
    // poller's to perform and retry (ADR-183). The response says queued,
    // exactly the TS wording, because that is all that is true here.
    let result = store
        .settle_pending_invite(room_id, agent, accepted, false, "operator".to_owned())
        .await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(Some(record)) => {
            let mut value = serde_json::json!({ "ok": true, "queued": true });
            value["invite"] = serde_json::to_value(&record).expect("plain JSON fields");
            res.render(Json(value));
        }
        // The prior read above is the same row: this arm means it vanished
        // between the two jobs, which is not a state an operator can act on.
        Ok(None) => refusal(res, StatusCode::NOT_FOUND, "not_found"),
        Err(_) => refusal(res, StatusCode::SERVICE_UNAVAILABLE, "invites_unavailable"),
    }
}
