//! TS test oracle — the agent HTTP API (`tests/api-agents.test.js`,
//! `tests/api-agent-*.test.js`).
//!
//! The retained TS suite is the parity ORACLE: each test below asserts the
//! SAME observable outcome the TS case asserts (status, JSON fields, message
//! text). Where native differs, the test keeps asserting the TS outcome and is
//! `#[ignore]`d with a one-line reason — this task changes no product code.
//!
//! Native surface under test: the browser console facade
//! (`native/hagency/src/console/agents.rs`, `agent_detail.rs`).
/* `fixture.rs` reaches the store seed through
 * `super::real_agent::matrix_common::domain`. Load exactly that module: the
 * `#[path]` sits at THIS file's top level (so it resolves against `tests/`),
 * and `real_agent` is a bare re-export. Including `console/real_agent.rs`
 * wholesale would also compile ITS test functions into this binary, which are
 * not this file's oracle. */
#[path = "../../hagency-matrix/tests/common/mod.rs"]
pub mod matrix_common_real;
pub mod real_agent {
    pub use super::matrix_common_real as matrix_common;
}
#[path = "console/fixture.rs"]
mod fixture;

use fixture::*;
use salvo::{
    prelude::*,
    test::{ResponseExt, TestClient},
};
use serde_json::{Value, json};

/* ── the console session helpers, the same ones `tests/console.rs` uses ── */
async fn issue(service: &Service) -> String {
    let mut response = TestClient::post(format!("{BASE}/api/native/v1/console/access"))
        .add_header("host", "127.0.0.1:13300", true)
        .bearer_auth(TOKEN)
        .send(service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    response.take_json::<Value>().await.unwrap()["ticket"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn exchange(service: &Service, ticket: &str) -> Response {
    TestClient::post(format!("{BASE}/console/session"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .json(&json!({"ticket":ticket}))
        .send(service)
        .await
}
async fn session(service: &Service) -> String {
    let ticket = issue(service).await;
    let response = exchange(service, &ticket).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}
async fn lifecycle_issue(service: &Service) -> String {
    let mut response = TestClient::post(format!(
        "{BASE}/api/native/v1/console/agent-lifecycle-access"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .bearer_auth(TOKEN)
    .send(service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    response.take_json::<Value>().await.unwrap()["ticket"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn lifecycle_session(service: &Service) -> String {
    let ticket = lifecycle_issue(service).await;
    let response = exchange(service, &ticket).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}
fn get(path: &str, cookie: &str) -> salvo::test::RequestBuilder {
    TestClient::get(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", cookie, true)
}
fn post(path: &str, cookie: &str) -> salvo::test::RequestBuilder {
    TestClient::post(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", cookie, true)
}

/* ═══════════════════ PASSING: the outcome matches TS ═══════════════════ */

/// TS `tests/api-agents.test.js:161`:
///   `GET /api/agents/:name returns registered agent` —
///   `expect(response.body.name).toBe('charlie')`.
/// Native serves the same observable: 200 and a `name` identifying the agent.
/// (TS addresses the agent by name in the URL; native's detail carries the
/// same `name` field, so the TS caller's assertion holds.)
#[tokio::test]
async fn ts_oracle_agent_detail_returns_the_named_agent() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/agents/UsageWorker", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        value["name"], "UsageWorker",
        "the detail names the agent the caller asked for"
    );
    f.close().await;
}

/// TS `tests/api-agents.test.js:161` (the miss arm) and
/// `tests/api-agent-project-side-binding.test.js:102`
/// (`an unknown agent is a 404, not a binding stored against nothing`) —
/// the TS body is `{ error: 'agent not found' }` (backend-v2.js:12159).
/// Native keeps that exact human string and adds a machine code.
#[tokio::test]
async fn ts_oracle_unknown_agent_is_the_same_404_words() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/agents/Nobody", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        value["error"], "agent not found",
        "the TS 404 wording, byte for byte (backend-v2.js:12159)"
    );
    f.close().await;
}

/// TS `tests/api-agent-project-side-binding.test.js:166`
/// (`minting refuses before the binding …`) and the whole
/// `api-agent-stop` suite: every one of these routes is behind the operator
/// credential. TS refuses an anonymous stop with 401
/// (`api-agent-stop.test.js:123` `post('/api/agents/worker/stop').expect(401)`).
/// Native refuses an anonymous console read the same way.
#[tokio::test]
async fn ts_oracle_anonymous_agent_read_is_refused() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let anonymous = TestClient::get(format!("{BASE}/console/api/agents"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));
    f.close().await;
}

/* ═════════════ PARITY GAPS: native differs — asserting the TS outcome ═════════════ */

/// TS `tests/api-agents.test.js:148` `POST /api/agents registers an agent`:
///   status 200, `body.ok === true`, `body.agent.name === 'bravo'`.
/// Native has no agent-create route (`console/agents.rs` mounts no POST on the
/// roster).
#[tokio::test]
#[ignore = "parity gap: no native POST /api/agents (agent creation) route"]
async fn ts_oracle_post_agents_registers_an_agent() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = post("/console/api/agents", &cookie)
        .json(&json!({"name":"bravo","role":"coding","identity":"Build agent"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["agent"]["name"], "bravo");
    f.close().await;
}

/// TS `tests/api-agents.test.js:174` `PATCH /api/agents/:name updates agent
/// fields`: 200, `body.agent.role === 'review'`, `body.agent.identity ===
/// 'New identity'`. Native has no agent PATCH.
#[tokio::test]
#[ignore = "parity gap: no native PATCH /api/agents/:name route"]
async fn ts_oracle_patch_agent_updates_fields() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = TestClient::patch(format!("{BASE}/console/api/agents/UsageWorker"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .json(&json!({"role":"review","identity":"New identity"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["agent"]["role"], "review");
    f.close().await;
}

/// TS `tests/api-agents.test.js:212` `GET /api/agents?view=names returns
/// string array`: 200 and a JSON array where every element is a string.
/// TS also sorts them (`backend-v2.js:11704` `a.localeCompare(b)`) and drops
/// empty names (`:11703`), so the native arm copies both.
#[tokio::test]
async fn ts_oracle_roster_view_names_returns_strings() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/agents?view=names", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert!(value.is_array(), "TS answers a bare array of names");
    let names: Vec<&str> = value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().expect("every element is a string"))
        .collect();
    assert!(!names.is_empty(), "the fixture seeds agents to name");
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "TS sorts the names");
    assert!(names.iter().all(|n| !n.is_empty()), "TS drops empty names");
    // The names are the SAME agents the envelope carries — one source, two
    // shapes, never a second derivation.
    let mut envelope = get("/console/api/agents", &cookie).send(&service).await;
    let envelope = envelope.take_json::<Value>().await.unwrap();
    let mut from_envelope: Vec<&str> = envelope["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    from_envelope.sort();
    assert_eq!(names, from_envelope);
    f.close().await;
}

/// TS `tests/api-agents.test.js:49` `backend agents API` — the roster read is
/// the store's records in a bare JSON array (`res.json(serializeAgents(...))`,
/// backend-v2.js:11706). Native answers an object envelope
/// (`{at_ms, unavailable, agents, permissions}`).
#[tokio::test]
#[ignore = "parity gap: the roster is an object envelope, not the TS bare array"]
async fn ts_oracle_roster_is_a_bare_array() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/agents", &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert!(value.is_array(), "TS answers a bare array");
    f.close().await;
}

/// TS `tests/api-agents.test.js:227` `DELETE /api/agents/:name?force=true
/// cascades cleanup across runtime state files`: 200 with `body.ok === true`.
/// Native mounts no DELETE on the roster at all.
#[tokio::test]
#[ignore = "parity gap: no native DELETE /api/agents/:name route"]
async fn ts_oracle_force_delete_cascades() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response =
        TestClient::delete(format!("{BASE}/console/api/agents/UsageWorker?force=true"))
            .add_header("host", "127.0.0.1:13300", true)
            .add_header("origin", BASE, true)
            .add_header("sec-fetch-site", "same-origin", true)
            .add_header("cookie", &cookie, true)
            .send(&service)
            .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    f.close().await;
}

/// TS `tests/api-agents.test.js:270` `POST /api/agents/:name/start claims
/// starting state before launch registration` and
/// `tests/api-agent-stop.test.js:163` (`stop(name).then(…)` racing a real
/// launch): TS starts an agent and answers 200. Native refuses with 501.
#[tokio::test]
#[ignore = "parity gap: native start fails closed with 501 agent_start_unavailable"]
async fn ts_oracle_start_launches_an_agent() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = post("/console/api/agents/UsageWorker/start", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    f.close().await;
}

/// TS `tests/api-agent-preset-binding.test.js:72` `binds the preset and
/// reports the resulting ceiling and remaining`: `PUT` → 200
/// `{ok, agent, ceilingTokens, remaining, tier}`. Native answers
/// `agent_preset_unavailable` (501) and the route is POST, so TS's PUT is 405.
#[tokio::test]
#[ignore = "parity gap: native preset fails closed with 501; the route is POST, not TS's PUT"]
async fn ts_oracle_preset_binding_reports_ceiling() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = TestClient::put(format!("{BASE}/console/api/agents/UsageWorker/preset"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .json(&json!({"presetId":"codex-default-namespace-v1"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    assert!(value.get("ceilingTokens").is_some(), "TS names the ceiling");
    f.close().await;
}

/// TS `tests/api-agent-stop.test.js:132` `stop confirms managed session
/// disappearance and retains the agent record`:
///   `expect(response.body).toMatchObject({ stopped: true, sessionKilled: true })`.
/// Native's fence reports `stopped:false` + `stop_pending:true` — there is no
/// production path that settles a stop, so the TS headline never appears.
#[tokio::test]
#[ignore = "parity gap: native stop is a pending fence (stopped:false, stop_pending:true), never stopped:true"]
async fn ts_oracle_stop_confirms_termination() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = post(
        &format!("/console/api/agents/{}/stop", f.engagement),
        &cookie,
    )
    .json(&json!({}))
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        value["stopped"],
        json!(true),
        "the TS confirmation the operator reads"
    );
    f.close().await;
}

/// TS `tests/api-agent-project-side-binding.test.js:52` `an operator can bind,
/// and the record shows it`: `PUT /api/agents/:name/project-side` → 200 with
/// `body.agent.projectSide === SIDE`. Native mounts no agent-binding route.
#[tokio::test]
#[ignore = "parity gap: no native PUT /api/agents/:name/project-side route"]
async fn ts_oracle_agent_project_side_binding() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = TestClient::put(format!(
        "{BASE}/console/api/agents/UsageWorker/project-side"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .add_header("origin", BASE, true)
    .add_header("sec-fetch-site", "same-origin", true)
    .add_header("cookie", &cookie, true)
    .json(&json!({"projectSide":"example.test"}))
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["agent"]["projectSide"], "example.test");
    f.close().await;
}

/// TS `tests/api-agent-matrix-identity.test.js:124` `from projectSide in
/// preference to a binding`: `POST /api/agents/:name/matrix-identity` → 200
/// with `body.sideResolvedFrom === 'agent.projectSide'` and a minted mxid
/// `@ac_<agent>:<side>`. Native has no identity-minting route.
#[tokio::test]
#[ignore = "parity gap: no native POST /api/agents/:name/matrix-identity route"]
async fn ts_oracle_agent_matrix_identity_minting() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = post("/console/api/agents/UsageWorker/matrix-identity", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        value["sideResolvedFrom"], "agent.projectSide",
        "the response names which source won"
    );
    f.close().await;
}

/// TS `tests/api-agent-provision.test.js:55` `API provisioning writes the
/// selected project mapping for bootstrap`: `POST /api/agents/:name/provision`
/// → 201 with `body.paths.workdir`. Native has no provision route.
#[tokio::test]
#[ignore = "parity gap: no native POST /api/agents/:name/provision route"]
async fn ts_oracle_agent_provision_writes_project_mapping() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = post("/console/api/agents/ProjectAgent/provision", &cookie)
        .json(&json!({"framework":"codex","project":"codex-demo"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::CREATED));
    let value = response.take_json::<Value>().await.unwrap();
    assert!(
        value["paths"]["workdir"].is_string(),
        "TS names the workdir"
    );
    f.close().await;
}

/// TS `tests/api-agent-token.test.js:58` `health reports missing managed agent
/// tokens without flipping hard-mode compatibility`:
///   `expect(health.body.ok).toBe(true)` and
///   `expect(health.body.auth.agentTokens).toMatchObject({...})`.
/// Native `/health` is 200 and unauthenticated, but its body is the native
/// readiness document (`status`/`implementation`/`components`) — there is no
/// `auth.agentTokens` block, and native has no per-agent HTTP token surface
/// (the console rides a session cookie; the runner rides `RunnerCapability`).
#[tokio::test]
#[ignore = "parity gap: no native auth.agentTokens on /health and no per-agent HTTP token surface"]
async fn ts_oracle_health_reports_agent_tokens() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let mut response = TestClient::get(format!("{BASE}/health"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    assert!(
        value["auth"]["agentTokens"].is_object(),
        "TS reports the per-agent token state in auth.agentTokens"
    );
    f.close().await;
}

/// TS `tests/api-agent-token.test.js:43` `non-agent senders (system) pass
/// through in hard mode without a token`: `POST /api/messages` → 200. Native
/// has no message-send route on any surface it serves.
#[tokio::test]
#[ignore = "parity gap: no native POST /api/messages route (agent token auth has no native surface)"]
async fn ts_oracle_system_message_passes_without_a_token() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let response = TestClient::post(format!("{BASE}/console/api/messages"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .json(
            &json!({"from":"system","to":"alpha","type":"inform","summary":"sys","full":"sys msg"}),
        )
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    f.close().await;
}
#[tokio::test]
#[ignore = "parity gap: no native POST /api/agents (agent creation) route"]
async fn ts_oracle_operator_can_attach_a_preset_at_registration() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = post("/console/api/agents", &cookie)
        .json(&json!({"name":"UsageWorker","presetId":"codex-default-namespace-v1"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        value["agent"]["presetId"], "codex-default-namespace-v1",
        "the operator's preset survives registration"
    );
    f.close().await;
}
