//! The retirement acts on the wire (TS parity: `lib/matrix-work-executor.js:13-49`).
//!
//! These drive the real `RetireClient` against the fixture's TLS homeserver and
//! assert the exact request the retained executor made: the POST target, the
//! `{}` body, the bearer credential, and the TS/port outcome rule — 2xx is
//! revoked, a *logout*'s 401 `M_UNKNOWN_TOKEN` is already-revoked, a leave's is
//! not, and anything else is the `matrix_http_<status>` refusal.
mod common;
use common::{Fake, Fixture, TOKEN, limits};
use hagency_matrix::{CancellationToken, Collector, RetireClient, RetireVerdict};
use serde_json::json;

fn client(fake: &Fake, token: &str) -> RetireClient {
    RetireClient::new(&fake.endpoint, token, &limits())
        .unwrap()
        .with_root_pem(include_bytes!("fixtures/ca.pem"))
        .unwrap()
}

/// TS executor:41-43 — the leave is `POST .../rooms/{encodeURIComponent(roomId)}/leave`
/// with body `{}` and the credential as the bearer token; 2xx is `Revoked`.
#[tokio::test]
async fn native_retire_leave_room_request_shape() {
    let mut fake = Fake::start(true).await;
    let retire = client(&fake, TOKEN);
    let cancel = CancellationToken::new();
    let (verdict, ()) = tokio::join!(retire.leave_room("!project:example.test", &cancel), async {
        let request = fake.next().await;
        assert_eq!(request.method, "POST");
        // The room id is ONE path segment. The port's shared `Http` path
        // layer leaves `:` literal (the same spelling every other ported
        // room call sends, e.g. `…/rooms/{roomId}/state`); TS's
        // `encodeURIComponent` wrote `%3A` for the same segment, and a
        // homeserver decodes the two to the identical room id.
        assert_eq!(
            request.target,
            "/_matrix/client/v3/rooms/!project:example.test/leave"
        );
        assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(request.body, b"{}".to_vec());
        request.json(200, json!({}));
    });
    assert_eq!(verdict.unwrap(), RetireVerdict::Revoked);
    fake.close().await;
}

/// TS executor:13-17 — the logout is `POST /_matrix/client/v3/logout`, body `{}`,
/// and 401 `M_UNKNOWN_TOKEN` counts as already revoked.
#[tokio::test]
async fn native_retire_logout_request_shape_and_unknown_token() {
    let mut fake = Fake::start(true).await;
    let retire = client(&fake, TOKEN);
    let cancel = CancellationToken::new();
    let (verdict, ()) = tokio::join!(retire.logout(&cancel), async {
        let request = fake.next().await;
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/_matrix/client/v3/logout");
        assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
        assert_eq!(request.body, b"{}".to_vec());
        request.json(401, json!({"errcode": "M_UNKNOWN_TOKEN"}));
    });
    assert_eq!(verdict.unwrap(), RetireVerdict::Revoked);
    fake.close().await;
}

/// The asymmetry the retained executor carries: only a *logout* treats 401
/// `M_UNKNOWN_TOKEN` as already-revoked; a leave answers `matrix_http_401`
/// (executor:16-18 vs :46-48).
#[tokio::test]
async fn native_retire_leave_unknown_token_is_refused() {
    let mut fake = Fake::start(true).await;
    let retire = client(&fake, TOKEN);
    let cancel = CancellationToken::new();
    let (verdict, ()) = tokio::join!(retire.leave_room("!project:example.test", &cancel), async {
        let request = fake.next().await;
        assert!(request.target.ends_with("/leave"));
        request.json(401, json!({"errcode": "M_UNKNOWN_TOKEN"}));
    });
    assert_eq!(verdict.unwrap(), RetireVerdict::Refused(401));
    fake.close().await;
}

/// Any other status is the retained `matrix_http_<status>` refusal, and the
/// credential really is the caller's exact one — not a shared or ambient token.
#[tokio::test]
async fn native_retire_refusal_reports_ts_status_word() {
    let mut fake = Fake::start(true).await;
    let retire = client(&fake, "synthetic-representative-token");
    let cancel = CancellationToken::new();
    let (verdict, ()) = tokio::join!(retire.logout(&cancel), async {
        let request = fake.next().await;
        assert_eq!(request.target, "/_matrix/client/v3/logout");
        assert_eq!(
            request.headers["authorization"],
            "Bearer synthetic-representative-token"
        );
        request.json(403, json!({"errcode": "M_FORBIDDEN"}));
    });
    assert_eq!(verdict.unwrap(), RetireVerdict::Refused(403));
    fake.close().await;
}

/// Route (1) — the LIVE worker's retirement. The running agent already holds an
/// authenticated transport, so its own `Collector` performs the acts: leave
/// every room the host config names, then log out. This is the retained
/// executor's "use the stored credential" arm (matrix-work-executor.js:40-49)
/// and the architect's preferred route.
#[tokio::test]
async fn native_retire_live_worker_leaves_every_room_then_logs_out() {
    let mut fake = Fake::start(true).await;
    let f = Fixture::new();
    let collector = Collector::new(
        f.config(&fake.endpoint)
            .with_root_pem(include_bytes!("fixtures/ca.pem"))
            .unwrap(),
        f.store.clone(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let (retirement, ()) = tokio::join!(collector.retire_agent(&cancel), async {
        // The one room this host config names (`Fixture::config`):
        // `!direct:example.test`.
        let leave = fake.next().await;
        assert_eq!(leave.method, "POST");
        assert_eq!(
            leave.target,
            "/_matrix/client/v3/rooms/!direct:example.test/leave"
        );
        assert_eq!(leave.headers["authorization"], format!("Bearer {TOKEN}"));
        assert_eq!(leave.body, b"{}".to_vec());
        leave.json(200, json!({}));
        let logout = fake.next().await;
        assert_eq!(logout.method, "POST");
        assert_eq!(logout.target, "/_matrix/client/v3/logout");
        assert_eq!(logout.headers["authorization"], format!("Bearer {TOKEN}"));
        assert_eq!(logout.body, b"{}".to_vec());
        logout.json(200, json!({}));
    });
    let retirement = retirement.unwrap();
    assert_eq!(retirement.leaves, vec![RetireVerdict::Revoked]);
    assert_eq!(retirement.logout, RetireVerdict::Revoked);
    assert!(retirement.complete());
    assert!(!retirement.receipt().is_empty());
    fake.close().await;
}
