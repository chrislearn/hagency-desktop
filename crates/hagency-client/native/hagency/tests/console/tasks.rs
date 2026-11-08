use super::*;
use hagency_store::{MAX_TASK_COMMENTS, MAX_TASK_PAGE};

/// Issue one scoped console ticket and exchange it for a cookie, the same
/// finite-ticket flow `configuration.rs` uses. The authority throttles ticket
/// issuance, so callers space their calls by `throttle()`.
/// One login is the whole console (the operator's decision; integ's typing
/// #41 removed the per-scope issue routes): the former scoped helper is the
/// same plain `session()` every other test uses.
async fn scoped(service: &Service, _scope: &str) -> String {
    session(service).await
}

/// One configurable session: the scope the task writes require.
async fn writer(service: &Service) -> String {
    tokio::time::sleep(std::time::Duration::from_millis(1010)).await;
    scoped(service, "resource-configuration-access").await
}

fn patch(path: &str, cookie: &str) -> salvo::test::RequestBuilder {
    TestClient::patch(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", cookie, true)
}

fn delete(path: &str, cookie: &str) -> salvo::test::RequestBuilder {
    TestClient::delete(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", cookie, true)
}

/// The task lifecycle the operator drives (`backend-v2.js:13194-13332`,
/// `lib/task-store.js`): create, read back, filter, edit, comment, the ten-step
/// status walk, and delete — asserted on the ROUTE's own JSON, which is the
/// TS-visible outcome.
#[tokio::test]
async fn native_console_task_lifecycle_over_the_routes() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = writer(&service).await;

    // POST /api/tasks — the retained `{ok:true, task}` envelope, with the
    // retained defaults and the server-served `next`.
    let mut response = post("/console/api/tasks", &cookie)
        .json(&json!({"title":"Wire the board","assignee":"Octos","labels":["a","a","b"]}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let created = response.take_json::<Value>().await.unwrap();
    assert_eq!(created["ok"], true);
    let task = created["task"].clone();
    let id = task["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("task_"), "{id}");
    assert_eq!(task["status"], "created");
    assert_eq!(task["priority"], "p2");
    assert_eq!(task["granularity"], "task");
    assert_eq!(task["labels"], json!(["a", "b"]));
    assert_eq!(task["comments"], json!([]));
    assert_eq!(
        task["next"],
        json!(["accepted"]),
        "the server serves the map"
    );

    // A missing title is the retained store's own word.
    let mut response = post("/console/api/tasks", &cookie)
        .json(&json!({"title":"   "}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "invalid_task_command"
    );

    // GET /api/tasks — the list envelope, plus the two server-owned facts.
    let mut response = get("/console/api/tasks", &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let list = response.take_json::<Value>().await.unwrap();
    assert!(list["at_ms"].as_u64().unwrap() > 0);
    assert_eq!(list["permissions"]["configureResource"], true);
    assert_eq!(
        list["unavailable"],
        json!(["health"]),
        "health is named, not zeroed"
    );
    assert_eq!(list["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(list["tasks"][0]["id"], id.as_str());

    // The retained filters, one at a time.
    for (query, expected) in [
        ("?assignee=Octos", 1),
        ("?assignee=nobody", 0),
        ("?status=created", 1),
        ("?status=done", 0),
        ("?priority=p2", 1),
        ("?priority=p0", 0),
        ("?label=b", 1),
        ("?label=zz", 0),
        ("?offset=1", 0),
        ("?limit=1", 1),
        ("?limit=1&offset=1", 0),
    ] {
        let mut response = get(&format!("/console/api/tasks{query}"), &cookie)
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK), "{query}");
        let value = response.take_json::<Value>().await.unwrap();
        assert_eq!(
            value["tasks"].as_array().unwrap().len(),
            expected,
            "{query}"
        );
    }
    // An unknown key, a duplicated key and an over-large page are refused.
    for query in [
        "?bogus=1",
        "?limit=1&limit=2",
        &format!("?limit={}", MAX_TASK_PAGE + 1),
    ] {
        let response = get(&format!("/console/api/tasks{query}"), &cookie)
            .send(&service)
            .await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::BAD_REQUEST),
            "{query}"
        );
    }

    // GET /api/tasks/:id — the bare row, no envelope.
    let mut response = get(&format!("/console/api/tasks/{id}"), &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["id"],
        id.as_str()
    );
    assert_eq!(
        get("/console/api/tasks/task_missing", &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );

    // PATCH /api/tasks/:id — the operator's full-field edit. The status is NOT
    // editable here: only `/transition` moves it.
    let mut response = patch(&format!("/console/api/tasks/{id}"), &cookie)
        .json(&json!({"priority":"p0","assignee":null,"status":"done","description":"edited"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let edited = response.take_json::<Value>().await.unwrap()["task"].clone();
    assert_eq!(edited["priority"], "p0");
    assert_eq!(edited["assignee"], Value::Null);
    assert_eq!(edited["status"], "created", "PATCH cannot move the status");
    assert_eq!(edited["description"], "edited");

    // POST /api/tasks/:id/comments — the retained comment shape.
    let mut response = post(&format!("/console/api/tasks/{id}/comments"), &cookie)
        .json(&json!({"text":"started"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let commented = response.take_json::<Value>().await.unwrap()["task"].clone();
    assert_eq!(commented["comments"][0]["author"], "anonymous");
    assert_eq!(commented["comments"][0]["text"], "started");
    assert!(
        commented["comments"][0]["ts"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

    // POST /api/tasks/:id/transition — `status` is required, and the walk
    // follows the retained map.
    let mut response = post(&format!("/console/api/tasks/{id}/transition"), &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "status_required"
    );
    let response = post(&format!("/console/api/tasks/{id}/transition"), &cookie)
        .json(&json!({"status":"in_progress"}))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code,
        Some(StatusCode::BAD_REQUEST),
        "created -> in_progress is not a retained pair"
    );
    let mut response = post(&format!("/console/api/tasks/{id}/transition"), &cookie)
        .json(&json!({"status":"accepted"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let accepted = response.take_json::<Value>().await.unwrap()["task"].clone();
    assert_eq!(accepted["status"], "accepted");
    assert_eq!(accepted["next"], json!(["in_progress"]));
    assert!(accepted["started_at"].as_str().is_some());
    // `blocked` needs both metadata fields.
    let response = post(&format!("/console/api/tasks/{id}/transition"), &cookie)
        .json(&json!({"status":"in_progress"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let response = post(&format!("/console/api/tasks/{id}/transition"), &cookie)
        .json(&json!({"status":"blocked"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    let mut response = post(&format!("/console/api/tasks/{id}/transition"), &cookie)
        .json(&json!({"status":"blocked","waiting_reason":"needs a decision","waiting_until":"2026-01-01T00:00:00.000Z"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let blocked = response.take_json::<Value>().await.unwrap()["task"].clone();
    assert_eq!(blocked["waiting_reason"], "needs a decision");
    assert_eq!(blocked["next"], json!(["in_progress"]));

    // POST /api/tasks/:id/accept — the retained named transition; it is the
    // same `transitionTask(id, 'accepted')` call, so a terminal row refuses it.
    let response = post(&format!("/console/api/tasks/{id}/accept"), &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));

    // DELETE /api/tasks/:id — the retained `{ok:true, task}` reply, then 404.
    let mut response = delete(&format!("/console/api/tasks/{id}"), &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let removed = response.take_json::<Value>().await.unwrap();
    assert_eq!(removed["ok"], true);
    assert_eq!(removed["task"]["id"], id.as_str());
    assert_eq!(
        delete(&format!("/console/api/tasks/{id}"), &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    f.close().await;
}

/// The authority matrix: a read-only session may list but not write, and the
/// missing-scope word is served before any store job.
#[tokio::test]
async fn native_console_tasks_authority_matrix() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();

    // Anonymous: no session, no read.
    let anonymous = TestClient::get(format!("{BASE}/console/api/tasks"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));

    let readonly = session(&service).await;
    // The foreign-header and poisoned-cookie matrix, as the alerts read applies it.
    for (name, value) in [
        ("host", "evil.test"),
        ("origin", "https://evil.test"),
        ("sec-fetch-site", "cross-site"),
        ("x-forwarded-for", "127.0.0.1"),
        ("cookie", "hagency_console=bad"),
    ] {
        let response = get("/console/api/tasks", &readonly)
            .add_header(name, value, true)
            .send(&service)
            .await;
        assert!(matches!(
            response.status_code,
            Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        ));
    }
    // One login lists and carries the configure permission (the operator's
    // one-login decision; `can_configure` is `logged_in`).
    let mut response = get("/console/api/tasks", &readonly).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["permissions"]["configureResource"], true);
    assert_eq!(value["tasks"], json!([]));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    f.close().await;
}

/// The per-agent list and the project board: their own routes, the same
/// envelope, and the board's named-unavailable columns.
#[tokio::test]
async fn native_console_agent_tasks_and_project_board() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = writer(&service).await;

    for (title, assignee) in [("For Aria", "Aria"), ("For Octos", "Octos")] {
        let response = post("/console/api/tasks", &cookie)
            .json(&json!({"title":title,"assignee":assignee}))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
    }

    // GET /api/agents/:name/tasks — the assignee filter under its own path.
    let mut response = get("/console/api/agents/Aria/tasks", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(value["tasks"][0]["assignee"], "Aria");
    assert!(value["at_ms"].as_u64().unwrap() > 0);
    assert_eq!(
        get("/console/api/agents/nobody/tasks", &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    // An agent path takes no selection.
    assert_eq!(
        get("/console/api/agents/Aria/tasks?limit=1", &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );

    // GET /api/project-board — the retained envelope with native's named gaps.
    let mut response = get("/console/api/project-board", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let board = response.take_json::<Value>().await.unwrap();
    assert!(board["generatedAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(board["staleAfterMs"], 300_000);
    assert_eq!(board["activityLimit"], 20);
    // The console fixture admits an approved engagement on `project_one`, so
    // the board reports exactly that one project and its one active member.
    assert_eq!(board["totals"]["projects"], 1);
    assert_eq!(board["totals"]["agents"], 1);
    assert_eq!(
        board["totals"]["tasks"]["created"], 2,
        "both new tasks are created"
    );
    let projects = board["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["id"], "project_one");
    assert_eq!(projects[0]["agents"], json!(["UsageWorker"]));
    // The lane counts only tasks whose ASSIGNEE is a project member: the two
    // tasks above name Aria/Octos, who are not on `project_one`, so every lane
    // is 0 while the global totals still see both tasks. That difference is
    // the point of lanes.
    assert_eq!(projects[0]["taskLanes"]["created"], 0);
    assert_eq!(
        projects[0]["taskLanes"]["done"], 0,
        "an empty lane is served as 0, not omitted"
    );
    let unavailable = board["unavailable"].as_array().unwrap();
    for column in ["health", "repositories", "worktrees", "activity"] {
        assert!(
            unavailable.iter().any(|v| v == column),
            "{column} is named unavailable rather than zeroed"
        );
    }
    // `?activity_limit` is the one accepted selection; anything else is refused.
    assert_eq!(
        get("/console/api/project-board?activity_limit=5", &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    assert_eq!(
        get("/console/api/project-board?bogus=1", &cookie)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    f.close().await;
}

/// The retained comment bound (`lib/task-store.js:301`): the 101st comment is
/// refused rather than silently dropped.
#[tokio::test]
async fn native_console_task_comment_bound() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = writer(&service).await;
    let mut response = post("/console/api/tasks", &cookie)
        .json(&json!({"title":"Many comments"}))
        .send(&service)
        .await;
    let id = response.take_json::<Value>().await.unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for index in 0..MAX_TASK_COMMENTS {
        let response = post(&format!("/console/api/tasks/{id}/comments"), &cookie)
            .json(&json!({"text":format!("comment {index}")}))
            .send(&service)
            .await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::OK),
            "comment {index}"
        );
    }
    let mut response = post(&format!("/console/api/tasks/{id}/comments"), &cookie)
        .json(&json!({"text":"one too many"}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "invalid_task_command"
    );
    // The row still holds exactly the bound.
    let mut response = get(&format!("/console/api/tasks/{id}"), &cookie)
        .send(&service)
        .await;
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["comments"]
            .as_array()
            .unwrap()
            .len(),
        MAX_TASK_COMMENTS
    );
    f.close().await;
}

/// Board #92: every tasks route must answer the methods a real browser sends,
/// through the REAL console router (`Fixture::service` = `App::router`, the
/// same builder `hagency serve` serves). A test on `tasks::router()` alone
/// cannot see this class of defect — and neither could the lifecycle test
/// above, which only ever issued GET.
///
/// The live symptom was `405 (Method Not Allowed)` logged while
/// `/console/tasks/` was open. The tasks page's own "Project board" `<Link>`
/// makes Next issue a **HEAD** prefetch (`next/dist/shared/lib/router` sends
/// `method: 'HEAD'`), and salvo does not fold HEAD onto a `.get()` route the
/// way the retained Express server does — so the request fell past the GET
/// child, matched the path with a different method, and salvo answered 405.
/// (A second, independent half of the same gap: the read guard called HEAD a
/// mutation, so even a routed HEAD was refused its session by the `Origin`
/// requirement a browser omits on a same-origin prefetch.)
/// This pins the family: GET still answers, and HEAD answers too, for the API
/// routes. Document rendering now belongs to Desktop.
#[tokio::test]
async fn native_console_tasks_routes_answer_head_like_the_retained_get() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = writer(&service).await;

    let mut created = post("/console/api/tasks", &cookie)
        .json(&json!({"title":"Head parity"}))
        .send(&service)
        .await;
    let id = created.take_json::<Value>().await.unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Every GET route in the card's family: GET answers, and HEAD answers.
    for path in [
        "/console/api/tasks".to_owned(),
        format!("/console/api/tasks/{id}"),
        "/console/api/agents/Alice/tasks".to_owned(),
        "/console/api/project-board".to_owned(),
    ] {
        let listed = get(&path, &cookie).send(&service).await;
        assert_eq!(listed.status_code, Some(StatusCode::OK), "GET {path}");
        let headed = head(&path, &cookie).send(&service).await;
        assert_eq!(
            headed.status_code,
            Some(StatusCode::OK),
            "HEAD {path} must answer as the retained Express GET does, not 405"
        );
    }

    f.close().await;
}
