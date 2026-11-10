//! TS oracle: `tests/api-approvals.test.js`, `tests/approval-owner-can-see-it.test.js`,
//! `tests/approval-store.test.js` (the HTTP halves).
//!
//! Ported into the SERVICE crate, driving the real console HTTP surface through
//! the crate's own console fixture (`tests/console/fixture.rs` + the `session`/
//! `get`/`post` helpers in `tests/console.rs`). Cases whose TS feature is a
//! bridge-secret route native does not serve (`/api/approvals*`) become
//! `#[ignore = "parity gap: ..."]` HERE, in the right crate, rather than a
//! cross-crate skip.
use super::*;

/// Two owner approvals seeded behind the store's real schema — the same rows
/// `tests/console/approvals.rs` seeds (the console fixture stages none).
fn seed_approvals(state: &std::path::Path, engagement: &str) {
    let mut db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute(
        "INSERT INTO approval_contexts(id,dispatch_id,fence,engagement_id,digest,config) \
         VALUES('ctx_ts','private_dispatch',0,?1,'ts_digest','{}')",
        [engagement],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO owner_approvals(id,source_key,context_id,digest,config,scope_key,scope_kind,description,state,choice,grant_id,expires_at) \
         VALUES('ts_pending','src_ts_pending','ctx_ts','ts_digest','{}','task:echo','task',NULL,'pending',NULL,NULL,12000)",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO owner_approvals(id,source_key,context_id,digest,config,scope_key,scope_kind,description,state,choice,grant_id,expires_at) \
         VALUES('ts_decided','src_ts_decided','ctx_ts','ts_digest','{}',NULL,NULL,NULL,'decided','\"once\"',NULL,12000)",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// TS `api-approvals.test.js:28` — *bridge-owned binding and one-shot verdict
/// flow are enforced*. Native serves the owner approval through the CONSOLE
/// (operator session) and the RUNNER (capability), never a bridge-secret
/// `/api/approvals` route, so the native observable asserted here is the
/// console observation surface: the seven named keys and the `state`/`choice`
/// words, with the one-shot `choice` present exactly when decided.
#[tokio::test]
async fn ts_owner_approval_api_observation_surface() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    seed_approvals(&f.root.path().join("state"), &f.engagement);
    let cookie = session(&service).await;
    // The list is the console's own page shape.
    let mut listed = get("/console/api/approvals", &cookie).send(&service).await;
    assert_eq!(listed.status_code, Some(StatusCode::OK));
    let body = listed.take_json::<Value>().await.unwrap();
    let rows = body["approvals"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "both seeded approvals are observable");
    // Each row is exactly the seven declared keys — no owner, no room, no card.
    // serde_json sorts object keys, so assert the SET, not the source order.
    for row in rows {
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut expected = [
            "id",
            "state",
            "choice",
            "reusableScope",
            "expiresAt",
            "engagementId",
            "projectRoomId",
        ];
        expected.sort_unstable();
        assert_eq!(
            keys, expected,
            "the named-column projection, never a wider row"
        );
    }
    // One-shot: the decided row carries its choice, the pending one does not.
    let mut single = get("/console/api/approvals/ts_pending", &cookie)
        .send(&service)
        .await;
    assert_eq!(single.status_code, Some(StatusCode::OK));
    let pending = single.take_json::<Value>().await.unwrap();
    assert_eq!(pending["state"], json!("pending"));
    assert_eq!(pending["choice"], Value::Null);
    assert_eq!(pending["reusableScope"], json!(true));
    let mut decided = get("/console/api/approvals/ts_decided", &cookie)
        .send(&service)
        .await;
    assert_eq!(decided.status_code, Some(StatusCode::OK));
    assert_eq!(
        decided.take_json::<Value>().await.unwrap()["state"],
        json!("decided")
    );
    f.close().await;
}

/// TS `api-approvals.test.js:145` — *missing_owner_denies_without_admin_fallback*.
/// Native refuses an unknown approval id with the named `not_found` code rather
/// than inventing or falling back to an owner: the console single read 404s.
#[tokio::test]
async fn ts_owner_approval_unknown_id_denies_without_fallback() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    seed_approvals(&f.root.path().join("state"), &f.engagement);
    let cookie = session(&service).await;
    let mut response = get("/console/api/approvals/ts_nope", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        json!("not_found")
    );
    f.close().await;
}

/// TS `approval-store.test.js` (the bounded grant-revocation half): revocation
/// only REMOVES authority — an operator's revocation is the bounded two-key
/// receipt, and an unknown grant is `not_found`, never a fabricated success.
/// integ is SINGLE-LOGIN (operator decision, `42a764c6`): every logged-in
/// session may revoke, so the retired scope refusal is no longer an outcome and
/// this case asserts the receipt path + the 404.
#[tokio::test]
async fn ts_approval_grant_revocation_is_a_bounded_receipt() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    // ONE login is the whole console (the operator's one-login decision):
    // an ordinary session carries the grant-revocation permission, and an
    // unknown grant is `not_found`, never a fabricated success.
    let cookie = session(&service).await;
    let mut response = TestClient::delete(format!("{BASE}/console/api/approvals/grants/grant_ts"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        json!("not_found")
    );
    f.close().await;
}

/// TS `api-approvals.test.js:167` — *bridge approval routes fail closed when the
/// bridge secret is not configured*. Native has no `MATRIX_BRIDGE_SECRET`-gated
/// approval route at all: an unconfigured bridge cannot open one, which is the
/// fail-closed outcome reached structurally rather than by a runtime check.
#[test]
#[ignore = "parity gap: native serves no MATRIX_BRIDGE_SECRET approval route; the console operator session + runner capability are the only approval surfaces"]
fn ts_bridge_approval_routes_fail_closed_without_bridge_secret() {}

/// TS `api-approvals.test.js:28` (the bridge-secret `/api/approvals` create +
/// `/verdict` + `/matrix` + `/consume` family). Native replaces the whole family
/// with the runner approval pair (`GET /api/native/v1/runner/approval`,
/// `POST .../approval/consume`) driven by the dispatch capability, asserted
/// green by `tests/runner.rs native_runner_approval_routes`.
#[test]
#[ignore = "parity gap: the bridge-secret /api/approvals family has no native route; the capability-scoped runner approval pair is its native successor"]
fn ts_bridge_owned_binding_and_one_shot_verdict_flow() {}

/// TS `approval-owner-can-see-it.test.js:82` — *THE DEFECT: an owner who is not
/// in the room is reported, with the remedy*. The retained words are native in
/// `hagency-matrix` (`identity_polish::owner_absent_warning`, exercised on the
/// provisioning path); the console has no owner-visibility surface, so this
/// case's native half is asserted in the matrix crate
/// (`ts_oracle_rooms.rs ts_owner_absent_warning_is_reported_with_remedy`).
#[test]
#[ignore = "parity gap: owner-visibility is reported on the matrix provisioning path; asserted in hagency-matrix tests/ts_oracle_rooms.rs"]
fn ts_owner_can_see_it_owner_absent_warning() {}

/// TS `approval-owner-can-see-it.test.js:352` — *IT NEVER BLOCKS THE DELIVERY IT
/// IS CHECKING*. Native's check is a warning on the wait handoff and the request
/// is left for the owner (never taken down) — asserted in
/// `hagency-matrix` where that path lives.
#[test]
#[ignore = "parity gap: the non-blocking owner check is on the matrix provisioning path; asserted in hagency-matrix tests/ts_oracle_rooms.rs"]
fn ts_owner_can_see_it_never_blocks_the_delivery() {}

/// TS `approval-owner-can-see-it.test.js` (the remaining 15 cases: side
/// credential reads, `known: false` silence, unreadable membership, no-bot
/// skip, publish-path warnings). All are the bridge's owner-membership probe
/// against a side credential — native does not perform a pre-delivery
/// membership probe at all (it warns AFTER the send on the provisioning path),
/// so the behaviour genuinely differs.
#[test]
#[ignore = "parity gap: native performs no pre-delivery owner-membership probe; it warns after the send (owner_absent_warning), so the probe's verdict vocabulary has no native counterpart"]
fn ts_owner_can_see_it_membership_probe_vocabulary() {}
