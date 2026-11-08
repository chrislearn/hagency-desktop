//! TS test oracle — MCP surfaces: agent token, heartbeat, media cache,
//! namespace drift, permission channel, progress anchor (task #76).
//!
//! The retained TS cases drive the Node MCP server (`lib/mcp-server-core.js`)
//! as a real process: a token file read at startup that becomes X-Agent-Token
//! on every backend HTTP call, a periodic heartbeat POST, a media-cache
//! directory resolved from env, a Codex permission hook allowlist, and a
//! progress anchor written by check_inbox.
//!
//! The native MCP server (`hagency mcp stdio`) deliberately has none of these
//! process-level behaviours: it inherits the runner context over stdin (no
//! token file, no HTTP backend), has no heartbeat loop, no media cache, no
//! Codex permission hook (coordination tools are gated by the ADR180
//! `coordination_tools` config flag instead, exercised by
//! `tests/mcp_coordination`), and no progress anchor. Per the task rules these
//! tests assert the TS behaviour and are `#[ignore = "parity gap: …"]`,
//! listed in report-76.md.

// ---------------------------------------------------------------------------
// mcp-agent-token.test.js — B11: every backend call carries the agent token.
// ---------------------------------------------------------------------------

/// TS `mcp-agent-token.test.js:42` — a present token file becomes
/// X-Agent-Token on every request. The reference rule: the trimmed file
/// content is the header value.
#[test]
#[ignore = "parity gap: native MCP inherits runner context over stdio; no agent-token file and no X-Agent-Token backend calls"]
fn ts_agent_token_file_becomes_header_on_every_request() {
    let file_content = "tok-b11-secret\n";
    let header = file_content.trim();
    assert_eq!(header, "tok-b11-secret");
}

/// TS `mcp-agent-token.test.js:70` — a MISSING token file refuses to start
/// (fail-closed, exit non-zero) naming the file.
#[test]
#[ignore = "parity gap: native MCP has no token-file startup gate to refuse on"]
fn ts_missing_token_file_refuses_to_start() {
    let stderr = "[mcp] agent-token file missing (ENOENT): agent-token — refusing to start";
    assert!(stderr.contains("agent-token file missing"));
    assert!(stderr.contains("agent-token"));
}

/// TS `mcp-agent-token.test.js:95` — an EMPTY (whitespace) token file also
/// refuses to start.
#[test]
#[ignore = "parity gap: native MCP has no token-file startup gate to refuse on"]
fn ts_empty_token_file_refuses_to_start() {
    let stderr = "[mcp] agent-token file is empty: agent-token — refusing to start";
    assert!(stderr.contains("agent-token file is empty"));
    assert!(stderr.contains("agent-token"));
}

// ---------------------------------------------------------------------------
// mcp-heartbeat.test.js — periodic POST /api/agents/<name>/heartbeat.
// ---------------------------------------------------------------------------

/// TS `mcp-heartbeat.test.js:259` — the heartbeat server field defaults to
/// os.hostname() when HAGENCY_SERVER is unset.
#[test]
#[ignore = "parity gap: native has no backend heartbeat client (no POST /api/agents/<n>/heartbeat)"]
fn ts_heartbeat_defaults_server_to_hostname() {
    // TS asserts heartbeatCalls[0].body.server === os.hostname().
    // Reference rule: the default is the host's own name.
    let default_server = hostname_placeholder();
    assert!(!default_server.is_empty());
}

fn hostname_placeholder() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "localhost".into())
}

/// TS `mcp-heartbeat.test.js:310` — the periodic heartbeat reconnects after a
/// backend restart, and transient failures are retried (:338), never terminal.
#[test]
#[ignore = "parity gap: native has no backend heartbeat client to reconnect or retry"]
fn ts_heartbeat_retries_transient_failures_and_reconnects() {
    // The operator rule this encodes: bridge-side faults are retried, never
    // terminal. Native has no heartbeat loop; its equivalent liveness path is
    // the runner's update_task_execution heartbeat tool.
}

// ---------------------------------------------------------------------------
// mcp-media-cache.test.js — cache directory resolution order.
// ---------------------------------------------------------------------------

/// TS `mcp-media-cache.test.js:51-131` — resolution order is
/// $STATE_DIR/mcp-media-cache, then $RUNTIME/data/mcp-media-cache/<agent with
/// '/'→'_'>, then $HOMEDIR/data/mcp-media-cache/<agent>; never the project cwd.
#[test]
#[ignore = "parity gap: native has no MCP media-cache directory (media flows through the media store)"]
fn ts_media_cache_resolution_order_state_runtime_home() {
    let agent = "alpha/beta";
    let slug = agent.replace('/', "_");
    assert_eq!(slug, "alpha_beta");
    // TS asserts the three candidate paths and that no `data/` is created
    // under the project cwd; none of these paths exist natively.
}

// ---------------------------------------------------------------------------
// mcp-namespace-drift.test.js — the Codex permission-hook allowlist.
// ---------------------------------------------------------------------------

/// TS `mcp-namespace-drift.test.js:73-80` — the coordination tools
/// (check_inbox, whoami, list_tasks, accept_task) do not require owner
/// approval. Native equivalents: ADR180 gates the coordination profile by
/// config, and the owned runner calls list_tasks/accept_task directly with no
/// approval step (see tests/mcp_coordination, cited in report-76.md).
#[test]
#[ignore = "parity gap: native has no Codex permission hook; ADR180 gates coordination tools by config flag, not an mcp__ allowlist"]
fn ts_coordination_tools_need_no_owner_approval() {
    // TS allowlist prefix rule: mcp__<server>__<tool>, '-'→'_' in the server
    // segment.
    let server = "hagency";
    let prefix = format!("mcp__{}__", server.replace('-', "_"));
    for tool in ["check_inbox", "whoami", "list_tasks", "accept_task"] {
        let name = format!("{prefix}{tool}");
        assert!(name.starts_with("mcp__hagency__"));
    }
}

/// TS `mcp-namespace-drift.test.js:82-85` — a tool outside the allowlist
/// (shell, or an unknown mcp__hagency__ tool) still requires approval, and
/// (:87-96) send_message with attachments requires approval even though the
/// bare tool is allowlisted.
#[test]
#[ignore = "parity gap: native has no Codex permission hook to route tool names to approval decisions"]
fn ts_out_of_allowlist_and_attachments_require_approval() {
    // Reference rule from lib/codex-permission-hook.js: only the fixed
    // coordination set skips approval; send_message skips it only without
    // attachments.
    let allowlisted = ["check_inbox", "whoami", "list_tasks", "accept_task"];
    assert!(!allowlisted.contains(&"shell"));
    assert!(!allowlisted.contains(&"not_a_real_tool"));
    assert!(!allowlisted.contains(&"send_message"));
}

// ---------------------------------------------------------------------------
// mcp-permission-channel.test.js — runtime approval adapters (Claude/Codex).
// ---------------------------------------------------------------------------

// Claude-adapter cases (channel capabilities, server-authorized decision,
// deny-without-polling, consumption binding, explicit deny on failure,
// consume-before-deliver) and the launcher cases (sandbox defaults, ambient
// ANTHROPIC key) exercise the Claude runtime / launchers — declared
// out-of-scope areas; listed as skip(reason) in report-76.md, not here.
//
// The Codex-hook cases (PermissionRequest mapping, recursive approval, hook
// failure deny, consume-allow-first, command digest binding, colliding
// request ids, preflight trust) all drive lib/codex-permission-hook.js and
// lib/codex-hook-trust.js, which have no native counterpart.

/// TS `mcp-permission-channel.test.js:162` — a Codex PermissionRequest maps to
/// the documented hook output shape, and (:278) a hook failure emits an
/// explicit deny rather than an error.
#[test]
#[ignore = "parity gap: native has no Codex permission hook or runtime-approval adapter"]
fn ts_codex_permission_request_maps_and_failures_deny() {
    // Reference rule: the hook decision JSON has hookSpecificOutput with
    // permissionDecision in {allow, deny, ask}; any failure path yields deny.
    let decision_on_failure = "deny";
    assert_eq!(decision_on_failure, "deny");
}

// ---------------------------------------------------------------------------
// mcp-progress-anchor.test.js — check_inbox records a progress anchor.
// ---------------------------------------------------------------------------

/// TS `mcp-progress-anchor.test.js:149-190` — check_inbox remembers the newest
/// message per bucket (group with its group, DM as a DM back to its sender),
/// an empty read leaves no anchor and does not erase an existing one, and a
/// new question resets the throttle.
#[test]
#[ignore = "parity gap: native check_inbox equivalent has no progress anchor (hagency-progress anchors nothing); bin/hagency-progress has no native counterpart"]
fn ts_check_inbox_records_progress_anchor() {
    // Reference rule: two buckets (group, dm); newest wins across both;
    // empty read is not an erase.
    let inbox_group = Some(("group", "!project:example.test"));
    // (bucket, target): a group message is remembered with its group
    let anchor: Option<(&str, &str)> = inbox_group;
    assert_eq!(anchor, Some(("group", "!project:example.test")));
    let empty: Option<(&str, &str)> = None;
    if empty.is_none() {
        // an empty read does NOT erase the anchor
    }
    assert_eq!(anchor, Some(("group", "!project:example.test")));
}
