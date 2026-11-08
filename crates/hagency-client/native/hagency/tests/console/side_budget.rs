use super::*;

// `PUT /api/project-sides/:id/allocation` and `GET …/budget`
// (backend-v2.js:9541, :9567) on the console wire: the write behind the
// configuration scope, the read beside the scope-free project-sides list,
// and the fleet totals block (backend-v2.js:15700-15720) that names its
// own denominator. Every expected figure is derived from the shared seed:
// two approved (reserved) engagements of 100 tokens each commit 200, the
// third admitted-but-pending one commits nothing, and exactly one of the
// three agents carries observed usage.

/// One login is the whole console (the operator's decision; the per-scope
/// issue routes are gone): the former configuration-scoped helper is the
/// same plain `session()` every other test uses.
async fn configuration(service: &Service) -> String {
    session(service).await
}

fn put(path: &str, cookie: &str) -> salvo::test::RequestBuilder {
    TestClient::put(format!("{BASE}{path}"))
        .add_header("host", "127.0.0.1:13300", true)
        .add_header("origin", BASE, true)
        .add_header("sec-fetch-site", "same-origin", true)
        .add_header("cookie", cookie, true)
}

/// The allocation write and read: one login sets and clears, and the reply
/// carries the SAME six-key side record the list serves beside the budget
/// (backend-v2.js:9545 `{ok, side, budget}`).
#[tokio::test]
async fn native_console_side_allocation_write_and_read() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let operator = configuration(&service).await;
    let body = json!({"allocated_tokens": 500});
    let mut response = put(
        "/console/api/project-sides/example.test/allocation",
        &operator,
    )
    .json(&body)
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(
        value.as_object().unwrap().len(),
        3,
        "exactly ok, side, budget"
    );
    assert_eq!(value["ok"], true);
    let side = &value["side"];
    assert_eq!(side["id"], "example.test", "the id IS the server name");
    assert_eq!(
        side.as_object().unwrap().len(),
        6,
        "the same six-key projection the list serves"
    );
    let budget = &value["budget"];
    assert_eq!(budget["allocated"], 500);
    assert_eq!(budget["committed"], 200, "two reserved engagements of 100");
    assert_eq!(budget["remaining"], 300);
    assert_eq!(budget["commitments"].as_array().unwrap().len(), 2);
    assert_eq!(budget["totalCommitted"], 200);
    assert_eq!(budget["poolCommitted"], 0);
    assert_eq!(budget["orphanedCommitted"], 0);

    // The camelCase spelling is accepted exactly like the retained
    // `req.body?.allocated_tokens ?? req.body?.allocatedTokens`
    // (backend-v2.js:9543).
    let mut response = put(
        "/console/api/project-sides/example.test/allocation",
        &operator,
    )
    .json(&json!({"allocatedTokens": 50}))
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["budget"]["allocated"], 50);
    assert_eq!(value["budget"]["remaining"], 0, "saturates, never negative");

    // The read spreads the budget FLAT beside sideId — the retained route's
    // own spread (backend-v2.js:9571) — with no nested budget key.
    let mut response = get("/console/api/project-sides/example.test/budget", &operator)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["sideId"], "example.test");
    assert!(value.get("budget").is_none(), "the budget travels flat");
    assert_eq!(value["allocated"], 50);
    assert_eq!(value["committed"], 200);
    assert_eq!(value["remaining"], 0);
    assert_eq!(value["commitments"].as_array().unwrap().len(), 2);

    // The clear: NULL is unallocated — not unlimited — and remaining goes
    // null with it (lib/project-side-store.js:443-452).
    let mut response = put(
        "/console/api/project-sides/example.test/allocation",
        &operator,
    )
    .json(&json!({}))
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["budget"]["allocated"], Value::Null);
    assert_eq!(value["budget"]["remaining"], Value::Null);

    // The store owns the not-found verdict (backend-v2.js:9544, :9568).
    assert_eq!(
        put("/console/api/project-sides/nope.test/allocation", &operator)
            .json(&body)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    assert_eq!(
        get("/console/api/project-sides/nope.test/budget", &operator)
            .send(&service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    // A list observation takes no selection: the budget reads likewise
    // refuse every query parameter.
    assert_eq!(
        get(
            "/console/api/project-sides/example.test/budget?limit=1",
            &operator
        )
        .send(&service)
        .await
        .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
}

/// The fleet totals block (backend-v2.js:15700-15720): the seed's three
/// agents, exactly one measured, with the busy-time and task columns named
/// as unavailable rather than invented as zero.
#[tokio::test]
async fn native_console_usage_totals() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/usage/totals", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value: Value = response.take_json().await.unwrap();
    assert_eq!(value["ok"], true);
    let totals = &value["totals"];
    assert_eq!(totals["agents"], 3, "UsageWorker, AlertWorker, PageWorker");
    assert_eq!(
        totals["tokensMeasuredFor"], 1,
        "only UsageWorker is observed"
    );
    assert_eq!(totals["tokensPartial"], true);
    assert!(
        totals["tokensDrawn"].is_u64() && totals["tokensDrawn"].as_u64().unwrap() > 0,
        "the measured figure is served, not dropped to null"
    );
    assert!(
        totals["tokensUsed"].as_u64().unwrap() >= totals["tokensDrawn"].as_u64().unwrap(),
        "display volume adds the cache reads the ceiling figure omits"
    );
    assert_eq!(
        value["unavailable"],
        json!(["busy_sec", "tasks"]),
        "named gaps, never zero"
    );
}
