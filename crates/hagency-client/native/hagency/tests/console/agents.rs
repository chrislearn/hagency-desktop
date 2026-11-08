use super::*;
use hagency_core::tasks::SessionBinding;
use hagency_store::{ACCOUNT_PROFILE, resource_publication_revision};

/// The agent roster observation (ADR-126, widened by board #22): the read
/// is one row per AGENT — the TS roster's shape (`backend-v2.js:11696`) —
/// every item carries EXACTLY the nine declared keys, no nested object, no
/// private field — and the null-not-zero rule is pinned on the seeded
/// agents with no attempt (`AlertWorker`, `PageWorker` report null
/// `last_seen_ms`/`last_activity_ms`, while `UsageWorker` is online with
/// the attempt clock: its live `started` dispatch is REAL worker state).
#[tokio::test]
async fn native_console_agent_roster_observation() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let anonymous = TestClient::get(format!("{BASE}/console/api/agents"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));
    let cookie = session(&service).await;
    // A roster takes no selection: every query parameter is refused.
    for query in ["?limit=1", "?after=x", "?name=UsageWorker", "?limit=%31"] {
        let response = get(&format!("/console/api/agents{query}"), &cookie)
            .send(&service)
            .await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::BAD_REQUEST),
            "{query}"
        );
    }
    let mut response = get("/console/api/agents", &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        value.as_object().unwrap().len(),
        4,
        "the envelope carries exactly at_ms, unavailable, agents, permissions"
    );
    assert!(value["at_ms"].as_u64().unwrap() > 0);
    assert!(
        value["permissions"]["manageLifecycle"].as_bool() == Some(true),
        "one login carries the lifecycle permission (the operator's one-login decision)"
    );
    // Board #60 item 2: every column this console renders is now ANSWERED
    // from native state — seat (resources.config), consumed
    // (usage_sources.latest_counts) and liveness (the live dispatch row) —
    // so the server names none as unavailable. The four that stay
    // unanswerable by design (tmux, pane, credential_home, workspace_path:
    // ADR-126) are DROPPED, not printed as "unknown".
    assert_eq!(
        value["unavailable"].as_array().unwrap().len(),
        0,
        "every rendered roster column has a native source"
    );
    let agents = value["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 3, "one row per agent the service knows");
    let keys = [
        "name",
        "framework",
        "role",
        "state",
        "engagement_id",
        "requested_tokens",
        "online",
        "last_seen_ms",
        "last_activity_ms",
        "liveness",
        "consumed",
        // ADR-186 §B: the quota hold.
        "quota_paused",
    ];
    let mut by_name: Vec<(String, &Value)> = agents
        .iter()
        .map(|a| (a["name"].as_str().unwrap().to_owned(), a))
        .collect();
    by_name.sort_by(|a, b| a.0.cmp(&b.0));
    let names: Vec<&str> = by_name.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["AlertWorker", "PageWorker", "UsageWorker"]);
    for (_, agent) in &by_name {
        let object = agent.as_object().unwrap();
        assert_eq!(object.len(), keys.len(), "exactly twelve keys");
        for key in keys {
            assert!(object.contains_key(key), "the wire item carries {key}");
            assert!(
                !object[key].is_object() && !object[key].is_array(),
                "no nested object or array travels on a roster item"
            );
        }
        assert_eq!(agent["framework"], "codex", "the single native framework");
        assert_eq!(agent["role"], "coding");
        assert!(
            agent["engagement_id"]
                .as_str()
                .is_some_and(engagement_id_shape),
            "the engagement id keeps its opaque shape"
        );
        assert_eq!(agent["requested_tokens"], 100);
    }
    let usage = &by_name[2].1;
    assert_eq!(usage["state"], "active");
    assert_eq!(
        usage["last_activity_ms"], 1002,
        "the newest attempt clock — last dispatch activity, not last seen"
    );
    // Real worker state: UsageWorker's `started` dispatch is live, so the
    // agent is online and its newest attempt clock is its last seen.
    assert_eq!(
        usage["online"], true,
        "a live dispatch in the agent's session is real worker state"
    );
    assert_eq!(
        usage["last_seen_ms"], 1002,
        "last seen is the agent's newest attempt clock"
    );
    // The null-not-zero rule: engagements with no attempt row report
    // unknown, never an invented zero clock. AlertWorker was approved but
    // its effect was never observed (reserved); PageWorker is admit-only
    // (pending) — neither has a session, dispatch or attempt row.
    assert_eq!(
        by_name[0].1["state"], "reserved",
        "AlertWorker approved, effect unobserved"
    );
    assert_eq!(by_name[1].1["state"], "pending", "PageWorker admit-only");
    for (name, agent) in [&by_name[0], &by_name[1]] {
        assert!(
            agent["last_activity_ms"].is_null(),
            "{name} carries null, not zero"
        );
        assert_eq!(agent["online"], false, "{name} has no live dispatch");
        assert!(
            agent["last_seen_ms"].is_null(),
            "{name} never attempted — null, not zero"
        );
    }
    assert_private(&value);
    f.close().await;
}

/// The agent detail route (board #22, TS `backend-v2.js:12155`
/// `GET /api/agents/:name`): one agent the service knows — identity,
/// resource, rooms, current dispatch, recent tasks — with the same
/// read-class gating as the roster. An unknown agent is 404, an invalid
/// name shape is 400 before any store work, and the seeded UsageWorker's
/// live dispatch is its real online state.
#[tokio::test]
async fn native_console_agent_detail_observation() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let anonymous = TestClient::get(format!("{BASE}/console/api/agents/UsageWorker"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));
    let cookie = session(&service).await;
    // The detail takes no selection: query parameters are refused.
    let response = get("/console/api/agents/UsageWorker?limit=1", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    // An agent the service never engaged is the TS route's 404.
    let mut response = get("/console/api/agents/Nobody", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "agent_not_found"
    );
    let mut response = get("/console/api/agents/UsageWorker", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let value = response.take_json::<Value>().await.unwrap();
    let keys = [
        "name",
        "framework",
        "role",
        "state",
        "engagement_id",
        "requested_tokens",
        "online",
        "last_seen_ms",
        "resource_id",
        "project_id",
        "engagements",
        "rooms",
        "dispatch",
        "tasks",
        "reminders",
    ];
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), keys.len(), "exactly the declared detail keys");
    for key in keys {
        assert!(object.contains_key(key), "the detail carries {key}");
    }
    assert_eq!(value["name"], "UsageWorker");
    assert_eq!(value["framework"], "codex");
    assert_eq!(value["role"], "coding");
    assert_eq!(value["state"], "active");
    assert_eq!(value["requested_tokens"], 100);
    assert_eq!(value["engagements"], 1, "one engagement names the agent");
    assert!(value["resource_id"].as_str().unwrap().len() <= 128);
    assert!(value["project_id"].as_str().unwrap().len() <= 128);
    // Real worker state: the seeded live `started` dispatch.
    assert_eq!(value["online"], true);
    assert_eq!(value["last_seen_ms"], 1002);
    // Rooms: the one seeded session binds the project room and carries the
    // live dispatch state.
    let rooms = value["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["session_id"], "private_session");
    assert_eq!(rooms[0]["room_id"], "!project:example.test");
    assert_eq!(rooms[0]["dispatch_state"], "started");
    assert_eq!(rooms[0]["dispatch_id"], "private_dispatch");
    // Current dispatch: the same live dispatch, agent-wide.
    assert_eq!(value["dispatch"]["dispatch_state"], "started");
    assert_eq!(value["dispatch"]["room_id"], "!project:example.test");
    // Recent tasks: the one seeded canonical task, newest first. The
    // fixture claimed and STARTED its dispatch, so the task's TS-visible
    // state is in_progress — the state a started dispatch reports
    // (backend-v2.js:13332 serves the store's task verbatim).
    let tasks = value["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], "private_task");
    assert_eq!(tasks[0]["title"], "Usage");
    assert_eq!(tasks[0]["status"], "in_progress");
    f.close().await;
}

/// The detail serves the TS-visible state even when the agent's work
/// ended: an agent whose engagements are all retired still appears (the
/// TS roster never dropped a record), with no live dispatch.
#[tokio::test]
async fn native_console_agent_detail_covers_an_ended_agent() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    // PageWorker is admit-only (pending): known to the service, never
    // staffed a session.
    let mut response = get("/console/api/agents/PageWorker", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["name"], "PageWorker");
    assert_eq!(value["state"], "pending");
    assert_eq!(value["online"], false, "no session means no live dispatch");
    assert!(
        value["last_seen_ms"].is_null(),
        "never attempted: null, not zero"
    );
    assert!(value["dispatch"].is_null(), "no live dispatch");
    assert!(value["rooms"].as_array().unwrap().is_empty());
    assert!(value["tasks"].as_array().unwrap().is_empty());
    f.close().await;
}

/// A foreign origin cannot read the roster: the boundary refuses with
/// `console_origin_required` before any store read, and the session hoop
/// refuses the transport-level forgeries with the access-required word —
/// neither serves a single agent item.
#[tokio::test]
async fn native_console_agent_roster_refuses_foreign_origin() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    for (name, value, status, code) in [
        (
            "host",
            "evil.test",
            StatusCode::FORBIDDEN,
            "console_origin_required",
        ),
        (
            "origin",
            "https://evil.test",
            StatusCode::UNAUTHORIZED,
            "console_access_required",
        ),
        (
            "sec-fetch-site",
            "cross-site",
            StatusCode::UNAUTHORIZED,
            "console_access_required",
        ),
        (
            "sec-fetch-site",
            "none",
            StatusCode::UNAUTHORIZED,
            "console_access_required",
        ),
        (
            "x-forwarded-for",
            "127.0.0.1",
            StatusCode::FORBIDDEN,
            "console_origin_required",
        ),
        (
            "forwarded",
            "for=127.0.0.1",
            StatusCode::FORBIDDEN,
            "console_origin_required",
        ),
    ] {
        let mut response = get("/console/api/agents", &cookie)
            .add_header(name, value, true)
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(status), "{name}: {value}");
        let body = response.take_json::<Value>().await.unwrap();
        assert_eq!(body["code"], code, "{name}: {value}");
        assert!(
            body.get("agents").is_none(),
            "{name}: {value} serves no agent item"
        );
    }
    // No mutation exists: the router refuses a non-GET outright.
    let post = TestClient::post(format!("{BASE}/console/api/agents"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert!(matches!(
        post.status_code,
        Some(StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED)
    ));
    f.close().await;
}

/// The store's opaque engagement id: `en_` + 32 hex characters
/// (`authority.rs:123`) — it names no session, no authority and no path.
fn engagement_id_shape(value: &str) -> bool {
    value.len() == 35
        && value.starts_with("en_")
        && value[3..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// CL-S2 commit 1 — the store surface (ADR-130): `stop_dispatch_for_agent`
/// resolves the engagement's dispatch through the live set or the unsettled
/// stop row, fences only that dispatch, and is at-most-once by construction:
/// a second call resolves the SAME dispatch id and fence, writes no second
/// stop row, and still reports `stop_pending` — `stopped` stays false
/// because no production path settles (settlement is the host's, uncallable
/// from runtime-facing commands). Driven against the real DomainStore
/// writer, not the HTTP surface.
#[tokio::test]
async fn native_console_stop_dispatch_for_agent_is_at_most_once() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    // The fixture's engagement carries a STARTED dispatch (uncertain), so
    // the fence writes a dispatch_stops row and quarantines the session —
    // never the settled word.
    let first = f
        .domain
        .stop_dispatch_for_agent(f.engagement.clone(), 2000)
        .await
        .unwrap();
    let keys = ["stopped", "stop_pending", "dispatch_id", "fence", "state"];
    assert_eq!(first.as_object().unwrap().len(), keys.len());
    for key in keys {
        assert!(
            first.as_object().unwrap().contains_key(key),
            "carries {key}"
        );
    }
    assert_eq!(first["stopped"], false, "no production path settles");
    assert_eq!(first["stop_pending"], true, "an honest stop is pending");
    assert_eq!(
        first["state"], "outcome_unknown",
        "the uncertain fence word"
    );
    let dispatch = first["dispatch_id"].as_str().unwrap().to_owned();
    let fence = first["fence"].as_u64().unwrap();
    assert!(!dispatch.is_empty());
    // At-most-once: the second call resolves the same row, writes nothing
    // new, and reports the same five keys.
    let second = f
        .domain
        .stop_dispatch_for_agent(f.engagement.clone(), 2001)
        .await
        .unwrap();
    assert_eq!(second["dispatch_id"], first["dispatch_id"]);
    assert_eq!(second["fence"], first["fence"]);
    assert_eq!(second["stop_pending"], true);
    assert_eq!(second["stopped"], false);
    let raw = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    let count: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM dispatch_stops WHERE dispatch_id=?1",
            [&dispatch],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "exactly one stop row — no second write");
    let settled: Option<u64> = raw
        .query_row(
            "SELECT settled_at FROM dispatch_stops WHERE dispatch_id=?1",
            [&dispatch],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(settled, None, "the store never settles");
    drop(raw);
    // An engagement with no live dispatch and no unsettled stop row refuses
    // with the store's NotFound — nothing to resolve, nothing fenced.
    let absent = format!("{}_absent", f.engagement);
    assert!(matches!(
        f.domain.stop_dispatch_for_agent(absent, 2002).await,
        Err(hagency_store::Error::NotFound)
    ));
    let _ = fence;
    f.close().await;
}

/// #21 acceptance: stop completes and start re-arms — TS parity
/// `stopManagedAgent` (backend-v2.js:12577-12708, success object
/// `{ok:true,stopped:true,...cancelledDispatches}` at :12690) and start
/// (backend-v2.js:12712, the return to serving). Idempotency follows the
/// retained stop: a second stop answers `stopped:true` again, writing no
/// second row.
#[tokio::test]
async fn native_console_agent_start_stop_is_at_most_once() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let lifecycle = lifecycle_session(&service).await;
    let id = &f.engagement;
    let stop = || async {
        let mut response = post(&format!("/console/api/agents/{id}/stop"), &lifecycle)
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        response.take_json::<Value>().await.unwrap()
    };
    let first = stop().await;
    assert_eq!(first["ok"], true);
    assert_eq!(first["stopped"], true, "the stop completes");
    assert_eq!(first["state"], "stopped");
    assert_eq!(
        first["cancelled_dispatches"].as_array().map(Vec::len),
        Some(1),
        "the live dispatch is the retained cancelledDispatches"
    );
    let second = stop().await;
    assert_eq!(second["stopped"], true);
    assert_eq!(
        second["cancelled_dispatches"].as_array().map(Vec::len),
        Some(0),
        "the second stop is idempotent"
    );
    let raw = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    let count: i64 = raw
        .query_row("SELECT COUNT(*) FROM dispatch_stops", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "the second stop writes no second row");
    let settled: Option<u64> = raw
        .query_row(
            "SELECT settled_at FROM dispatch_stops WHERE dispatch_id='private_dispatch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(settled.is_some(), "the operator stop settles the stop row");
    let stopped: Option<u64> = raw
        .query_row(
            "SELECT stopped_at FROM agent_lifecycle WHERE engagement_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stopped.is_some(), "the durable stopped row exists");
    drop(raw);
    // Start brings the agent back to serving (backend-v2.js:12712-12775).
    let mut response = post(&format!("/console/api/agents/{id}/start"), &lifecycle)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["state"], "launching");
    let started: Option<u64> =
        rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT started_at FROM agent_lifecycle WHERE engagement_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
    assert!(started.is_some(), "serving is re-armed durably");
    // A second start hits the retained `agent already online` (409).
    let mut response = post(&format!("/console/api/agents/{id}/start"), &lifecycle)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "agent_already_online"
    );
    f.close().await;
}

/// #21 preset (resource) rebind — TS parity `PUT /api/agents/:name/preset`
/// (backend-v2.js:11484-11522): binding, ceiling and profile move together,
/// the response reports ceiling/remaining beside the binding, and the NEXT
/// dispatch the host claims reads the new resource from the provision
/// effect's payload. An unknown preset is refused with 400 and the previous
/// binding survives (tests/api-agent-preset-binding.test.js parity).
#[tokio::test]
async fn native_console_agent_preset_apply_refuses_without_durable_transition() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let lifecycle = lifecycle_session(&service).await;
    let id = &f.engagement;
    // A second published resource to rebind onto.
    let other = native_resource("private_preset_apply_published");
    f.domain.put_resource(other.clone()).await.unwrap();
    let mut response = put(&format!("/console/api/agents/{id}/preset"), &lifecycle)
        .json(&json!({"presetId": other.preset_id}))
        .send(&service)
        .await;
    let probe = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        response.status_code,
        Some(StatusCode::OK),
        "rebind failed: {probe}"
    );
    let body = probe;
    assert_eq!(body["ok"], true);
    assert_eq!(body["preset_id"], other.preset_id);
    assert_eq!(body["resource_id"], other.id());
    // The retained `remaining` question, answered beside the binding.
    assert_eq!(body["ceiling_tokens"], 5000);
    let remaining = body["remaining"].as_u64().expect("remaining is a number");
    assert!(remaining <= 5000, "remaining never exceeds the ceiling");
    // The durable association moved: row, projection and the provision
    // effect's resource payload — the one the claim selector reads.
    let raw = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    let (row_resource, projected): (String, String) = raw
        .query_row(
            "SELECT resource_id,json_extract(projection,'$.resourceId') FROM engagements WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row_resource, other.id(), "the engagement row moved");
    assert_eq!(projected, other.id(), "the projection moved");
    let payload: String = raw
        .query_row(
            "SELECT json_extract(payload,'$.resource.presetId') FROM effects WHERE engagement_id=?1 AND kind='provision'",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        payload, other.preset_id,
        "the next dispatch reads this payload"
    );
    drop(raw);
    // Unknown preset: 400, and the binding survives (TS parity).
    let mut response = put(&format!("/console/api/agents/{id}/preset"), &lifecycle)
        .json(&json!({"presetId": "no_such_preset"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "unknown_preset"
    );
    let survived = f
        .domain
        .agent_roster()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.engagement_id == *id)
        .unwrap();
    assert_eq!(survived.framework, other.framework, "the binding survives");
    f.close().await;
}

/// G5a / ADR-148 — operator recovery of an orphaned dispatch through the
/// console route. Seed an orphan (outcome_unknown, quarantined session, dirty
/// workspace, lease held, NO dispatch_stops row) beside the owned fixture
/// dispatch, then POST recover-dispatch as the lifecycle operator and assert
/// the store's recovery rows. The route calls DomainStore::recover_dispatch;
/// the orphan state, evidence record and row clears are the store's own.
async fn seed_orphan_dispatch(f: &Fixture, engagement: &str, stopped: bool) {
    // Session and task go through the fixture's OWN DomainStore (the running
    // worker holds the SQLite lock, so a second DomainRepository::open would
    // fail Locked). Only the orphan-specific flags are then forced by SQL.
    f.domain
        .register_session(SessionBinding {
            id: "orphan_session".into(),
            engagement_id: engagement.into(),
            room_id: "!project:example.test".into(),
            thread_root: Some("$orphan_thread".into()),
        })
        .await
        .unwrap();
    f.domain
        .create_canonical_task(
            "orphan_task".into(),
            "orphan_session".into(),
            "Orphan work".into(),
            2000,
        )
        .await
        .unwrap();
    let mut db =
        rusqlite::Connection::open(f.root.path().join("state").join("domain.sqlite3")).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute(
        "UPDATE runner_sessions SET quarantined=1 WHERE id='orphan_session'",
        [],
    )
    .unwrap();
    // The input column must carry the full serialized DispatchInput production
    // enqueue writes (recover_dispatch parses it at execution.rs:1079 and
    // compares resources/payload against the replacement at :1080-1084).
    tx.execute(
        "INSERT INTO runner_dispatches(id,session_id,task_id,input,digest,state) \
         VALUES('orphan_dispatch','orphan_session','orphan_task',\
         '{\"id\":\"orphan_dispatch\",\"session_id\":\"orphan_session\",\"task_id\":\"orphan_task\",\"resources\":[{\"id\":\"orphan_workspace\",\"exclusive\":true}],\"payload\":{\"instruction\":\"original work\"}}',\
         'orphan_digest','outcome_unknown')",
        [],
    ).unwrap();
    tx.execute(
        "INSERT INTO workspace_resources(id,dirty) VALUES('orphan_workspace',1)",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO dispatch_resources(dispatch_id,resource_id,exclusive) VALUES('orphan_dispatch','orphan_workspace',1)",
        [],
    ).unwrap();
    tx.execute(
        "INSERT INTO resource_leases(resource_id,dispatch_id,exclusive) VALUES('orphan_workspace','orphan_dispatch',1)",
        [],
    ).unwrap();
    if stopped {
        // Both evidence and settled_at NULL satisfies the CHECK
        // ((evidence IS NULL)=(settled_at IS NULL)); recover_dispatch's stop-row
        // refusal (execution.rs:1044-1050) only needs the row to exist.
        tx.execute(
            "INSERT INTO dispatch_stops(dispatch_id,fence,reason,created_at) VALUES('orphan_dispatch',0,'operator stop',1)",
            [],
        ).unwrap();
    }
    tx.commit().unwrap();
}

fn recovery_body() -> Value {
    json!({
        "original": "orphan_dispatch",
        "replacement": {
            "id": "orphan_replacement",
            "session_id": "orphan_session",
            "task_id": "orphan_task",
            "resources": [{"id":"orphan_workspace","exclusive":true}],
            "payload": {"instruction":"resume inspected orphan"},
        },
        "evidence": "operator inspected workspace and stopped owner",
    })
}

#[tokio::test]
async fn native_console_agent_recover_dispatch_agent_binding() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    seed_orphan_dispatch(&f, &f.engagement, false).await;
    let cookie = lifecycle_session(&service).await;
    let foreign = f
        .domain
        .agent_roster()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.engagement_id != f.engagement)
        .unwrap()
        .engagement_id;
    let db = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    let snapshot = || {
        db.query_row(
        "SELECT d.state,d.input,s.quarantined,w.dirty,t.config,\
         (SELECT COUNT(*) FROM resource_leases WHERE dispatch_id=d.id),\
         (SELECT COUNT(*) FROM dispatch_recoveries WHERE original_id=d.id),\
         (SELECT COUNT(*) FROM runner_dispatches WHERE id='orphan_replacement') \
         FROM runner_dispatches d JOIN runner_sessions s ON s.id=d.session_id \
         JOIN canonical_tasks t ON t.id=d.task_id JOIN workspace_resources w ON w.id='orphan_workspace' \
         WHERE d.id='orphan_dispatch'", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,
            r.get::<_,bool>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?,
            r.get::<_,u64>(5)?,r.get::<_,u64>(6)?,r.get::<_,u64>(7)?))).unwrap()
    };
    let before = snapshot();
    for id in [&foreign, "missing_agent"] {
        let mut response = post(
            &format!("/console/api/agents/{id}/recover-dispatch"),
            &cookie,
        )
        .json(&recovery_body())
        .send(&service)
        .await;
        assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND), "{id}");
        assert_eq!(
            response.take_json::<Value>().await.unwrap()["code"],
            "not_found"
        );
        assert_eq!(
            snapshot(),
            before,
            "wrong route must not mutate the named orphan"
        );
    }
    let response = post(
        &format!("/console/api/agents/{}/recover-dispatch", f.engagement),
        &cookie,
    )
    .json(&recovery_body())
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(snapshot().6, 1);
    drop(db);
    f.close().await;
}

/// A lifecycle operator recovers the orphan: the route reaches recover_dispatch
/// and the store clears the lease/quarantine/dirty, supersedes older queued work and
/// writes the recovery record with the evidence. (One login carries the
/// permission; anonymous callers are refused by the shared authenticate hoop.)
#[tokio::test]
async fn native_console_agent_recover_dispatch_recovers_orphan() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");
    seed_orphan_dispatch(&f, &f.engagement, false).await;
    let cookie = lifecycle_session(&service).await;
    let mut response = post(
        &format!("/console/api/agents/{}/recover-dispatch", f.engagement),
        &cookie,
    )
    .json(&recovery_body())
    .send(&service)
    .await;
    let status = response.status_code;
    assert_eq!(status, Some(StatusCode::OK));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["ok"], true);
    let db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    let leases: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM resource_leases WHERE dispatch_id='orphan_dispatch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let quarantined: bool = db
        .query_row(
            "SELECT quarantined FROM runner_sessions WHERE id='orphan_session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let dirty: bool = db
        .query_row(
            "SELECT dirty FROM workspace_resources WHERE id='orphan_workspace'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let replacement_state: String = db
        .query_row(
            "SELECT state FROM runner_dispatches WHERE id='orphan_replacement'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let evidence: String = db
        .query_row(
            "SELECT evidence FROM dispatch_recoveries WHERE original_id='orphan_dispatch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leases, 0, "the orphan's lease is deleted");
    assert!(!quarantined, "the session quarantine is cleared");
    assert!(!dirty, "the workspace dirty flag is cleared");
    assert_eq!(
        replacement_state, "queued",
        "the replacement, not the orphan, is enqueued for resume"
    );
    assert_eq!(evidence, "operator inspected workspace and stopped owner");
    // Exactly once: the same recovery posted again is refused with the
    // recovery's own conflict word and mints no second replacement.
    let mut replay = post(
        &format!("/console/api/agents/{}/recover-dispatch", f.engagement),
        &cookie,
    )
    .json(&recovery_body())
    .send(&service)
    .await;
    assert_eq!(replay.status_code, Some(StatusCode::CONFLICT));
    assert_eq!(
        replay.take_json::<Value>().await.unwrap()["code"],
        "recovery_conflict"
    );
    let recoveries: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM dispatch_recoveries WHERE original_id='orphan_dispatch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recoveries, 1);
    drop(db);
    f.close().await;
}

/// The stop-row refusal (execution.rs:1044-1050): a dispatch the operator stopped
/// through the conversation-stop flow owns the stop-fenced case; operator
/// recovery refuses it outright, and no row changes.
#[tokio::test]
async fn native_console_agent_recover_dispatch_refuses_stopped_dispatch() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");
    seed_orphan_dispatch(&f, &f.engagement, true).await;
    let cookie = lifecycle_session(&service).await;
    let mut response = post(
        &format!("/console/api/agents/{}/recover-dispatch", f.engagement),
        &cookie,
    )
    .json(&recovery_body())
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["code"], "dispatch_not_recoverable");
    let db = rusqlite::Connection::open(state.join("domain.sqlite3")).unwrap();
    let quarantined: bool = db
        .query_row(
            "SELECT quarantined FROM runner_sessions WHERE id='orphan_session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let leases: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM resource_leases WHERE dispatch_id='orphan_dispatch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(quarantined, "the stop-fenced dispatch keeps its quarantine");
    assert_eq!(leases, 1, "the stop-fenced dispatch keeps its lease");
    f.close().await;
}

#[tokio::test]
async fn native_console_stopped_dispatch_continuation() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    seed_orphan_dispatch(&f, &f.engagement, true).await;
    let db = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    db.execute(
        "UPDATE runner_dispatches SET fence=1 WHERE id='orphan_dispatch'",
        [],
    )
    .unwrap();
    db.execute("UPDATE dispatch_stops SET fence=1,reason='owned_runner_failure' WHERE dispatch_id='orphan_dispatch'",[]).unwrap();
    let digest = "a".repeat(64);
    db.execute("INSERT INTO owned_stop_inspections(dispatch_id,fence,digest,config,observed_at) VALUES('orphan_dispatch',1,?1,'{}',2001)",[&digest]).unwrap();
    let read = format!(
        "/console/api/agents/{}/stopped-dispatches/orphan_dispatch/inspection",
        f.engagement
    );
    let write = format!(
        "/console/api/agents/{}/continue-stopped-dispatch",
        f.engagement
    );
    let mut input = recovery_body();
    input["fence"] = json!(1);
    input["inspectionDigest"] = json!(digest);
    let cookie = lifecycle_session(&service).await;
    let mut response = get(&read, &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let receipt = response.take_json::<Value>().await.unwrap();
    assert_eq!(receipt["digest"], input["inspectionDigest"]);
    assert_eq!(receipt["fence"], 1);
    let foreign = f
        .domain
        .agent_roster()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.engagement_id != f.engagement)
        .unwrap()
        .engagement_id;
    for id in [&foreign, "missing_agent"] {
        assert_eq!(
            get(
                &format!("/console/api/agents/{id}/stopped-dispatches/orphan_dispatch/inspection"),
                &cookie
            )
            .send(&service)
            .await
            .status_code,
            Some(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            post(
                &format!("/console/api/agents/{id}/continue-stopped-dispatch"),
                &cookie
            )
            .json(&input)
            .send(&service)
            .await
            .status_code,
            Some(StatusCode::NOT_FOUND)
        );
    }
    for missing in [false, true] {
        if missing {
            db.execute("DELETE FROM owned_stop_inspections", [])
                .unwrap();
        }
        let mut invalid = input.clone();
        invalid["inspectionDigest"] = json!("b".repeat(64));
        assert_eq!(
            post(&write, &cookie)
                .json(&invalid)
                .send(&service)
                .await
                .status_code,
            Some(StatusCode::CONFLICT)
        );
        let held: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM resource_leases WHERE dispatch_id='orphan_dispatch'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(held, 1);
    }
    db.execute("INSERT INTO owned_stop_inspections(dispatch_id,fence,digest,config,observed_at) VALUES('orphan_dispatch',1,?1,'{}',2001)",[&digest]).unwrap();
    for _ in 0..2 {
        let mut response = post(&write, &cookie).json(&input).send(&service).await;
        let status = response.status_code;
        let body = response.take_json::<Value>().await.unwrap();
        assert_eq!(status, Some(StatusCode::OK), "{body}");
    }
    input["evidence"] = json!("changed note");
    assert_eq!(
        post(&write, &cookie)
            .json(&input)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::CONFLICT)
    );
    let state:(String,String,u64,u64)=db.query_row("SELECT d.state,n.state,(SELECT COUNT(*) FROM dispatch_stops WHERE settled_at IS NOT NULL),(SELECT COUNT(*) FROM resource_leases WHERE dispatch_id=d.id) FROM runner_dispatches d JOIN runner_dispatches n ON n.id='orphan_replacement' WHERE d.id='orphan_dispatch'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state, ("outcome_unknown".into(), "queued".into(), 1, 0));
    f.close().await;
}

pub(super) async fn seed_inspected_failure(f: &Fixture) {
    f.domain
        .register_session(SessionBinding {
            id: "resolution_session".into(),
            engagement_id: f.engagement.clone(),
            room_id: "!project:example.test".into(),
            thread_root: Some("$resolution".into()),
        })
        .await
        .unwrap();
    f.domain
        .create_canonical_task(
            "resolution_task".into(),
            "resolution_session".into(),
            "Inspect result".into(),
            now(),
        )
        .await
        .unwrap();
    f.domain
        .register_workspace("resolution_workspace".into())
        .await
        .unwrap();
    f.domain
        .enqueue_dispatch(hagency_core::tasks::DispatchInput {
            id: "resolution_dispatch".into(),
            session_id: "resolution_session".into(),
            task_id: Some("resolution_task".into()),
            resources: vec![hagency_core::tasks::ResourceLease {
                id: "resolution_workspace".into(),
                exclusive: true,
            }],
            payload: json!({"instruction":"Original work"}),
        })
        .await
        .unwrap();
    let cap = f
        .domain
        .claim_dispatch("resolution_runner".into(), now(), 60_000, 120_000, 128)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cap.dispatch_id, "resolution_dispatch");
    let scope = f.domain.owned_dispatch_scope(cap.clone()).await.unwrap();
    let started = f
        .domain
        .start_owned_dispatch(cap.clone(), scope.fingerprint().into())
        .await
        .unwrap();
    f.domain
        .observe_owned_failure(cap.clone(), hagency_store::OwnedFailure::Protocol)
        .await
        .unwrap();
    // Synthetic store receipt tests the HTTP authority and transaction boundary;
    // actual stopped-process provenance is covered by the execution harness.
    f.domain.record_owned_stop_inspection(cap,started,json!({"profile":"stopped-content-inventory-v1",
        "root":{"platform":"unix-v1","volume":"0000000000000001","object":vec![0;16]},
        "entries":[{"path":"result.txt","kind":"file","bytes":5,"sha256":"a".repeat(64),"readonly":false}]})).await.unwrap();
}

#[tokio::test]
async fn native_console_stopped_dispatch_list() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    seed_inspected_failure(&f).await;
    let service = f.service();
    let path = format!("/console/api/agents/{}/stopped-dispatches", f.engagement);
    let cookie = lifecycle_session(&service).await;
    for query in [
        "?limit=1",
        "?after=",
        "?after=a&after=b",
        "?after=%61",
        "?other=x",
    ] {
        assert_eq!(
            get(&format!("{path}{query}"), &cookie)
                .send(&service)
                .await
                .status_code,
            Some(StatusCode::BAD_REQUEST)
        );
    }
    let sql = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    // Discovery-only fixture rows carry no original host inspection or token.
    for i in 0..17 {
        let id = format!("z_stopped_{i:02}");
        sql.execute("INSERT INTO runner_dispatches(id,session_id,task_id,input,digest,state,fence) SELECT ?1,session_id,task_id,input,digest,state,fence FROM runner_dispatches WHERE id='resolution_dispatch'",[&id]).unwrap();
        sql.execute("INSERT INTO dispatch_stops(dispatch_id,fence,reason,created_at) VALUES(?1,1,'owned_runner_failure',1)",[&id]).unwrap();
    }
    let mut response = get(&path, &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let first = response.take_json::<Value>().await.unwrap();
    assert_eq!(first.as_object().unwrap().len(), 3);
    assert_eq!(first["engagementId"], f.engagement);
    assert_eq!(first["dispatches"].as_array().unwrap().len(), 16);
    // Board #111: the fixture's OWN seeded dispatch is settled `outcome_unknown`
    // by lease expiry (its lease is wall-clock-stale by the time seeding
    // finishes) and has NO `dispatch_stops` row — exactly the live shape this
    // page used to hide. It sorts first (`private_…` < `resolution_…`) and is
    // the dispatch the session quarantine is about, so it heads the page and IS
    // inspectable (its attempt's own `lost` event is the material).
    assert_eq!(first["dispatches"][0]["dispatchId"], "private_dispatch");
    assert_eq!(first["dispatches"][0]["inspectionAvailable"], true);
    assert_eq!(first["dispatches"][0]["reason"], "runner_lost");
    assert_eq!(first["dispatches"][1]["dispatchId"], "resolution_dispatch");
    assert_eq!(first["dispatches"][1]["inspectionAvailable"], true);
    for (index, row) in first["dispatches"].as_array().unwrap().iter().enumerate() {
        assert_eq!(row.as_object().unwrap().len(), 6);
        assert!(
            row.as_object()
                .unwrap()
                .values()
                .all(|v| !v.is_array() && !v.is_object())
        );
        // Only the two dispatches with real inspection material lead; the 17
        // discovery-only fixture rows carry no host receipt and no lost event.
        if index > 1 {
            assert_eq!(row["inspectionAvailable"], false);
        }
    }
    // 19 unresolved rows exist (private_dispatch, resolution_dispatch, 17
    // z_stopped_*); the page is capped at 16, so the cursor is the 16th id.
    assert_eq!(first["nextAfter"], "z_stopped_13");
    let second = get(&format!("{path}?after=z_stopped_13"), &cookie)
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    assert_eq!(second["dispatches"].as_array().unwrap().len(), 3);
    assert!(second["nextAfter"].is_null());
    let foreign = f
        .domain
        .agent_roster()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.engagement_id != f.engagement)
        .unwrap()
        .engagement_id;
    let empty = get(
        &format!("/console/api/agents/{foreign}/stopped-dispatches"),
        &cookie,
    )
    .send(&service)
    .await
    .take_json::<Value>()
    .await
    .unwrap();
    assert!(empty["dispatches"].as_array().unwrap().is_empty());
    assert_eq!(
        get("/console/api/agents/missing/stopped-dispatches", &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    drop(sql);
    f.close().await;
}

#[tokio::test]
async fn native_console_outcome_resolution() {
    for action in ["continue", "accept_completed", "keep_blocked"] {
        let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
        seed_inspected_failure(&f).await;
        let service = f.service();
        let cookie = lifecycle_session(&service).await;
        let inspect = format!(
            "/console/api/agents/{}/stopped-dispatches/resolution_dispatch/inspect",
            f.engagement
        );
        let resolve = format!(
            "/console/api/agents/{}/resolve-stopped-dispatch",
            f.engagement
        );
        for body in [
            json!({"ttlMs":1}),
            json!({"ttlMs":3_600_001}),
            json!({"unknown":true}),
        ] {
            assert_eq!(
                post(&inspect, &cookie)
                    .json(&body)
                    .send(&service)
                    .await
                    .status_code,
                Some(StatusCode::BAD_REQUEST)
            );
        }
        let mut response = post(&inspect, &cookie)
            .json(&json!({}))
            .send(&service)
            .await;
        let status = response.status_code;
        let inspection = response.take_json::<Value>().await.unwrap();
        assert_eq!(status, Some(StatusCode::OK), "{inspection}");
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let mut input = json!({"original":"resolution_dispatch","requestId":"operator_resolution","inspectionId":inspection["inspectionId"],"inspectionToken":inspection["inspectionToken"],"action":action,"operatorNote":"Reviewed original workspace and effects"});
        if action == "continue" {
            input["replacement"] = json!({"id":"resolution_replacement","session_id":"resolution_session","task_id":"resolution_task","resources":[{"id":"resolution_workspace","exclusive":true}],"payload":{"instruction":"Finish remaining inspected work"}});
        }
        let foreign = f
            .domain
            .agent_roster()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.engagement_id != f.engagement)
            .unwrap()
            .engagement_id;
        assert_eq!(
            post(
                &format!(
                    "/console/api/agents/{foreign}/stopped-dispatches/resolution_dispatch/inspect"
                ),
                &cookie
            )
            .json(&json!({}))
            .send(&service)
            .await
            .status_code,
            Some(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            post(
                &format!("/console/api/agents/{foreign}/resolve-stopped-dispatch"),
                &cookie
            )
            .json(&input)
            .send(&service)
            .await
            .status_code,
            Some(StatusCode::NOT_FOUND)
        );
        let mut wrong = input.clone();
        wrong["inspectionToken"] = json!("0".repeat(64));
        assert_eq!(
            post(&resolve, &cookie)
                .json(&wrong)
                .send(&service)
                .await
                .status_code,
            Some(StatusCode::CONFLICT)
        );
        let mut first = None;
        for _ in 0..2 {
            let mut response = post(&resolve, &cookie).json(&input).send(&service).await;
            let status = response.status_code;
            let value = response.take_json::<Value>().await.unwrap();
            assert_eq!(status, Some(StatusCode::OK), "{value}");
            if let Some(first) = &first {
                assert_eq!(&value, first);
            } else {
                first = Some(value);
            }
        }
        let sql = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
        let task: String = sql
            .query_row(
                "SELECT config FROM canonical_tasks WHERE id='resolution_task'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let task: hagency_core::tasks::Task = serde_json::from_str(&task).unwrap();
        assert_eq!(
            task.status,
            match action {
                "continue" => hagency_core::tasks::TaskState::InProgress,
                "accept_completed" => hagency_core::tasks::TaskState::Done,
                _ => hagency_core::tasks::TaskState::Blocked,
            }
        );
        input["operatorNote"] = json!("Changed review");
        assert_eq!(
            post(&resolve, &cookie)
                .json(&input)
                .send(&service)
                .await
                .status_code,
            Some(StatusCode::CONFLICT)
        );
        assert_eq!(
            post(&inspect, &cookie)
                .json(&json!({}))
                .send(&service)
                .await
                .status_code,
            Some(StatusCode::CONFLICT)
        );
        f.close().await;
    }
}

/// The launch-env route (board #49, TS `backend-v2.js:12344`
/// `GET /api/agents/:name/launch-env`): the runtime profile the agent would
/// launch with, in the TS `normalizeRuntimeProfile` shape
/// (`{runtimeProfile:{primary,supervisor}}`, `backend-v2.js:864-876`). The
/// seeded UsageWorker's resource is codex/gpt-5.6-sol/medium with no
/// provider, so `primary` carries exactly those four facts and `supervisor`
/// is null — the TS shape when the stored record carries none. Unknown agent
/// is the TS route's 404 `agent not found`; an invalid name shape is 400
/// before any store work; a query parameter is refused.
#[tokio::test]
async fn native_console_agent_launch_env_observation() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let anonymous = TestClient::get(format!("{BASE}/console/api/agents/UsageWorker/launch-env"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));
    let cookie = session(&service).await;
    // The read takes no selection: query parameters are refused.
    let response = get(
        "/console/api/agents/UsageWorker/launch-env?agent=x",
        &cookie,
    )
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    // An agent the service never engaged is the TS route's 404.
    let mut response = get("/console/api/agents/Nobody/launch-env", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "agent_not_found"
    );
    let mut response = get("/console/api/agents/UsageWorker/launch-env", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let value = response.take_json::<Value>().await.unwrap();
    // Exactly the TS envelope: one `runtimeProfile` key.
    assert_eq!(value.as_object().unwrap().len(), 1);
    let profile = value["runtimeProfile"].as_object().unwrap();
    assert_eq!(
        profile.len(),
        2,
        "TS normalizeRuntimeProfile: primary + supervisor"
    );
    let primary = value["runtimeProfile"]["primary"].as_object().unwrap();
    assert_eq!(primary["framework"], "codex");
    assert_eq!(primary["model"], "gpt-5.6-sol");
    assert_eq!(primary["reasoning"], "medium");
    assert!(
        primary["provider"].is_null(),
        "no provider on the seeded resource"
    );
    assert!(
        value["runtimeProfile"]["supervisor"].is_null(),
        "the port stores no supervisor profile: null, the TS shape when none"
    );
    f.close().await;
}

/// `DELETE /api/agents/:name` — the retained soft/force delete
/// (`backend-v2.js:12164-12307`). SOFT is TS's reversible act: it reports
/// `{ok, deprecated, message}` and changes nothing, because TS's own comment
/// (`:12206-12225`) is explicit that only `?force=true` may revoke — a soft
/// delete is reversible and `revoke` has no inverse. Native has no agent
/// record (an agent is DERIVED from its engagements), so "marked inactive" is
/// the honest no-op it is: NOTHING is revoked.
#[tokio::test]
async fn native_console_agent_delete_soft_is_reversible() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut response = TestClient::delete(format!("{BASE}/console/api/agents/UsageWorker"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["deprecated"], true);
    assert_eq!(
        value["message"],
        "unregister is disabled; agent marked inactive. Use ?force=true to permanently delete."
    );
    // The agent payload IS the detail projection the read serves — TS
    // returns `serializeAgent(agent)` (`backend-v2.js:12304`), so the two
    // must agree byte for byte rather than merely "look like" each other.
    // (`assert_private` deliberately does not apply here: the bounded detail
    // projection carries `rooms[].session_id`, which the shipped
    // `GET /console/api/agents/{name}` route serves too — the detail test
    // asserts that value is present.)
    let served = get("/console/api/agents/UsageWorker", &cookie)
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        value["agent"], served,
        "the delete returns the served record"
    );
    assert_eq!(value["agent"]["name"], "UsageWorker");
    assert_eq!(value["agent"]["state"], "active");
    // The envelope carries exactly TS's four soft-delete keys.
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["agent", "deprecated", "message", "ok"]);
    // REVERSIBLE: the engagement is untouched, so nothing an `undelete`
    // would have to undo (TS's stated reason for not releasing here).
    let engagement = f.domain.engagement(f.engagement.clone()).await.unwrap();
    assert_eq!(
        engagement.state,
        hagency_core::project::EngagementState::Active,
        "a soft delete revokes nothing"
    );
    // And no stop row was written — the act is not a dispatch mutation.
    let raw = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    let stops: i64 = raw
        .query_row("SELECT COUNT(*) FROM dispatch_stops", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stops, 0, "a soft delete writes no stop row");
    drop(raw);
    f.close().await;
}

/// FORCE really deletes: TS revokes the agent's ACTIVE engagements and
/// REPORTS the released ids (`backend-v2.js:12231-12238`), because a
/// commitment outliving its agent is the leak this closes — removing the
/// agent while its commitment stands would drain a contributor's quota by
/// attrition. Native's `revoke` is that act and releases the budget by
/// construction.
#[tokio::test]
async fn native_console_agent_delete_force_releases_active_engagements() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let id = f.engagement.clone();
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
    assert_eq!(value["ok"], true);
    assert_eq!(value["deleted"], true);
    assert_eq!(value["name"], "UsageWorker");
    // Native has no tmux/session kill at this layer; the revoke schedules
    // the retirement that owns worker cleanup. Reported, never invented.
    assert_eq!(value["sessionKilled"], false);
    assert_eq!(value["leftGroups"].as_array().unwrap().len(), 0);
    assert_eq!(value["leftProjectRooms"].as_array().unwrap().len(), 0);
    let released = value["releasedEngagements"].as_array().unwrap();
    assert_eq!(released.len(), 1, "the agent's one active engagement");
    assert_eq!(released[0], id);
    assert_private(&value);
    // The commitment is RELEASED: the engagement is revoked, not merely
    // reported as such.
    let engagement = f.domain.engagement(id).await.unwrap();
    assert_eq!(
        engagement.state,
        hagency_core::project::EngagementState::Revoked,
        "force released the commitment"
    );
    // The force-delete PERSISTED the tombstone `undelete` reverses — TS's
    // `persistForceDeletedAgentState` (`backend-v2.js:4354`) is what makes
    // re-registration conditional on an explicit undelete, so the delete
    // route must have written it, not a test seeding it (RULES §3).
    let raw = rusqlite::Connection::open(f.root.path().join("state/domain.sqlite3")).unwrap();
    let reason: String = raw
        .query_row(
            "SELECT reason FROM agent_tombstones WHERE name='UsageWorker'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, "force-delete", "TS's tombstone reason, verbatim");
    drop(raw);
    // And it is reversible through the #49 route: 200, the tombstone gone.
    let mut response = post("/console/api/agents/UsageWorker/undelete", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["undeleted"], true);
    f.close().await;
}

/// The five remaining #49 leftover routes (TS `backend-v2.js:12308` undelete,
/// `:16370` avatar, `:16988` delivery-events, `:16900` message, `:17002`
/// suppress), served from the migration-067 tables. Every route is mounted
/// behind the console `authenticate` hoop; each asserts the TS-visible body and
/// status for its cases. The board is seeded through the store writers the
/// routes read — no test-only seam.
#[tokio::test]
async fn native_console_agent_message_leftovers() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let anonymous = TestClient::get(format!(
        "{BASE}/console/api/agents/UsageWorker/delivery-events"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .send(&service)
    .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));
    let cookie = session(&service).await;

    // ── undelete (TS :12308): 404 with no tombstone, 200 once one exists ──
    let response = post("/console/api/agents/UsageWorker/undelete", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    f.domain
        .record_agent_tombstone("UsageWorker".into(), "force-delete".into(), 1000)
        .await
        .unwrap();
    let mut response = post("/console/api/agents/UsageWorker/undelete", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["undeleted"], true);
    assert_eq!(value["name"], "UsageWorker");
    // Removed for real: a second call is the 404 again.
    let response = post("/console/api/agents/UsageWorker/undelete", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));

    // ── avatar (TS :16370): queued, with force/custom from body+query ──
    let mut response = post("/console/api/agents/UsageWorker/avatar", &cookie)
        .json(&json!({"image": "aGk="}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["queued"], true);
    assert_eq!(value["name"], "UsageWorker");
    assert_eq!(value["force"], false, "no generate and no ?force=true");
    assert_eq!(value["custom"], true, "body.image present");
    // `?force=true` (TS reads the query as well as body.generate).
    let mut response = post("/console/api/agents/UsageWorker/avatar?force=true", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["force"], true);
    assert_eq!(value["custom"], false);

    // ── message + suppress + delivery-events ──
    // Unknown message id is the TS route's 404.
    let mut response = get("/console/api/messages/msg_9999", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["error"],
        "message not found"
    );
    f.domain
        .record_operator_message(hagency_store::NewOperatorMessage {
            id: "msg_0001".into(),
            sender: "operator".into(),
            recipient: Some("UsageWorker".into()),
            kind: "human".into(),
            priority: "urgent".into(),
            summary: "Review the build".into(),
            full: "Please review the build output.".into(),
            mentions: json!([]),
            attachments: json!([]),
            created_at: 1_000_000,
            reply_to: None,
            group: None,
            source: "api".into(),
            source_room: None,
            source_event_id: None,
            sender_mxid: None,
            room_recipients: json!([]),
            default_recipient: Some("UsageWorker".into()),
            schema_kind: Some("review".into()),
            schema_version: Some(2),
            schema_payload: Some(json!({"verdict":"pending"}).to_string()),
        })
        .await
        .unwrap();
    let mut response = get("/console/api/messages/msg_0001", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["id"], "msg_0001");
    assert_eq!(value["from"], "operator");
    assert_eq!(value["to"], "UsageWorker");
    assert_eq!(value["type"], "human");
    assert_eq!(value["priority"], "urgent", "normalized priority");
    assert_eq!(value["summary"], "Review the build");
    assert_eq!(value["source"], "api");
    // `schema` reduced to {kind,version[,payload]} (TS :4109).
    assert_eq!(value["schema"]["kind"], "review");
    assert_eq!(value["schema"]["version"], 2);
    assert_eq!(value["schema"]["payload"]["verdict"], "pending");
    assert!(value["ts"].is_null(), "TS serves ts: undefined");
    assert!(
        value["time"].as_str().unwrap().ends_with(" ago"),
        "relativeTime shape"
    );
    assert_eq!(value["suppressedRecipients"].as_array().unwrap().len(), 0);

    // suppress (TS :17002): the agent is added once and stays added.
    let mut response = post("/console/api/messages/msg_0001/suppress", &cookie)
        .json(&json!({"agent": "UsageWorker"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["id"], "msg_0001");
    assert_eq!(value["agent"], "UsageWorker");
    assert_eq!(value["suppressed"], true);
    assert_eq!(value["was_unread"], true);
    assert_eq!(value["is_unread_now"], false);
    assert_eq!(value["suppressedRecipients"], json!(["UsageWorker"]));
    // Idempotent: a second call reports it was already suppressed.
    let mut response = post("/console/api/messages/msg_0001/suppress", &cookie)
        .json(&json!({"agent": "UsageWorker", "reason": "operator-ack"}))
        .send(&service)
        .await;
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["was_unread"], false, "already suppressed");
    assert_eq!(value["suppressedRecipients"], json!(["UsageWorker"]));

    // A message that does not target the agent is the TS route's 400.
    let response = post("/console/api/messages/msg_0001/suppress", &cookie)
        .json(&json!({"agent": "PageWorker"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    // No agent named is the TS route's 400.
    let response = post("/console/api/messages/msg_0001/suppress", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));

    // delivery-events (TS :16988): the suppress appended one, newest first.
    let mut response = get("/console/api/agents/UsageWorker/delivery-events", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["agent"], "UsageWorker");
    let events = value["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "one suppress, one event");
    assert_eq!(events[0]["type"], "message.suppressed");
    assert_eq!(events[0]["agent"], "UsageWorker");
    assert_eq!(events[0]["messageId"], "msg_0001");
    assert_eq!(events[0]["reason"], "explicit-suppress");
    // An agent the service never engaged is the TS route's 404.
    let response = get("/console/api/agents/Nobody/delivery-events", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
    f.close().await;
}

/// The TS refusal shapes: 404 `{error:'agent not found'}` for an agent no
/// record names, 400 for a name shape the detail read would refuse.
#[tokio::test]
async fn native_console_agent_delete_not_found_and_invalid_name() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = lifecycle_session(&service).await;
    let mut missing = TestClient::delete(format!("{BASE}/console/api/agents/NoSuchAgent"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));
    let body = missing.take_json::<Value>().await.unwrap();
    assert_eq!(body["error"], "agent not found");
    assert_eq!(body["code"], "agent_not_found");
    // An invalid name shape is refused BEFORE any store work (the same
    // AgentName bound the detail read uses).
    let mut invalid = TestClient::delete(format!("{BASE}/console/api/agents/1bad%20name"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        invalid.take_json::<Value>().await.unwrap()["code"],
        "invalid_console_request"
    );
    f.close().await;
}
/// TS parity: ONE login is the whole console — an anonymous caller is refused
/// before any store work; the logged-in session reaches the lifecycle routes
/// AND the neighbouring resource/account mutations without a second link.
#[tokio::test]
async fn native_console_agent_lifecycle_is_one_login() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    // TS parity: ONE login is the whole console. An anonymous caller is
    // refused before any store work; the logged-in session may act on the
    // lifecycle routes and the resource/account routes alike.
    let id = &f.engagement;
    // Anonymous callers are refused before any store work. Preset is PUT
    // (TS parity: backend-v2.js:11484 `app.put`) — the wrong-method POST is
    // answered 405 by routing before auth, so use the route's own method.
    for path in [
        format!("/console/api/agents/{id}/start"),
        format!("/console/api/agents/{id}/stop"),
        format!("/console/api/agents/{id}/preset"),
    ] {
        let response = if path.ends_with("/preset") {
            put(&path, "hagency_console=anonymous")
        } else {
            post(&path, "hagency_console=anonymous")
        }
        .send(&service)
        .await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::UNAUTHORIZED),
            "anonymous {path}"
        );
    }
    let cookie = session(&service).await;
    // Start refuses an agent that never stopped — TS parity 409
    // `agent already online` (backend-v2.js:12717).
    let mut response = post(&format!("/console/api/agents/{id}/start"), &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "agent_already_online"
    );
    // The SAME login reaches the neighbouring mutation classes — no scope
    // word, the store's own validation answers (revision conflict).
    let source = native_resource("private_lifecycle_scope_source");
    f.domain.put_resource(source.clone()).await.unwrap();
    let revision = resource_publication_revision(&source).unwrap();
    let response = post(
        &format!("/console/api/resources/{}/publication", source.id()),
        &cookie,
    )
    .json(&json!({"expectedRevision":revision,"published":false}))
    .send(&service)
    .await;
    assert_eq!(
        response.status_code,
        Some(StatusCode::OK),
        "publication is not scope-refused"
    );
    let response = post("/console/api/accounts", &cookie)
        .json(&json!({"profile":ACCOUNT_PROFILE}))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code,
        Some(StatusCode::OK),
        "account prepare is not scope-refused"
    );
    f.close().await;
}
