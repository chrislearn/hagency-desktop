//! Operator task graphs (board #47) — TS parity with the retained
//! `lib/task-graph.js` routes (`backend-v2.js:15974-16020`) and its oracle
//! (`tests/api-task-graphs.test.js`). Each test asserts the TS-visible
//! outcome: the route's JSON, the persisted `task_graphs.json` document, and
//! the durable `task_graph_dispatch` message a create produces.
use super::*;

fn graph_body() -> Value {
    json!({
        "owner": "operator",
        "label": "chain graph",
        "nodes": {
            "a": {"id": "a", "assignee": "alpha", "description": "Do a"},
            "b": {"id": "b", "assignee": "beta", "description": "Do b", "depends_on": ["a"]},
            "c": {"id": "c", "assignee": "gamma", "description": "Do c", "depends_on": ["a", "b"]}
        }
    })
}

/// TS `graph creation dispatches roots and chained completion dispatches
/// downstream nodes` (api-task-graphs.test.js:42): the create returns
/// `{ok:true, graph}` with the root `dispatched`, downstream `pending`, and
/// persists exactly one `task_graph_dispatch` message addressed to the root's
/// assignee.
#[tokio::test]
async fn native_console_task_graph_create_dispatches_roots() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = post("/console/api/task-graphs", &cookie)
        .json(&graph_body())
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    let graph = &value["graph"];
    assert_eq!(graph["status"], json!("active"));
    assert_eq!(graph["owner"], json!("operator"));
    assert_eq!(graph["label"], json!("chain graph"));
    // The root dispatched; b and c wait on it.
    assert_eq!(graph["nodes"]["a"]["status"], json!("dispatched"));
    assert_eq!(graph["nodes"]["b"]["status"], json!("pending"));
    assert_eq!(graph["nodes"]["c"]["status"], json!("pending"));
    assert!(
        graph["nodes"]["a"]["message_id"]
            .as_str()
            .unwrap()
            .starts_with("msg_")
    );
    let graph_id = graph["id"].as_str().unwrap().to_owned();

    // The durable dispatch message landed (oracle :58-64).
    let messages: Value =
        serde_json::from_slice(&std::fs::read(f.root.path().join("state/messages.json")).unwrap())
            .unwrap();
    let rows = messages.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["schema"]["kind"], json!("task_graph_dispatch"));
    assert_eq!(rows[0]["schema"]["version"], json!(1));
    assert_eq!(rows[0]["schema"]["payload"]["graphId"], json!(graph_id));
    assert_eq!(rows[0]["schema"]["payload"]["nodeId"], json!("a"));
    assert_eq!(rows[0]["to"], json!("alpha"));
    assert_eq!(rows[0]["type"], json!("request"));
    assert_eq!(rows[0]["priority"], json!("high"));
    assert_eq!(rows[0]["summary"], json!("Task assigned: Do a"));

    // And the document persisted (TS `task_graphs.json`).
    let persisted: Value = serde_json::from_slice(
        &std::fs::read(f.root.path().join("state/task_graphs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted[&graph_id]["nodes"]["a"]["status"],
        json!("dispatched")
    );

    f.close().await;
}

/// TS routes (backend-v2.js:15984-15997): list answers the graphs array
/// (newest `updatedAt` first, optional `?status=` filter), read answers the
/// bare graph or 404 `task graph not found`.
#[tokio::test]
async fn native_console_task_graph_list_and_read() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let created = post("/console/api/task-graphs", &cookie)
        .json(&graph_body())
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    let graph_id = created["graph"]["id"].as_str().unwrap().to_owned();

    let mut list = get("/console/api/task-graphs", &cookie)
        .send(&service)
        .await;
    assert_eq!(list.status_code, Some(StatusCode::OK));
    let graphs = list.take_json::<Value>().await.unwrap();
    assert_eq!(graphs.as_array().unwrap().len(), 1);
    assert_eq!(graphs[0]["id"], json!(graph_id));

    // The status filter narrows the list (TS `listGraphs({status})`).
    let active = get("/console/api/task-graphs?status=active", &cookie)
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    assert_eq!(active.as_array().unwrap().len(), 1);
    let complete = get("/console/api/task-graphs?status=complete", &cookie)
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    assert_eq!(complete.as_array().unwrap().len(), 0);
    // An unknown status is the TS `invalid_graph_status` 400.
    let invalid = get("/console/api/task-graphs?status=nope", &cookie)
        .send(&service)
        .await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));

    let mut read = get(&format!("/console/api/task-graphs/{graph_id}"), &cookie)
        .send(&service)
        .await;
    assert_eq!(read.status_code, Some(StatusCode::OK));
    assert_eq!(
        read.take_json::<Value>().await.unwrap()["label"],
        json!("chain graph")
    );
    let missing = get("/console/api/task-graphs/graph_none", &cookie)
        .send(&service)
        .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));

    f.close().await;
}

/// TS `delete cancels the graph and all non-terminal nodes`
/// (api-task-graphs.test.js:352): DELETE answers `{ok:true, graph}` with the
/// graph and every node `cancelled`; the row remains readable afterwards.
#[tokio::test]
async fn native_console_task_graph_delete_cancels() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let created = post("/console/api/task-graphs", &cookie)
        .json(&graph_body())
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    let graph_id = created["graph"]["id"].as_str().unwrap().to_owned();

    let mut deleted = delete(&format!("/console/api/task-graphs/{graph_id}"), &cookie)
        .send(&service)
        .await;
    assert_eq!(deleted.status_code, Some(StatusCode::OK));
    let value = deleted.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["graph"]["status"], json!("cancelled"));
    for node in ["a", "b", "c"] {
        assert_eq!(
            value["graph"]["nodes"][node]["status"],
            json!("cancelled"),
            "node {node}"
        );
    }
    // A cancelled graph is still served (TS keeps the row).
    let read = get(&format!("/console/api/task-graphs/{graph_id}"), &cookie)
        .send(&service)
        .await;
    assert_eq!(read.status_code, Some(StatusCode::OK));
    let missing = delete("/console/api/task-graphs/graph_none", &cookie)
        .send(&service)
        .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));

    f.close().await;
}

/// TS `graph creation dispatches roots and chained completion dispatches
/// downstream nodes` (api-task-graphs.test.js:42): a node PATCH marks the
/// root complete, and the follow-up advance dispatches the next node.
#[tokio::test]
async fn native_console_task_graph_node_update_advances() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let created = post("/console/api/task-graphs", &cookie)
        .json(&graph_body())
        .send(&service)
        .await
        .take_json::<Value>()
        .await
        .unwrap();
    let graph_id = created["graph"]["id"].as_str().unwrap().to_owned();

    // Complete a; the route answers {ok, graph, node} and b dispatches.
    let mut patched = patch(
        &format!("/console/api/task-graphs/{graph_id}/nodes/a"),
        &cookie,
    )
    .json(&json!({"status": "complete", "result": {"answer": 42}}))
    .send(&service)
    .await;
    assert_eq!(patched.status_code, Some(StatusCode::OK));
    let value = patched.take_json::<Value>().await.unwrap();
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["node"]["status"], json!("complete"));
    assert_eq!(value["graph"]["nodes"]["a"]["status"], json!("complete"));
    assert_eq!(value["graph"]["nodes"]["b"]["status"], json!("dispatched"));
    assert_eq!(value["graph"]["nodes"]["c"]["status"], json!("pending"));

    // A second dispatch message landed for b.
    let messages: Value =
        serde_json::from_slice(&std::fs::read(f.root.path().join("state/messages.json")).unwrap())
            .unwrap();
    let rows = messages.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["schema"]["payload"]["nodeId"], json!("b"));

    // A patch without status/result/error is the TS `invalid_patch` 400.
    let invalid = patch(
        &format!("/console/api/task-graphs/{graph_id}/nodes/a"),
        &cookie,
    )
    .json(&json!({"note": "nothing"}))
    .send(&service)
    .await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));

    f.close().await;
}

/// TS rejects a dependency cycle (api-task-graphs.test.js:614) with the
/// `invalid_dependency_cycle` 400, and a node missing its assignee with
/// `invalid_node_assignee`.
#[tokio::test]
async fn native_console_task_graph_rejects_invalid_input() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut body = graph_body();
    body["nodes"]["a"]["depends_on"] = json!(["c"]);
    let cycle = post("/console/api/task-graphs", &cookie)
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(cycle.status_code, Some(StatusCode::BAD_REQUEST));

    let mut missing_assignee = graph_body();
    missing_assignee["nodes"]["a"]
        .as_object_mut()
        .unwrap()
        .remove("assignee");
    let refused = post("/console/api/task-graphs", &cookie)
        .json(&missing_assignee)
        .send(&service)
        .await;
    assert_eq!(refused.status_code, Some(StatusCode::BAD_REQUEST));

    // Anonymous is refused before any store work.
    let anonymous = TestClient::get(format!("{BASE}/console/api/task-graphs"))
        .add_header("host", "127.0.0.1:13300", true)
        .send(&service)
        .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));

    f.close().await;
}
