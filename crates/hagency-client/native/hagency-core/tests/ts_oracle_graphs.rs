//! TS test oracle — task graphs (task #65).
//!
//! Maps the retained `tests/api-task-graphs.test.js` observable outcomes onto
//! the native `hagency_core::graphs` planning model, asserting the SAME
//! outcome (cycle rejection, dependency-failure cascade, delete=cancel,
//! chained root-then-downstream dispatch).
use hagency_core::graphs::{Graph, GraphDefinition, NodeDefinition, NodeObservation, NodeStatus};

/// TS `buildChainGraph` (api-task-graphs.test.js:13): a → b, a+b → c.
fn chain_graph() -> GraphDefinition {
    let node = |id: &str, assignee: &str, deps: &[&str]| NodeDefinition {
        id: id.to_string(),
        assignee: assignee.to_string(),
        description: format!("Do {id}"),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        condition: None,
    };
    GraphDefinition {
        label: "chain graph".into(),
        nodes: vec![
            node("a", "alpha", &[]),
            node("b", "beta", &["a"]),
            node("c", "gamma", &["a", "b"]),
        ],
    }
}

/// TS `graph creation dispatches roots` (api-task-graphs.test.js:42): a chain
/// graph's roots (deps = []) dispatch first, downstream nodes only after their
/// dependencies complete.
#[test]
fn ts_oracle_graph_roots_dispatch_then_downstream() {
    let graph = Graph::new(chain_graph()).unwrap();
    let first = graph.advance().unwrap();
    // a is the only root; it dispatches now.
    assert_eq!(first.assignments.len(), 1);
    assert_eq!(first.assignments[0].node_id, "a");
    assert_eq!(first.graph.progress["a"].status, NodeStatus::Dispatched);
    // b and c are still pending — nothing upstream has completed.
    assert_eq!(first.graph.progress["b"].status, NodeStatus::Pending);
    assert_eq!(first.graph.progress["c"].status, NodeStatus::Pending);

    // Complete a, then advance: b dispatches; c still waits on b.
    let done_a = first
        .graph
        .observe(
            "a",
            &NodeObservation::Complete {
                result: serde_json::json!({}),
            },
        )
        .unwrap();
    let second = done_a.advance().unwrap();
    assert_eq!(second.assignments.len(), 1);
    assert_eq!(second.assignments[0].node_id, "b");
    assert_eq!(second.graph.progress["b"].status, NodeStatus::Dispatched);
    assert_eq!(second.graph.progress["c"].status, NodeStatus::Pending);
}

/// TS `rejects graph creation when dependencies contain a cycle`
/// (api-task-graphs.test.js:614). Native `GraphDefinition::validate` rejects a
/// cycle with the same observable class (Err, no graph created).
#[test]
fn ts_oracle_graph_rejects_dependency_cycle() {
    let mut def = chain_graph();
    // Introduce a cycle: a depends on c (while c already depends on a via b).
    def.nodes[0].depends_on = vec!["c".to_string()];
    let err = Graph::new(def).unwrap_err();
    assert_eq!(err.0, "graph dependency cycle");
}

/// TS `failed dependency cascades failure through remaining pending nodes`
/// (api-task-graphs.test.js:325): failing a marks every downstream node
/// failed, with the `dependency failed: …` error text.
#[test]
fn ts_oracle_graph_failed_dependency_cascades() {
    let graph = Graph::new(chain_graph()).unwrap();
    let first = graph.advance().unwrap(); // a dispatched
    // Fail a directly (it is Dispatched).
    let failed_a = first
        .graph
        .observe(
            "a",
            &NodeObservation::Failed {
                error: "A exploded".into(),
            },
        )
        .unwrap();
    let cascaded = failed_a.advance().unwrap();
    // b and c both cascade to failed.
    assert_eq!(cascaded.graph.progress["b"].status, NodeStatus::Failed);
    assert_eq!(cascaded.graph.progress["c"].status, NodeStatus::Failed);
    // TS asserts the error carries "dependency failed".
    let b_err = cascaded.graph.progress["b"].error.as_deref().unwrap();
    let c_err = cascaded.graph.progress["c"].error.as_deref().unwrap();
    assert!(b_err.contains("dependency failed"), "b error: {b_err}");
    assert!(c_err.contains("dependency failed"), "c error: {c_err}");
    // The whole graph is now failed.
    assert_eq!(
        serde_json::to_string(&cascaded.graph.status).unwrap(),
        "\"failed\""
    );
}

/// TS `delete cancels the graph and all non-terminal nodes`
/// (api-task-graphs.test.js:352). Native `Graph::cancel` cancels the graph and
/// every non-terminal node.
#[test]
fn ts_oracle_graph_delete_cancels_all_nonterminal() {
    let graph = Graph::new(chain_graph()).unwrap();
    let cancelled = graph.cancel().unwrap();
    assert_eq!(
        serde_json::to_string(&cancelled.status).unwrap(),
        "\"cancelled\""
    );
    for id in ["a", "b", "c"] {
        assert_eq!(
            cancelled.progress[id].status,
            NodeStatus::Cancelled,
            "node {id}"
        );
    }
}
