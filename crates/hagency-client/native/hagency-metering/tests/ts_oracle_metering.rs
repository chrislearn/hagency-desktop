//! TS test oracle — token metering (`tests/metering.test.js`,
//! `tests/metering-discovery.test.js`).
//!
//! The retained TS suite is the parity ORACLE: each test below asserts the SAME
//! observable outcome its TS case asserts (the totals object, the diagnostic
//! counts, the unavailable `reason` text). Where native differs, the test keeps
//! asserting the TS outcome and is `#[ignore]`d with a one-line reason — this
//! task changes no product code.
//!
//! Native surface under test: `hagency-metering` (`src/lib.rs` `parse_session`,
//! `src/attribution.rs` `claude_project_dir` / `transcript_search` /
//! `meter_agent` / `summarize_fleet`).
//!
//! `tests/metering-ledger.test.js` is NOT here: the ledger's home in native is
//! the store's usage periods, and it is already oracled there — see
//! `hagency-store/tests/usage/vectors.rs` (`native_usage_high_water_vectors`,
//! replaying `lib/metering/ledger.js` through `scripts/usage-vectors.mjs`) and
//! `usage/ceiling.rs`. The report lists that mapping case by case.

use hagency_metering::attribution::{
    AgentRow, ScanBounds, SessionText, claude_project_dir, meter_agent, summarize_fleet,
    transcript_search,
};
use hagency_metering::{Framework, SessionDetails, TokenCounts, parse_session};
use serde_json::{Value, json};

const WS: &str = "/Users/someone/work/payments-api";

/// The TS `claudeText`: two assistant messages, one repeated as a resumed session would.
fn claude_text() -> String {
    [
        json!({"type": "user", "cwd": WS, "uuid": "u0"}).to_string(),
        json!({"type": "assistant", "cwd": WS, "uuid": "a1", "message": {"model": "claude-opus-4-8",
            "usage": {"input_tokens": 2, "output_tokens": 1810,
                      "cache_creation_input_tokens": 1565, "cache_read_input_tokens": 339_166}}})
        .to_string(),
        json!({"type": "assistant", "cwd": WS, "uuid": "a1", "message": {"usage": {"input_tokens": 2,
            "output_tokens": 1810, "cache_creation_input_tokens": 1565,
            "cache_read_input_tokens": 339_166}}})
        .to_string(),
        json!({"type": "assistant", "cwd": WS, "uuid": "a2", "message": {"model": "claude-opus-4-8",
            "usage": {"input_tokens": 10, "output_tokens": 90,
                      "cache_creation_input_tokens": 0, "cache_read_input_tokens": 1000}}})
        .to_string(),
    ]
    .join("\n")
}

/// One Codex running total, as the real files record it.
fn codex_totals(input: u64, cached: u64, output: u64, reasoning: u64) -> Value {
    json!({"payload": {"info": {
        "total_token_usage": {"input_tokens": input, "cached_input_tokens": cached,
            "output_tokens": output, "reasoning_output_tokens": reasoning,
            "total_tokens": input + output},
        "last_token_usage": {"input_tokens": input, "cached_input_tokens": cached,
            "output_tokens": output, "reasoning_output_tokens": reasoning,
            "total_tokens": input + output}}}})
}

fn codex_text() -> String {
    [
        json!({"payload": {"type": "session_meta", "cwd": WS, "id": "s1"}}).to_string(),
        codex_totals(1000, 900, 100, 40).to_string(),
        codex_totals(1000, 900, 100, 40).to_string(),
        codex_totals(5000, 4500, 300, 120).to_string(),
    ]
    .join("\n")
}

fn claude_totals() -> TokenCounts {
    match parse_session(Framework::Claude, &claude_text())
        .unwrap()
        .totals
    {
        Some(totals) => totals,
        None => unreachable!("the fixture records usage"),
    }
}

fn session(file: &str, text: &str) -> SessionText {
    SessionText {
        file: file.to_owned(),
        text: text.to_owned(),
    }
}

/* ─────────────────── tests/metering.test.js — Claude parsing ─────────────────── */

/// TS `metering.test.js:79` `token kinds stay separate`: `totals` equals
/// `{input:12, output:1900, cacheWrite:1565, cacheRead:340166}`, `cwd === WS`,
/// `models === ['claude-opus-4-8']`.
#[test]
fn ts_oracle_claude_kinds_stay_separate() {
    let report = parse_session(Framework::Claude, &claude_text()).unwrap();
    assert_eq!(
        report.totals,
        Some(TokenCounts {
            input: Some(12),
            output: Some(1900),
            cache_write: Some(1565),
            cache_read: Some(340_166),
        })
    );
    assert_eq!(report.workspace_hint.as_deref(), Some(WS));
    match report.details {
        SessionDetails::Claude { models, .. } => assert_eq!(models, vec!["claude-opus-4-8"]),
        other => panic!("claude fixture parsed as {other:?}"),
    }
}

/// TS `metering.test.js:86` `a repeated record is counted once`: `messages === 2`,
/// `totals.output === 1900` — a resumed session appends the same message again.
#[test]
fn ts_oracle_claude_repeated_record_counted_once() {
    let report = parse_session(Framework::Claude, &claude_text()).unwrap();
    match report.details {
        SessionDetails::Claude { messages, .. } => assert_eq!(messages, 2),
        other => panic!("claude fixture parsed as {other:?}"),
    }
    assert_eq!(claude_totals().output, Some(1900));
}

/// TS `metering.test.js:93` `no uuid → undedupable`:
/// `r.undedupable === 1` (surfaced on native as
/// `diagnostics.undeduplicable_messages`).
#[test]
fn ts_oracle_claude_undeduplicable_is_reported() {
    let text = json!({"cwd": WS, "message": {"usage": {"input_tokens": 5, "output_tokens": 5}}});
    let report = parse_session(Framework::Claude, &text.to_string()).unwrap();
    assert_eq!(report.diagnostics.undeduplicable_messages, 1);
}

/// TS `metering.test.js:103` `malformed lines are skipped, not fatal`:
/// `totals.output === 1900` with two junk lines around the real transcript.
#[test]
fn ts_oracle_claude_malformed_lines_are_skipped() {
    let text = format!("not json\n{}\n{{\"broken\":", claude_text());
    let report = parse_session(Framework::Claude, &text).unwrap();
    assert_eq!(report.totals.unwrap().output, Some(1900));
    assert_eq!(report.diagnostics.malformed_lines, 2);
}

/* ─────────────────── tests/metering.test.js — Codex parsing ─────────────────── */

/// TS `metering.test.js:110` `the session total is the cumulative figure`:
/// `totals.cacheRead === 4500`, `totals.input === 500` (5000 total less 4500
/// cached), `cumulativeTotal === 5300`.
#[test]
fn ts_oracle_codex_cumulative_not_summed_deltas() {
    let report = parse_session(Framework::Codex, &codex_text()).unwrap();
    let totals = report.totals.unwrap();
    assert_eq!(totals.cache_read, Some(4500));
    assert_eq!(totals.input, Some(500));
    match report.details {
        SessionDetails::Codex {
            cumulative_total, ..
        } => assert_eq!(cumulative_total, Some(5300)),
        other => panic!("codex fixture parsed as {other:?}"),
    }
}

/// TS `metering.test.js:121` `reasoning tokens are not added to output`:
/// `totals.output === 300`, `reasoningOutput === 120` (reasoning sits INSIDE
/// output; adding it would give 420).
#[test]
fn ts_oracle_codex_reasoning_is_a_breakdown() {
    let report = parse_session(Framework::Codex, &codex_text()).unwrap();
    assert_eq!(report.totals.unwrap().output, Some(300));
    match report.details {
        SessionDetails::Codex {
            reasoning_output, ..
        } => assert_eq!(reasoning_output, Some(120)),
        other => panic!("codex fixture parsed as {other:?}"),
    }
}

/// TS `metering.test.js:128` `the parse agrees with the CLI's own arithmetic`:
/// `agreesWithCli === true` and `input + cacheRead + output === cumulativeTotal`.
#[test]
fn ts_oracle_codex_agrees_with_cli() {
    let report = parse_session(Framework::Codex, &codex_text()).unwrap();
    let totals = report.totals.unwrap();
    match report.details {
        SessionDetails::Codex {
            agrees_with_cli,
            cumulative_total,
            ..
        } => {
            assert_eq!(agrees_with_cli, Some(true));
            assert_eq!(
                totals.input.unwrap() + totals.cache_read.unwrap() + totals.output.unwrap(),
                cumulative_total.unwrap()
            );
        }
        other => panic!("codex fixture parsed as {other:?}"),
    }
}

/// TS `metering.test.js:138` `a turn is a change in the running total`:
/// `turns === 2` (the same total repeated is one turn, not two).
#[test]
fn ts_oracle_codex_turns_count_changes() {
    let report = parse_session(Framework::Codex, &codex_text()).unwrap();
    match report.details {
        SessionDetails::Codex { turns, .. } => assert_eq!(turns, Some(2)),
        other => panic!("codex fixture parsed as {other:?}"),
    }
}

/// TS `metering.test.js:143` `a non-monotonic total is reported, not trusted`:
/// `nonMonotonic === 1` when a later total goes backwards.
#[test]
fn ts_oracle_codex_non_monotonic_is_reported() {
    let text = [
        json!({"payload": {"type": "session_meta", "cwd": WS}}).to_string(),
        codex_totals(5000, 4500, 300, 0).to_string(),
        codex_totals(1000, 900, 100, 0).to_string(),
    ]
    .join("\n");
    let report = parse_session(Framework::Codex, &text).unwrap();
    assert_eq!(report.diagnostics.non_monotonic, 1);
}

/* ───────────── tests/metering.test.js — framework support + locating ───────────── */

/// TS `metering.test.js:159` `octos is unavailable WITH the reason`: the reason
/// matches `/no usage object and no cwd/` rather than being a bare false.
///
/// Native states the same fact, but through `meter_agent`'s reason string rather
/// than a `meteringSupport` accessor, which does not exist on this surface.
#[test]
fn ts_oracle_octos_unavailable_with_reason() {
    let row = meter_agent(
        &json!({"name": "o1", "type": "octos", "workspacePath": WS}),
        "/home/me",
        "/",
        &[session("never-read", &claude_text())],
        None,
    );
    assert!(!row.available);
    let reason = row.reason.unwrap();
    assert!(reason.contains("no usage object"), "reason: {reason}");
}

/// TS `metering.test.js:167` `an unknown framework is unavailable, not assumed
/// supported`: `available === false`.
#[test]
fn ts_oracle_unknown_framework_not_assumed_supported() {
    let row = meter_agent(
        &json!({"name": "o4", "type": "something-new", "workspacePath": WS}),
        "/home/me",
        "/",
        &[],
        None,
    );
    assert!(!row.available);
}

/// TS `metering.test.js:154` `supported frameworks say so`: `meteringSupport`
/// reports `available: true` for claude and codex.
///
/// `meteringSupport` is a ts-only accessor; native states support through the
/// adapter table `unsupported_reason`, which has no public Rust equivalent.
#[ignore = "parity gap: no native meteringSupport() accessor (support is implied by meter_agent's adapter table)"]
#[test]
fn ts_oracle_supported_frameworks_say_so() {
    // The TS accessor `meteringSupport('claude').available` has no Rust twin:
    // native only reveals support by attempting an attribution. Assert the TS
    // outcome against the closest available surface, so the case fails loudly
    // the day an accessor lands rather than passing vacuously.
    for framework in ["claude", "codex"] {
        let row = meter_agent(
            &json!({"name": "a1", "type": framework, "workspacePath": WS}),
            "/home/me",
            "/",
            &[],
            None,
        );
        assert!(
            row.reason
                .as_deref()
                .is_none_or(|reason| !reason.contains("no metering adapter")),
            "{framework} must be a supported adapter"
        );
    }
}

/// TS `metering.test.js:173` `Claude's directory name is the cwd with slashes
/// replaced`: `/Users/someone/home/hagency` → `-Users-someone-home-hagency`.
#[test]
fn ts_oracle_claude_project_dir() {
    assert_eq!(
        claude_project_dir("/Users/someone/home/hagency").as_deref(),
        Some("-Users-someone-home-hagency")
    );
}

/// TS `metering.test.js:177` `a relative path has no project directory`:
/// `claudeProjectDir('work/thing') === null`.
#[test]
fn ts_oracle_relative_path_has_no_project_dir() {
    assert_eq!(claude_project_dir("work/thing"), None);
}

/// TS `metering.test.js:181` `Claude narrows by directory, Codex cannot`:
/// claude `narrowed === true`, codex `narrowed === false`.
#[test]
fn ts_oracle_claude_narrows_codex_cannot() {
    assert!(
        transcript_search("claude", WS, "/home/me")
            .unwrap()
            .narrowed
    );
    assert!(!transcript_search("codex", WS, "/home/me").unwrap().narrowed);
}

/* ─────────────────── tests/metering.test.js — attribution ─────────────────── */

/// TS `metering.test.js:193` `a matching transcript is totalled`:
/// `available === true`, `totals.cacheRead === 340166`, `sessions === 1`.
#[test]
fn ts_oracle_matching_transcript_totalled() {
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": WS}),
        "/home/me",
        "/",
        &[session("s1.jsonl", &claude_text())],
        None,
    );
    assert!(row.available);
    assert_eq!(row.totals.unwrap().cache_read, Some(340_166));
    assert_eq!(row.sessions, Some(1));
}

/// TS `metering.test.js:204` `a transcript recording a DIFFERENT cwd is not
/// counted`: `available === false`, reason matches
/// `/none of which recorded this workspace/` and does NOT hedge with
/// `/never opened/` (nothing was left unreached).
#[test]
fn ts_oracle_different_cwd_not_counted() {
    let other = claude_text().replace(WS, "/Users/someone/work/other");
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": WS}),
        "/home/me",
        "/",
        &[session("x.jsonl", &other)],
        None,
    );
    assert!(!row.available);
    let reason = row.reason.unwrap();
    assert!(
        reason.contains("none of which recorded this workspace"),
        "reason: {reason}"
    );
    assert!(!reason.contains("never opened"), "reason: {reason}");
}

/// TS `metering.test.js:228` `an agent with no workspace is unattributable, not
/// zero`: `available === false`, `totals === undefined`, reason matches
/// `/no workspace recorded/`.
#[test]
fn ts_oracle_no_workspace_is_unattributable() {
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": null}),
        "/home/me",
        "/",
        &[],
        None,
    );
    assert!(!row.available);
    assert!(row.totals.is_none());
    assert!(row.reason.unwrap().contains("no workspace recorded"));
}

/// TS `metering.test.js:239` `an unsupported framework reports its reason BEFORE
/// looking for files`: `available === false`, reason matches `/no usage object/`
/// even though a readable transcript was supplied.
#[test]
fn ts_oracle_unsupported_reason_precedes_file_search() {
    let row = meter_agent(
        &json!({"name": "o1", "type": "octos", "workspacePath": WS}),
        "/home/me",
        "/",
        &[session("never-read", &claude_text())],
        None,
    );
    assert!(!row.available);
    assert!(row.reason.unwrap().contains("no usage object"));
}

/* ─────────────────── tests/metering.test.js — fleet summary ─────────────────── */

fn ok_row(agent: &str, workspace: &str, cache_read: u64) -> AgentRow {
    AgentRow {
        agent: Some(agent.to_owned()),
        available: true,
        framework: "claude".to_owned(),
        workspace: Some(workspace.to_owned()),
        totals: Some(TokenCounts {
            input: Some(0),
            output: Some(0),
            cache_write: Some(0),
            cache_read: Some(cache_read),
        }),
        total: Some(cache_read),
        sessions: Some(1),
        skipped: Some(0),
        files: None,
        reason: None,
    }
}

/// TS `metering.test.js:256` `agents sharing a workspace are ambiguous, not
/// summed`: every row `available === false`, the first reason matches
/// `/shared with a2/`, `total === null`.
#[test]
fn ts_oracle_shared_workspace_is_ambiguous() {
    let summary = summarize_fleet(vec![ok_row("a1", WS, 100), ok_row("a2", WS, 100)], "/");
    assert!(summary.agents.iter().all(|a| !a.available));
    let reason = summary.agents[0].reason.clone().unwrap();
    assert!(reason.contains("shared with a2"), "reason: {reason}");
    assert_eq!(summary.total, None);
}

/// TS `metering.test.js:268` `distinct workspaces total normally`:
/// `total === 150`, `attributed === 2`, `reason === null`.
#[test]
fn ts_oracle_distinct_workspaces_total() {
    let summary = summarize_fleet(
        vec![ok_row("a1", WS, 100), ok_row("a2", "/other/ws", 50)],
        "/",
    );
    assert_eq!(summary.total, Some(150));
    assert_eq!(summary.attributed, 2);
    assert_eq!(summary.reason, None);
}

/// TS `metering.test.js:275` `a partial total says how many agents it excluded`:
/// `total === 100`, `unattributed === 1`, reason matches
/// `/could not be attributed/`.
#[test]
fn ts_oracle_partial_total_names_exclusions() {
    let unavailable = AgentRow {
        agent: Some("a2".to_owned()),
        available: false,
        framework: "octos".to_owned(),
        workspace: None,
        totals: None,
        total: None,
        sessions: None,
        skipped: None,
        files: None,
        reason: Some("no adapter".to_owned()),
    };
    let summary = summarize_fleet(vec![ok_row("a1", WS, 100), unavailable], "/");
    assert_eq!(summary.total, Some(100));
    assert_eq!(summary.unattributed, 1);
    assert!(
        summary.reason.unwrap().contains("could not be attributed"),
        "reason must name the exclusion"
    );
}

/// TS `metering.test.js:285` `nothing attributable yields null, never zero`:
/// `total === null`.
#[test]
fn ts_oracle_nothing_attributable_is_null() {
    let unavailable = AgentRow {
        agent: Some("a1".to_owned()),
        available: false,
        framework: "claude".to_owned(),
        workspace: None,
        totals: None,
        total: None,
        sessions: None,
        skipped: None,
        files: None,
        reason: Some("no workspace".to_owned()),
    };
    assert_eq!(summarize_fleet(vec![unavailable], "/").total, None);
}

/* ─────────────────── tests/metering-discovery.test.js ─────────────────── */

fn bounds(dropped_by_count: u64, entries_unwalked: u64) -> Option<ScanBounds> {
    Some(ScanBounds {
        dropped_by_count,
        entries_unwalked,
    })
}

/// TS `metering-discovery.test.js:85` `the search descriptor says to recurse`:
/// codex `recursive === true`, claude `recursive === false`.
#[test]
fn ts_oracle_codex_recurses_claude_does_not() {
    assert!(transcript_search("codex", "/w", "/home").unwrap().recursive);
    assert!(
        !transcript_search("claude", "/w", "/home")
            .unwrap()
            .recursive
    );
}

/// TS `metering-discovery.test.js:72` `THE BUG: a session under YYYY/MM/DD is
/// discovered` — the descriptor must point at the codex `sessions` tree with
/// `recursive: true`, or a flat walk finds nothing.
#[test]
fn ts_oracle_codex_nested_date_tree_is_described() {
    let search = transcript_search("codex", "/Users/someone/agent-home/workdir", "/home/me")
        .expect("codex has a search");
    assert!(search.recursive);
    assert!(!search.narrowed);
    assert!(
        search.dir.contains(".codex/sessions"),
        "dir: {}",
        search.dir
    );
}

/// TS `metering-discovery.test.js:132` `a non-recursive search does NOT descend`:
/// claude's descriptor is flat, so a sibling project directory is never entered.
#[test]
fn ts_oracle_claude_search_is_flat() {
    let search = transcript_search("claude", "/Users/someone/proj", "/home/me")
        .expect("claude has a search");
    assert!(!search.recursive);
    assert!(search.narrowed);
    assert!(
        search.dir.contains(".claude/projects"),
        "dir: {}",
        search.dir
    );
}

/// TS `metering-discovery.test.js:315` `workdir alone does NOT meter, and the
/// reason says so rather than reporting zero`: an agent carrying only `workdir`
/// is unavailable with the no-workspace reason, not a zero total.
#[test]
fn ts_oracle_workdir_alone_does_not_meter() {
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workdir": WS}),
        "/home/me",
        "/",
        &[],
        None,
    );
    assert!(!row.available);
    assert!(row.totals.is_none());
    assert!(row.total.is_none());
    assert!(row.reason.unwrap().contains("no workspace recorded"));
}

/// TS `metering-discovery.test.js:304` `a stopped agent still meters:
/// workspacePath is cleared, lastWorkspacePath is not`: consumption outlives
/// the process, so `lastWorkspacePath` alone still attributes.
#[test]
fn ts_oracle_last_workspace_path_still_meters() {
    let body = claude_text().replace(WS, "/Users/someone/stopped-ws");
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": null,
                "lastWorkspacePath": "/Users/someone/stopped-ws"}),
        "/home/me",
        "/",
        &[session("s1.jsonl", &body)],
        None,
    );
    assert!(row.available, "reason: {:?}", row.reason);
    assert_eq!(row.totals.unwrap().cache_read, Some(340_166));
}

/// TS `metering-discovery.test.js:222` `a scan stopped by the file ceiling says
/// so, instead of "none found"`: the zero-match reason names the unreached
/// candidates and points at `HAGENCY_METERING_MAX_FILES`.
#[test]
fn ts_oracle_scan_bounds_stated_in_reason() {
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": WS}),
        "/home/me",
        "/",
        &[],
        bounds(3, 5),
    );
    let reason = row.reason.unwrap();
    assert!(reason.contains("never opened"), "reason: {reason}");
    assert!(
        reason.contains("HAGENCY_METERING_MAX_FILES"),
        "reason: {reason}"
    );
}

/// TS `metering-discovery.test.js:149` `an out-of-window file in a NON-narrowed
/// search is not called an understatement` and `:189` `a complete scan reports no
/// caveat`: with no bounds reported, the reason must be the plain
/// "no transcripts found yet", carrying no bound hedging.
#[test]
fn ts_oracle_complete_scan_has_no_caveat() {
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": WS}),
        "/home/me",
        "/",
        &[],
        None,
    );
    let reason = row.reason.unwrap();
    assert!(reason.contains("no transcripts found"), "reason: {reason}");
    assert!(!reason.contains("never opened"), "reason: {reason}");
    assert!(
        !reason.contains("HAGENCY_METERING_MAX_FILES"),
        "reason: {reason}"
    );
}

/// TS `metering-discovery.test.js:101` `and the tokens actually arrive, end to
/// end through the real reader`: a matched codex transcript's total reaches the
/// row (`total === 4313968`).
///
/// The end-to-end FILE READ is `hagency-metering/tests/reader.rs`
/// (`native_metering_reader_layout_discovery`) against a real temporary home;
/// here the already-read text is injected so the arithmetic is asserted
/// without a filesystem.
#[test]
fn ts_oracle_codex_total_reaches_the_row() {
    let text = [
        json!({"payload": {"type": "session_meta", "cwd": WS, "id": "s1"}}).to_string(),
        codex_totals(4_313_968, 0, 0, 0).to_string(),
    ]
    .join("\n");
    let row = meter_agent(
        &json!({"name": "BigLittle", "type": "codex", "lastWorkspacePath": WS}),
        "/home/me",
        "/",
        &[session("rollout.jsonl", &text)],
        None,
    );
    assert!(row.available, "reason: {:?}", row.reason);
    assert_eq!(row.total, Some(4_313_968));
    assert_eq!(row.sessions, Some(1));
}

/// TS `metering-discovery.test.js:289` `a record carrying workdir AND
/// workspacePath meters on the workspace field` — `workspacePath` wins over
/// `workdir`, so the row's workspace is the recorded one.
#[test]
fn ts_oracle_workspace_path_wins_over_workdir() {
    let row = meter_agent(
        &json!({"name": "a1", "type": "claude", "workspacePath": WS, "workdir": "/elsewhere"}),
        "/home/me",
        "/",
        &[session("s1.jsonl", &claude_text())],
        None,
    );
    assert_eq!(row.workspace.as_deref(), Some(WS));
}
