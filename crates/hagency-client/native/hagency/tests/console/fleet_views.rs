use super::*;

/// LESSONS 2026-09-25 §"test the production path, not only the wire shape".
///
/// Task #46's three shape tests (`tests/http.rs`) read the views through the
/// BEARER mount `/api/native/v1/...`, which is the runner/operator surface.
/// The operator reaches the SAME views through the session-scoped console
/// mount — `console.rs` `.push(crate::fleet_views::router())` under
/// `/console/api` — and that is the path a live console actually walks. This
/// pins the console mount itself: a console session is required, and both
/// reads answer their TS shape on that path (five adapters in registry order;
/// the capability view is the role array). The host probe
/// (`frameworks/detect`) is deliberately not exercised here — it spawns
/// bounded child processes and is covered on the bearer mount.
#[tokio::test]
async fn native_console_fleet_views_read() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let anonymous = TestClient::get(format!("{BASE}/console/api/frameworks"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));

    let cookie = session(&service).await;

    let mut response = get("/console/api/frameworks", &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    let rows = value.as_array().unwrap();
    assert_eq!(rows.len(), 5);
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["claude", "codex-acp", "codex", "hermes", "octos"]);
    // The same projection the bearer test pins: the serializer's keys and the
    // flattened flag guard, so the two mounts cannot drift apart.
    assert!(rows[0].get("refusedFlags").is_some());
    assert!(rows[0].get("guardMessage").is_some());

    let mut capability = get("/console/api/capability", &cookie).send(&service).await;
    assert_eq!(capability.status_code, Some(StatusCode::OK));
    let view: Value = capability.take_json().await.unwrap();
    // The envelope the bearer test pins: the role array plus its provenance.
    assert_eq!(view["source"], "native/hagency-core/role-capacity.json");
    assert_eq!(view["roles"].as_array().unwrap().len(), 6);

    f.close().await;
}
