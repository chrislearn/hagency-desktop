//! Requester-facing reads for the console (board #48): the offer book
//! (`GET /api/offer-book`), the contributions list (`GET /api/contributions`)
//! and the engagement PREVIEW (`GET /api/engagements/preview`).
//!
//! All three are scope-free READS, like every other console read (the
//! engagements read lives in `usage.rs`): `authenticate` has already run and no
//! route here changes anything, so no management scope is consulted and no
//! mutation ticket is required. The store owns every projection — these routes
//! add no arithmetic path and invent no field.
//!
//! The preview is a DRY RUN in the strict sense the task demands: it calls the
//! store's own read-only `preview`, which decides nothing and writes nothing.
//! The test suite proves the no-write half by snapshotting the whole database
//! before and after.
use super::{Error, failed, recheck, usage::query};
use crate::{refusal, resources::domain};
use salvo::prelude::*;

pub(super) fn router() -> Router {
    Router::new()
        .push(Router::with_path("offer-book").get(offer_book))
        .push(Router::with_path("contributions").get(contributions))
        .push(Router::with_path("engagements/preview").get(preview))
}

#[handler]
async fn offer_book(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if query(req, &["projectRoomId"], 256).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let room = req.query::<String>("projectRoomId");
    if room.as_deref().is_some_and(|value| value.is_empty()) {
        failed(res, Error::Invalid);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.offer_book(room).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(book) => res.render(Json(book)),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn contributions(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // A list observation takes no selection: any query parameter is refused,
    // the same hygiene the roster and project-sides reads apply.
    if query(req, &[], 0).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.contributions().await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(rows) => res.render(Json(serde_json::json!({
            "contributions": rows,
        }))),
        Err(error) => store_error(res, error),
    }
}

#[handler]
async fn preview(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    // The retained route's own four query keys (`backend-v2.js:15435-15437`).
    if query(
        req,
        &["role", "projectRoomId", "requestedTokens", "ratePerDay"],
        320,
    )
    .is_err()
    {
        failed(res, Error::Invalid);
        return;
    }
    let role = req.query::<String>("role").unwrap_or_default();
    if hagency_core::qualification::check_role(&role).is_err() {
        failed(res, Error::Invalid);
        return;
    }
    let Some(store) = domain(depot, res) else {
        return;
    };
    let result = store.preview(role).await;
    if let Err(error) = recheck(depot) {
        failed(res, error);
        return;
    }
    match result {
        Ok(answer) => res.render(Json(answer)),
        Err(error) => store_error(res, error),
    }
}

fn store_error(res: &mut Response, error: hagency_store::Error) {
    let (status, code) = match error {
        hagency_store::Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_offer_query"),
        hagency_store::Error::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "offer_book_unavailable"),
    };
    refusal(res, status, code);
}
