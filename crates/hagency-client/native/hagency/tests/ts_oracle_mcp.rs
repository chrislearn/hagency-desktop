//! TS oracle: `tests/router-codex-mcp-approval.test.js` (the Codex native MCP
//! approval adapter).
//!
//! Ported into the SERVICE crate, whose MCP surface owns this behaviour:
//! `hagency::mcp::Session` (`native/hagency/src/mcp.rs`) and the task-bound
//! approval pair (`task_client::approval::{READ, CONSUME}`). The TS adapter is
//! the retained JS driver; native's equivalent is the MCP session the runner
//! speaks, with the same closed schema, the same refusal words and the same
//! capability-derived (never id-named) approval.
use hagency::{mcp::Session, task_client::Context};
use hagency_core::tasks::RunnerCapability;
use serde_json::{Value, json};

fn capability() -> RunnerCapability {
    RunnerCapability {
        dispatch_id: "dispatch".into(),
        runner_id: "runner".into(),
        fence: 1,
        secret: "a".repeat(64),
    }
}

fn session() -> Session {
    Session::new(Context::new("127.0.0.1:9".parse().unwrap(), capability(), "task".into()).unwrap())
}

fn init(id: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})
}

async fn request(s: &mut Session, v: Value) -> Option<Value> {
    s.handle(&serde_json::to_vec(&v).unwrap()).await.unwrap()
}

async fn ready() -> Session {
    let mut s = session();
    request(&mut s, init(json!("1"))).await.unwrap();
    assert!(
        request(
            &mut s,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await
        .is_none()
    );
    s
}

async fn call(s: &mut Session, id: u64, name: &str, arguments: Value) -> Value {
    request(
        s,
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
               "params":{"name":name,"arguments":arguments}}),
    )
    .await
    .unwrap()
}

/// TS `router-codex-mcp-approval.test.js:61` — *native MCP owner approval binds
/// actual item request and digest before native response*. The native binding is
/// structural: the approval tools accept ONLY the assigned task id (the
/// capability's task), never an approval id, owner, room, choice or action — so
/// there is no unbound field to forge. Derived from `mcp.rs:428-437`.
#[tokio::test]
async fn ts_mcp_approval_tools_bind_the_task_and_nothing_else() {
    let mut s = ready().await;
    // A read carrying any extra field is refused by name, before dispatch.
    // Every MCP request carries a UNIQUE id: the session refuses a reused one.
    let mut id = 2u64;
    for extra in [
        json!({"id":"task","choice":"always"}),
        json!({"id":"task","action":"approve"}),
        json!({"id":"task","approval_id":"approval_x"}),
        json!({"id":"task","owner_mxid":"@owner:example.test"}),
    ] {
        let refused = call(&mut s, id, "get_approval", extra).await;
        id += 1;
        assert_eq!(refused["result"]["isError"], json!(true));
        assert!(
            refused["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Read tools take the assigned task id only"),
            "an extra field on the read is refused by name: {refused}"
        );
    }
    // The mutation without its stable call_id is refused by name.
    let bare = call(&mut s, id, "consume_approval", json!({"id":"task"})).await;
    assert_eq!(bare["result"]["isError"], json!(true));
    assert!(
        bare["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Missing stable call_id"),
        "the mutation names the missing call_id: {bare}"
    );
}

/// TS `router-codex-mcp-approval.test.js:51` — *native Hagency coordination uses
/// the existing exact predicate and accepts only once*. The catalog is exact:
/// `get_approval` declares `id` alone, `consume_approval` declares `id` + a
/// `call_id`, and both close `additionalProperties`. Asserted green by
/// `tests/mcp.rs::native_mcp_approval_tools_are_catalogued_and_bounded` in this
/// crate — this case pins the same schema from the oracle's point of view.
#[tokio::test]
async fn ts_mcp_approval_catalog_is_exact_and_closed() {
    let mut s = ready().await;
    let catalog = request(
        &mut s,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await
    .unwrap();
    let tools = catalog["result"]["tools"].as_array().unwrap();
    let get = tools
        .iter()
        .find(|t| t["name"] == "get_approval")
        .expect("get_approval is catalogued");
    let consume = tools
        .iter()
        .find(|t| t["name"] == "consume_approval")
        .expect("consume_approval is catalogued");
    assert_eq!(
        get["inputSchema"]["properties"].as_object().unwrap().len(),
        1
    );
    assert!(get["inputSchema"]["properties"].get("id").is_some());
    assert_eq!(get["inputSchema"]["additionalProperties"], json!(false));
    assert_eq!(
        consume["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert!(
        consume["inputSchema"]["properties"]
            .get("call_id")
            .is_some()
    );
    assert_eq!(consume["inputSchema"]["additionalProperties"], json!(false));
    // Neither tool declares an approval id, owner, room, choice or action.
    for tool in [get, consume] {
        for key in ["approval_id", "owner_mxid", "room_id", "choice", "action"] {
            assert!(
                tool["inputSchema"]["properties"].get(key).is_none(),
                "the {key} key is not declared"
            );
        }
    }
}

/// TS `router-codex-mcp-approval.test.js:139` — *unknown server RPC receives a
/// protocol error and visible outcome_unknown*. An unknown MCP method is refused
/// with the JSON-RPC method error, never accepted. Derived from `mcp.rs:242-246`.
#[tokio::test]
async fn ts_mcp_unknown_method_is_a_protocol_error() {
    let mut s = ready().await;
    let response = request(
        &mut s,
        json!({"jsonrpc":"2.0","id":9,"method":"no/such/method","params":{}}),
    )
    .await
    .unwrap();
    assert_eq!(response["error"]["code"], json!(-32601));
    assert_eq!(
        response["error"]["message"],
        json!("Method is unavailable in this MCP lifecycle")
    );
    assert!(response.get("result").is_none(), "no result is invented");
}

/// TS `router-codex-mcp-approval.test.js:122` — *rejects URL input-form malformed
/// and uncorrelated elicitation requests*. An unknown tool name, or a
/// `tools/call` carrying an undeclared field, is refused with the invalid-params
/// code rather than dispatched. Derived from `mcp.rs:225-239`.
#[tokio::test]
async fn ts_mcp_unknown_tool_and_malformed_call_are_refused() {
    let mut s = ready().await;
    // An unknown tool name is refused before any dispatch.
    let unknown = request(
        &mut s,
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call",
               "params":{"name":"delete_project","arguments":{}}}),
    )
    .await
    .unwrap();
    assert_eq!(unknown["error"]["code"], json!(-32602));
    assert_eq!(
        unknown["error"]["message"],
        json!("Unknown tool or invalid tool request schema")
    );
    // An undeclared top-level field on the call envelope is refused too.
    let malformed = request(
        &mut s,
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call",
               "params":{"name":"get_approval","arguments":{"id":"task"},"extra":true}}),
    )
    .await
    .unwrap();
    assert_eq!(malformed["error"]["code"], json!(-32602));
}

/// TS `router-codex-mcp-approval.test.js:146,156,170,188` — *malformed or absent
/// native request IDs fail visibly without accepting*, *owner approval rejection
/// and persistence refusal cancel without accepting*, *a late owner allow after
/// duplicate or unknown RPC cannot resume or accept*, *an item completed or
/// restarted while owner approval waits cannot receive a late allow*. These are
/// the retained Codex adapter's elicitation/`owner`-callback state machine — an
/// in-process JS driver with no native counterpart: native's MCP session is a
/// pure request/response surface with no elicitation channel, and its
/// at-most-once rule lives in the store's settled-state machine (asserted by
/// `ts_approval_consume_is_at_most_once`).
#[test]
#[ignore = "parity gap: the Codex adapter's elicitation/late-allow state machine is an in-process JS driver; native's MCP session has no elicitation channel and its at-most-once lives in the store"]
fn ts_codex_elicitation_and_late_allow_state_machine() {}

/// TS `router-codex-mcp-approval.test.js:86` — *missing injected coordination
/// policy keeps even get_task owner gated*. Native's coordination policy is
/// fixed at session construction (the runner profile), not injected per call, so
/// there is no "missing injection" state to enter; the owner-gating of a locked
/// task is asserted by the store's approval suites.
#[test]
#[ignore = "parity gap: native's coordination policy is fixed at session construction (no per-call injection), so the missing-injection state has no native counterpart"]
fn ts_missing_coordination_policy_keeps_get_task_gated() {}

/// TS `router-codex-mcp-approval.test.js:93` — *rejects missing ambiguous stale
/// wrong-turn and mismatched-argument MCP candidates*. The candidate-correlation
/// logic (thread/turn/item matching, `__proto__` argument hygiene) is the Codex
/// adapter's own; native correlates by the authenticated capability, not by
/// candidate matching, so there is no candidate set to disambiguate.
#[test]
#[ignore = "parity gap: native correlates the approval by the authenticated capability, not by candidate matching; the Codex candidate-correlation logic has no native counterpart"]
fn ts_mcp_candidate_correlation_refusals() {}

/// TS `router-codex-mcp-approval.test.js:111,205` — *duplicate elicitation cannot
/// consume the same tool item twice* / *operation digest binds all MCP arguments
/// without depending on display order*. The at-most-once half IS native — the
/// store's settled-state machine, asserted by
/// `hagency-store/tests/ts_oracle_approvals.rs::ts_approval_consume_is_at_most_once`.
/// The `operationDigest` (a JS-side canonical digest over the MCP arguments) has
/// no native counterpart: native's canonical digest is over the request record.
#[test]
#[ignore = "covered: at-most-once consume is asserted by hagency-store tests/ts_oracle_approvals.rs; the JS operationDigest has no native counterpart"]
fn ts_mcp_duplicate_consume_and_operation_digest() {}
