//! Console `verify` route (board #14): the one route that talks to the
//! homeserver. `verify` calls `/_matrix/client/v3/account/whoami` with the
//! stored `as_token` (masquerading `?user_id=` for an appservice side),
//! records the verdict and the representative, and promotes a staged
//! credential only after the homeserver proves it accepts it. The homeserver
//! is the matrix Fake — a real HTTP server on 127.0.0.1 — so the `Authorization`
//! header and the whoami target are asserted on the wire, never on a seam.
#[path = "../../../hagency-matrix/tests/common/mod.rs"]
pub mod matrix_common;
use super::*;
use matrix_common::Fake;

fn registration_json() -> Value {
    let fleet = format!("hf_{}", "c".repeat(32));
    json!({
        "fleetId": fleet,
        "generation": 1,
        "serverName": "example.test",
        "receptionRoomId": "!reception:example.test",
        "representativeMxid": format!("@{fleet}_representative:example.test"),
        "approvalBotMxid": "@approval:example.test",
    })
}

fn appservice() -> Value {
    json!({
        "kind": "appservice",
        "asToken": "as_token_verify_1",
        "hsToken": "hs_token_verify_1",
        "namespace": "_ac_.*",
        "senderLocalpart": "hagency",
    })
}

/// The acceptance-critical path: `verify` calls the homeserver whoami with the
/// stored `as_token`, records `accepted` + the representative, and reports
/// `promoted:false` (nothing was staged).
#[tokio::test]
async fn native_console_side_verify_calls_whoami_and_records_the_verdict() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;

    // Register the side (create route ensures the side record).
    let created = post("/console/api/project-sides", &cookie)
        .json(&registration_json())
        .send(&service)
        .await;
    assert_eq!(created.status_code, Some(StatusCode::OK));

    // Install the credential and its API base URL pointing at the fake homeserver.
    let mut fake = Fake::start(false).await;
    let endpoint = fake.endpoint.trim_end_matches('/').to_string();
    let installed = TestClient::put(format!(
        "{BASE}/console/api/project-sides/example.test/credential"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .add_header("origin", BASE, true)
    .add_header("sec-fetch-site", "same-origin", true)
    .add_header("cookie", &cookie, true)
    .json(&json!({"credential": appservice(), "apiBaseUrl": endpoint}))
    .send(&service)
    .await;
    assert_eq!(installed.status_code, Some(StatusCode::OK));

    // Drive verify; the homeserver answers whoami.
    let scripted = async {
        let request = fake.next().await;
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.target,
            "/_matrix/client/v3/account/whoami?user_id=@hagency:example.test"
        );
        assert_eq!(
            request.headers["authorization"], "Bearer as_token_verify_1",
            "verify sends the stored as_token"
        );
        request.json(200, json!({"user_id": "@hagency:example.test"}));
    };
    let verify = async {
        post("/console/api/project-sides/example.test/verify", &cookie)
            .send(&service)
            .await
    };
    let (mut response, ()) = tokio::join!(verify, scripted);

    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["promoted"], json!(false));
    assert_eq!(body["side"]["accessState"], "accepted");
    assert_eq!(
        body["side"]["representative"]["mxid"],
        "@hagency:example.test"
    );
    f.close().await;
}

/// A staged credential is tried FIRST and promoted only after the homeserver
/// proves it accepts it; the old credential keeps working until then. Staging
/// is seeded through the store (no console route stages), then `verify` is the
/// act that promotes.
#[tokio::test]
async fn native_console_side_verify_promotes_a_staged_credential() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;

    let created = post("/console/api/project-sides", &cookie)
        .json(&registration_json())
        .send(&service)
        .await;
    assert_eq!(created.status_code, Some(StatusCode::OK));

    let mut fake = Fake::start(false).await;
    let endpoint = fake.endpoint.trim_end_matches('/').to_string();

    // A live credential, accepted (proven by a whoami we script now).
    let installed = TestClient::put(format!(
        "{BASE}/console/api/project-sides/example.test/credential"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .add_header("origin", BASE, true)
    .add_header("sec-fetch-site", "same-origin", true)
    .add_header("cookie", &cookie, true)
    .json(&json!({"credential": appservice(), "apiBaseUrl": endpoint}))
    .send(&service)
    .await;
    assert_eq!(installed.status_code, Some(StatusCode::OK));

    // Stage a replacement directly through the store: the old credential keeps
    // working, the new one waits.
    let staged = json!({
        "kind": "appservice",
        "asToken": "as_token_verify_staged",
        "hsToken": "hs_token_verify_staged",
        "namespace": "_ac_.*",
        "senderLocalpart": "hagency",
    });
    f.domain
        .set_credential("example.test".into(), Some(staged), true)
        .await
        .unwrap()
        .expect("side exists");

    // verify tries the STAGED credential first; the homeserver accepts it.
    let scripted = async {
        let request = fake.next().await;
        assert_eq!(
            request.headers["authorization"], "Bearer as_token_verify_staged",
            "verify tries the staged credential first"
        );
        request.json(200, json!({"user_id": "@hagency:example.test"}));
    };
    let verify = async {
        post("/console/api/project-sides/example.test/verify", &cookie)
            .send(&service)
            .await
    };
    let (mut response, ()) = tokio::join!(verify, scripted);

    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["ok"], json!(true));
    assert_eq!(
        body["promoted"],
        json!(true),
        "a proven staged credential is promoted"
    );
    // The live credential is now the staged one.
    let live = f
        .domain
        .credential_for("example.test".into())
        .await
        .unwrap()
        .expect("live credential");
    assert_eq!(live.as_token.as_deref(), Some("as_token_verify_staged"));
    f.close().await;
}
