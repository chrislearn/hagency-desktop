use super::*;

/// Seed one LIVE approval binding behind the store's real schema, the same
/// discipline the approvals test applies: the console fixture does not
/// stage bindings, so the rows go in through SQL that satisfies the
/// `current_approval_bindings` view (room available + snapshot safe +
/// engagement active + registration/project rows agreeing).
fn seed_binding(state: &std::path::Path, engagement: &str) {
    let mut db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    let room_config = serde_json::json!({
        "joined": ["@owner:example.test", "@approval:example.test"],
        "invite_only": true,
        "encrypted": true,
        "available": true,
    })
    .to_string();
    let tx = db.transaction().unwrap();
    // The room facts must match the seeded project row (owner mxid + owner
    // DM room) and the fleet registration (server name + approval bot) the
    // console fixture already wrote.
    tx.execute(
        "INSERT INTO approval_rooms(server_name,room_id,generation,fleet_id,project_id,registration_generation,owner_mxid,bot_mxid,device_id,available,digest,config) \
         VALUES('example.test','!private:example.test',1,(SELECT fleet_id FROM registrations), \
         (SELECT project_id FROM engagements WHERE id=?1),1,'@owner:example.test','@approval:example.test','DEVICE_PRIVATE',1,'digest_private',?2)",
        rusqlite::params![engagement, room_config],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO approval_bindings(engagement_id,server_name,room_id,room_generation,incarnation) \
         VALUES(?1,'example.test','!private:example.test',1,1)",
        [engagement],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// The plain-list branch of TS `GET /api/approval-bindings`
/// (backend-v2.js:9078-9082): every live binding as a flat object, the two
/// TS filters honoured, any other query parameter refused.
#[tokio::test]
async fn native_console_approval_bindings_list() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    seed_binding(&f.root.path().join("state"), &f.engagement);
    let service = f.service();
    let cookie = session(&service).await;
    // The unfiltered list: exactly the nine declared keys, the seeded row.
    let mut response = get("/console/api/approval-bindings", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    let bindings = value["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    let mut keys: Vec<&str> = binding
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "agent",
            "engagementId",
            "fleetId",
            "incarnation",
            "ownerMxid",
            "projectId",
            "roomGeneration",
            "roomId",
            "serverName"
        ]
    );
    assert_eq!(binding["agent"], "UsageWorker");
    assert_eq!(binding["roomId"], "!private:example.test");
    assert_eq!(binding["ownerMxid"], "@owner:example.test");
    assert_eq!(binding["roomGeneration"], 1);
    assert_eq!(binding["incarnation"], 1);
    // The agent filter matches; a foreign agent yields the empty list, not
    // an error (TS `listBindings` filters, it never 404s).
    let mut response = get("/console/api/approval-bindings?agent=UsageWorker", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["bindings"].as_array().unwrap().len(), 1);
    let mut response = get("/console/api/approval-bindings?agent=Nobody", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["bindings"].as_array().unwrap().len(), 0);
    // The `project` filter carries a ROOM id (the TS route's slot), so the
    // seeded room matches and another room does not. The `!` is sent raw —
    // the console query hygiene refuses `%`-encoded query strings (usage.rs),
    // and `matrix_room` wants the literal room id.
    let mut response = get(
        "/console/api/approval-bindings?project=!private:example.test",
        &cookie,
    )
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["bindings"].as_array().unwrap().len(), 1);
    let mut response = get(
        "/console/api/approval-bindings?project=!other:example.test",
        &cookie,
    )
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["bindings"].as_array().unwrap().len(), 0);
    // Any query parameter outside the allowlist is refused.
    for path in [
        "/console/api/approval-bindings?includeInactive=true",
        "/console/api/approval-bindings?projectRoomId=!private:example.test",
    ] {
        let response = get(path, &cookie).send(&service).await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::BAD_REQUEST),
            "{path}"
        );
    }
    f.close().await;
}

/// An unseeded store serves the empty list, not an error: the route observes
/// what is bound, and nothing being bound is a valid observation.
#[tokio::test]
async fn native_console_approval_bindings_empty() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/approval-bindings", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["bindings"].as_array().unwrap().len(), 0);
    f.close().await;
}

/// Seed one live grant under the seeded binding's engagement, the same SQL
/// discipline the approvals test uses for its revocation case: the grant is
/// the authority the TS unbind revokes (`revokeScopesByBinding`), so the test
/// can assert the durable effect next to the row removal.
fn seed_grant(state: &std::path::Path, engagement: &str) {
    let db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO approval_grants(id,engagement_id,binding_generation,scope_key,scope_kind,mode,task_id,task_epoch,context_key,revoked) \
         VALUES('grant_binding_unbind',?1,1,'task:echo','\"exact_command\"','always',NULL,NULL,'ctx_unbind',0)",
        [engagement],
    )
    .unwrap();
}

/// The live-grant predicate the store's `authorize` matches on: `revoked=0`
/// is the row's only authority, so revocation must remove it from this set.
fn grant_authorizes(state: &std::path::Path, engagement: &str) -> bool {
    let db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM approval_grants WHERE engagement_id=?1 AND binding_generation=1 AND scope_key='task:echo' AND context_key='ctx_unbind' AND revoked=0 AND mode='always')",
        [engagement],
        |r| r.get::<_, bool>(0),
    )
    .unwrap()
}

fn binding_count(state: &std::path::Path) -> u64 {
    let db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    db.query_row("SELECT COUNT(*) FROM approval_bindings", [], |r| r.get(0))
        .unwrap()
}

/// The operator unbind (board #52, TS `DELETE
/// /api/approval-bindings/:agent/:roomId` at backend-v2.js:9032-9046): the
/// binding row goes AND the authority it carried is revoked — the TS
/// `removeBinding` + `revokeScopesByBinding` pair. integ is SINGLE-LOGIN
/// (operator decision): the `authenticate` hoop is the whole gate, so an
/// ANONYMOUS caller is refused 401 and an ordinary authenticated session may
/// unbind; an unknown pair is a 404.
#[tokio::test]
async fn native_console_approval_binding_unbind_revokes() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let state = f.root.path().join("state");
    seed_binding(&state, &f.engagement);
    seed_grant(&state, &f.engagement);
    let service = f.service();
    let path = "/console/api/approval-bindings/UsageWorker/!private%3Aexample.test";
    // No cookie: the `authenticate` hoop refuses before any store work and
    // nothing changed.
    let mut anonymous = TestClient::delete(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));
    let refusal: Value = anonymous.take_json().await.unwrap();
    assert_eq!(refusal["code"], "console_access_required");
    assert_eq!(binding_count(&state), 1, "the refusal removed nothing");
    assert!(
        grant_authorizes(&state, &f.engagement),
        "the refusal revoked nothing"
    );
    // SINGLE-LOGIN: an ORDINARY authenticated session unbinds — no scope, no
    // capability word. The list carries no `permissions` envelope at all.
    let cookie = session(&service).await;
    let mut listed = get("/console/api/approval-bindings", &cookie)
        .send(&service)
        .await;
    assert_eq!(listed.status_code, Some(StatusCode::OK));
    let value: Value = listed.take_json().await.unwrap();
    assert!(
        value.as_object().unwrap().get("permissions").is_none(),
        "single-login serves no capability word"
    );
    // An unknown pair is a 404, as TS returns.
    let missing = TestClient::delete(format!(
        "{BASE}/console/api/approval-bindings/UsageWorker/!nope%3Aexample.test"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .add_header("origin", BASE, true)
    .add_header("sec-fetch-site", "same-origin", true)
    .add_header("cookie", &cookie, true)
    .send(&service)
    .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));
    let mut response = TestClient::delete(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["ok"], true);
    // The returned binding is the nine-key projection the list serves.
    let binding = &value["binding"];
    let mut keys: Vec<&str> = binding
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "agent",
            "engagementId",
            "fleetId",
            "incarnation",
            "ownerMxid",
            "projectId",
            "roomGeneration",
            "roomId",
            "serverName"
        ]
    );
    assert_eq!(binding["agent"], "UsageWorker");
    assert_eq!(binding["roomId"], "!private:example.test");
    // The binding is gone from the list and from the table — TS is a hard
    // delete — and the grant it carried no longer authorizes.
    assert_eq!(binding_count(&state), 0, "the binding row was removed");
    assert!(
        !grant_authorizes(&state, &f.engagement),
        "the grant the binding carried no longer authorizes"
    );
    let mut listed = get("/console/api/approval-bindings", &cookie)
        .send(&service)
        .await;
    assert_eq!(listed.status_code, Some(StatusCode::OK));
    let value: Value = listed.take_json().await.unwrap();
    assert_eq!(value["bindings"].as_array().unwrap().len(), 0);
    f.close().await;
}

/// The agent self-update surface (board #52, TS `PATCH /api/agents/:name` at
/// backend-v2.js:11530-11560): native has no writable agent record, so the
/// route is the SAFETY half only. The load-bearing refusal (TS :11537-11550):
/// `projectSide` in the body is refused, never silently dropped; every other
/// field is refused because there is nothing to apply it to.
fn runner_patch(
    path: &str,
    cap: &hagency_core::tasks::RunnerCapability,
) -> salvo::test::RequestBuilder {
    TestClient::patch(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("authorization", format!("Bearer {}", cap.secret), true)
        .add_header("x-hagency-dispatch", &cap.dispatch_id, true)
        .add_header("x-hagency-runner", &cap.runner_id, true)
        .add_header("x-hagency-fence", cap.fence.to_string(), true)
        .json(&json!({}))
}

#[tokio::test]
async fn native_runner_agent_self_update_refusals() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let path = "/api/native/v1/runner/agent";
    // A capability the runner hoop accepts must be claimed against a dispatch
    // whose lease is alive at REAL wall-clock time — the fixture seed claims
    // at synthetic t=1002ms, long expired now (authorize_attempt,
    // execution.rs:199-214). Enqueue and claim a fresh one through the store.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // A fresh session, never the fixture's: the candidates predicate gates on
    // task intents attached to the SESSION (execution.rs candidates SQL), and
    // `private_session` carries the retired intents of the seeded flow.
    f.domain
        .register_session(hagency_core::tasks::SessionBinding {
            id: "self_update_session".into(),
            engagement_id: f.engagement.clone(),
            room_id: "!self_update_room:example.test".into(),
            thread_root: None,
        })
        .await
        .expect("register self_update_session must succeed");
    f.domain
        .enqueue_dispatch(hagency_core::tasks::DispatchInput {
            id: "self_update_dispatch".into(),
            session_id: "self_update_session".into(),
            // No task: the candidates predicate also gates on task intents
            // and follow-ups; this dispatch exists only to hold the lease.
            task_id: None,
            resources: vec![],
            payload: json!({}),
        })
        .await
        .expect("enqueue self_update_dispatch must succeed");
    let cap = f
        .domain
        .claim_dispatch("self_update_runner".into(), now_ms, 60000, 120000, 128)
        .await
        .expect("claim command must not error")
        .expect("a queued dispatch must be claimable");
    // The runner hoop's Check requires a STARTED dispatch (check_runner,
    // execution.rs:762: authorize over ["started"]) — a leased lease is not
    // enough. Start it with the store's own start path: the owned-scope pair
    // is an engagement/resource authority gate this synthetic runner has no
    // credentials for, while start_dispatch is the claim-time continuation
    // every dispatch takes.
    f.domain
        .start_dispatch(cap.clone(), now_ms + 1)
        .await
        .expect("start self_update_dispatch must succeed");
    // No credential: the runner authenticate hoop refuses before the route.
    let response = TestClient::patch(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .json(&json!({"projectSide": "matrix.example.test"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
    // The load-bearing refusal: `projectSide` must not be settable from the
    // agent's own request (TS backend-v2.js:11537-11550 — an agent choosing
    // which customer it serves is an agent choosing its own employer). Both
    // spellings are refused.
    for key in ["projectSide", "project_side"] {
        let mut response = runner_patch(path, &cap)
            .json(&json!({key: "matrix.example.test"}))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST), "{key}");
        let body = response.take_string().await.unwrap();
        assert!(
            body.contains("project_side_not_settable_here"),
            "{key}: {body}"
        );
        // Lesson (first LIVE run): a refusal must say WHAT was refused and WHY,
        // never fall through to the generic word. The reason names the field
        // and the offending route class, and the remedy points at the operator
        // route (TS backend-v2.js:11550 "Naming the operator route in the
        // refusal is the point").
        assert!(
            body.contains("agent-authenticated"),
            "the refusal must say why: {key}: {body}"
        );
        assert!(
            body.contains("/project-side"),
            "the refusal must name the remedy route: {key}: {body}"
        );
        assert!(
            !body.contains("the request was refused"),
            "the generic fallback is not a recorded reason: {key}: {body}"
        );
    }
    // Any other field: native has no agent record to write, so the route
    // fails closed rather than reporting a success that applied nothing.
    let mut response = runner_patch(path, &cap)
        .json(&json!({"role": "coding"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    let body = response.take_string().await.unwrap();
    assert!(body.contains("agent_record_not_writable"), "{body}");
    // The reason must be a recorded one, not the generic fallback.
    assert!(
        !body.contains("the request was refused"),
        "the generic fallback is not a recorded reason: {body}"
    );
    f.close().await;
}
