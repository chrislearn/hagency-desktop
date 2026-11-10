//! Per-side allocation and budget (backend-v2.js:9541, :9567): the operator's
//! 「真配额」 mutation and the what-it-has / what-it-promised / what-is-left
//! read. The allocation write is an operator decision over the fleet's own
//! budget, so it sits behind the same `Scope::Configuration` the resource
//! configuration writes require (brief 28's rule: one concept, one scope)
//! rather than the lifecycle scope — nothing about the allocation mints,
//! stops or registers an agent. Reads take no scope, the same class as the
//! project-sides list they complete. The wire shapes are the retained ones:
//! `{ok:true, side, budget}` on write, `{ok:true, side_id, ...budget}` on
//! read, with the store owning every verdict (not-found, overflow).
use super::{Error, body, failed, recheck, usage::query};
use crate::{refusal, resources::domain};
use salvo::prelude::*;
use serde::Deserialize;

pub(super) fn router() -> Router {
    Router::with_path("project-sides/{id}")
        .push(Router::with_path("allocation").put(set_allocation))
        .push(Router::with_path("budget").get(read_budget))
}

/// `allocated_tokens` OR the camelCase spelling, exactly the retained
/// acceptance (`req.body?.allocated_tokens ?? req.body?.allocatedTokens`);
/// absent means clear to NULL — unallocated, which is not unlimited
/// (lib/project-side-store.js:443-452).
#[derive(Deserialize)]
struct Allocation {
    allocated_tokens: Option<Option<u64>>,
    #[serde(rename = "allocatedTokens")]
    allocated_tokens_camel: Option<Option<u64>>,
}

#[handler]
async fn set_allocation(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    let session = match depot.get_typed::<super::authority::Session>() {
        Ok(session) => session,
        Err(_) => {
            failed(res, Error::Unauthorized);
            return;
        }
    };
    match super::console(depot).and_then(|c| c.0.authority.can_configure(session)) {
        Ok(true) => {}
        Ok(false) => {
            failed(res, Error::ConfigurationForbidden);
            return;
        }
        Err(error) => {
            failed(res, error);
            return;
        }
    }
    let raw = match body(req, 1024).await {
        Ok(raw) => raw,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let input: Allocation = match serde_json::from_slice(&raw) {
        Ok(input) => input,
        Err(_) => {
            failed(res, Error::Invalid);
            return;
        }
    };
    let allocated = input
        .allocated_tokens
        .or(input.allocated_tokens_camel)
        .flatten();
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.set_side_allocation(id.clone(), allocated).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(()) => {
            // `{ok, side, budget}` (backend-v2.js:9545): the write's reply
            // carries the SAME budget the read route serves and the side
            // record the TS route returned — native's six-key projection is
            // the whole side record it has; the other retained columns are
            // the list route's server-owned `unavailable` names.
            let sides = store.project_sides().await;
            if let Err(error) = recheck(depot) {
                failed(res, error);
                return;
            }
            match sides {
                Ok(sides) => {
                    let Some(side) = sides.into_iter().find(|s| s.id == *id) else {
                        refusal(res, StatusCode::NOT_FOUND, "not_found");
                        return;
                    };
                    match store.side_budget(id.clone()).await {
                        Ok(budget) => res.render(Json(serde_json::json!({
                            "ok": true,
                            "side": side,
                            "budget": budget,
                        }))),
                        Err(error) => store_error(res, error),
                    }
                }
                Err(error) => store_error(res, error),
            }
        }
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn read_budget(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(id) = req.param::<String>("id") else {
        failed(res, Error::Invalid);
        return;
    };
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.side_budget(id.clone()).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        // `{ok, sideId, ...budget, ...commitments}` — the retained route
        // SPREADS both maps into the reply (backend-v2.js:9571), so the
        // budget fields and the breakdown travel flat beside `sideId`, not
        // nested under a `budget` key. Key ORDER is not part of the
        // contract (serde_json's map sorts; the console validator matches
        // the exact key SET, as it does for every native read).
        Ok(budget) => {
            let mut flat = match serde_json::to_value(&budget) {
                Ok(serde_json::Value::Object(map)) => map,
                _ => {
                    failed(res, Error::Invalid);
                    return;
                }
            };
            flat.insert("ok".into(), serde_json::json!(true));
            flat.insert("sideId".into(), serde_json::json!(id));
            res.render(Json(serde_json::Value::Object(flat)));
        }
        Err(error) => store_error(res, error),
    }
}

fn store_error(res: &mut Response, error: hagency_store::Error) {
    let (status, code) = match error {
        hagency_store::Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_side_budget"),
        hagency_store::Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        hagency_store::Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        hagency_store::Error::OutcomeUnknown => (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "side_budget_unavailable"),
    };
    refusal(res, status, code);
}
