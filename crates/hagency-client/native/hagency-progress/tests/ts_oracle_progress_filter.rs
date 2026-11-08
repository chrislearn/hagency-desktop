//! TS test oracle — `tests/progress-filter.test.js` (34 cases).
//!
//! The retained TS suite is the parity ORACLE: each test below asserts the SAME
//! observable outcome the TS case asserts, against the native surface that owns
//! it (`hagency-progress`: `policy.rs` `Filter`/`Verb`/`acp_tool`/`acp_failed`,
//! `summary.rs` `build_summary`/`Counts`, `tools.rs`). Where native differs the
//! test keeps asserting the TS outcome and is `#[ignore]`d with a one-line
//! reason — this task changes no product code.
use hagency_progress::{Counts, Filter, Kind, Source, Verb, acp_failed, acp_tool, build_summary};
use serde_json::{Value, json};

/// A `Filter::decide` decision reduced to the fields the TS caller asserts:
/// `{report, verb}` (verb as the TS string, `null` when absent).
fn decision(event: Option<&str>, tool: Option<&str>, filter: &Filter) -> Value {
    let d = filter.decide(event, tool);
    json!({
        "report": d.report,
        "verb": d.verb.map(|v| v.text()),
    })
}

/* ── "the default, which is the old hardcoded behaviour written down" ── */

#[test]
fn ts_oracle_no_filter_reports_start_steps_and_completion() {
    let filter = Filter::default();
    // TS: absent config means "nobody configured this", never "report nothing".
    assert_eq!(decision(Some("start"), None, &filter)["report"], true);
    assert_eq!(
        decision(Some("PostToolUse"), Some("Read"), &filter)["report"],
        true
    );
    assert_eq!(decision(Some("Stop"), None, &filter)["report"], true);
}

#[test]
fn ts_oracle_bash_is_reported_as_a_verb_never_a_command() {
    assert_eq!(Verb::for_tool("Bash").text(), "ran commands");
    let d = Filter::default().decide(Some("PostToolUse"), Some("Bash"));
    assert!(d.report);
    assert_eq!(d.verb.map(|v| v.text()), Some("ran commands"));
}

#[test]
fn ts_oracle_unknown_tool_is_worked_not_its_own_name() {
    assert_eq!(Verb::for_tool("SomeCustomerMcpTool").text(), "worked");
    // TS `verbFor(undefined)` -> "worked": native has no tool name -> Worked.
    assert_eq!(Verb::for_tool("").text(), "worked");
}

/* ── "narrowing what leaves the machine" ── */

#[test]
fn ts_oracle_events_can_be_reduced_to_start_and_finish() {
    let filter = Filter::parse(&json!({"events": ["start", "done"]}), None).unwrap();
    assert_eq!(decision(Some("start"), None, &filter)["report"], true);
    assert_eq!(decision(Some("Stop"), None, &filter)["report"], true);
    assert_eq!(
        decision(Some("PostToolUse"), Some("Read"), &filter)["report"],
        false
    );
}

#[test]
fn ts_oracle_a_tool_can_be_excluded_outright() {
    let filter = Filter::parse(&json!({"tools": {"exclude": ["Bash"]}}), None).unwrap();
    assert_eq!(
        decision(Some("PostToolUse"), Some("Bash"), &filter)["report"],
        false
    );
    assert_eq!(
        decision(Some("PostToolUse"), Some("Read"), &filter)["report"],
        true
    );
}

#[test]
fn ts_oracle_an_include_list_makes_everything_else_silent() {
    let filter = Filter::parse(&json!({"tools": {"include": ["Read"]}}), None).unwrap();
    assert_eq!(
        decision(Some("PostToolUse"), Some("Read"), &filter)["report"],
        true
    );
    assert_eq!(
        decision(Some("PostToolUse"), Some("Edit"), &filter)["report"],
        false
    );
}

#[test]
fn ts_oracle_a_step_with_no_tool_name_is_refused_not_worked() {
    let filter = Filter::default();
    // TS: malformed payload must not become a line in a customer's room.
    assert_eq!(
        decision(Some("PostToolUse"), None, &filter)["report"],
        false
    );
    assert_eq!(
        decision(Some("PostToolUse"), Some(""), &filter)["report"],
        false
    );
}

/* ── "per-customer rules" ── */

#[test]
fn ts_oracle_a_group_with_its_own_rule_uses_it_and_says_so() {
    let raw =
        json!({"events": ["start", "step", "done"], "perGroup": {"acme": {"events": ["done"]}}});
    let scoped = Filter::parse(&raw, Some("acme")).unwrap();
    assert_eq!(scoped.source(), Source::PerGroup);
    assert_eq!(decision(Some("start"), None, &scoped)["report"], false);
    assert_eq!(decision(Some("Stop"), None, &scoped)["report"], true);
}

#[test]
fn ts_oracle_a_group_with_no_rule_of_its_own_uses_the_defaults() {
    let raw =
        json!({"events": ["start", "step", "done"], "perGroup": {"acme": {"events": ["done"]}}});
    let other = Filter::parse(&raw, Some("other")).unwrap();
    assert_eq!(other.source(), Source::File);
    assert_eq!(decision(Some("start"), None, &other)["report"], true);
}

#[test]
fn ts_oracle_a_per_group_rule_replaces_rather_than_merging() {
    let raw = json!({"tools": {"exclude": ["Bash"]}, "perGroup": {"acme": {"events": ["step"]}}});
    let scoped = Filter::parse(&raw, Some("acme")).unwrap();
    // TS: the scoped rule REPLACES the top level, so Bash is NOT excluded here.
    assert_eq!(
        decision(Some("PostToolUse"), Some("Bash"), &scoped)["report"],
        true
    );
}

/* ── "failing closed on configuration, open on absence" ── */

#[test]
fn ts_oracle_bad_config_is_refused_not_repaired() {
    for bad in [
        json!({"events": "done"}),
        json!({"events": ["done", "  "]}),
        json!({"events": ["done", 3]}),
        json!({"tools": ["Bash"]}),
        json!({"tools": {"exclude": "Bash"}}),
        json!({"perGroup": []}),
        json!({"minIntervalMs": -1}),
        json!({"minIntervalMs": "60s"}),
        json!([]),
        json!("events=done"),
    ] {
        assert!(
            matches!(
                Filter::parse(&bad, None),
                Err(hagency_progress::Error::Config(_))
            ),
            "refused, not repaired: {bad}"
        );
    }
}

#[test]
fn ts_oracle_the_interval_has_a_floor_that_cannot_be_configured_away() {
    assert_eq!(
        Filter::parse(&json!({"minIntervalMs": 0}), None)
            .unwrap()
            .min_interval_ms(),
        5000.0
    );
    assert_eq!(
        Filter::parse(&json!({"minIntervalMs": 1}), None)
            .unwrap()
            .min_interval_ms(),
        5000.0
    );
    assert_eq!(
        Filter::parse(&json!({"minIntervalMs": 120000}), None)
            .unwrap()
            .min_interval_ms(),
        120000.0
    );
}

#[test]
fn ts_oracle_deciding_is_pure_same_inputs_same_answer() {
    let filter = Filter::parse(
        &json!({"events": ["step"], "tools": {"exclude": ["Bash"]}}),
        None,
    )
    .unwrap();
    let once = decision(Some("PostToolUse"), Some("Read"), &filter);
    let twice = decision(Some("PostToolUse"), Some("Read"), &filter);
    assert_eq!(once, twice);
}

/* ── "ACP as the transport, which needs nothing installed" ── */

#[test]
fn ts_oracle_a_tool_call_becomes_the_same_shape_a_hook_payload_has() {
    assert_eq!(
        acp_tool(&json!({"sessionUpdate": "tool_call", "kind": "read", "toolCallId": "t1"})),
        Some("Read")
    );
}

#[test]
fn ts_oracle_kinds_map_to_the_tool_names_an_operator_filters_by() {
    assert_eq!(
        acp_tool(&json!({"sessionUpdate": "tool_call", "kind": "execute"})),
        Some("Bash")
    );
    let filter = Filter::parse(&json!({"tools": {"exclude": ["Bash"]}}), None).unwrap();
    assert_eq!(
        decision(Some("PostToolUse"), Some("Bash"), &filter)["report"],
        false
    );
}

#[test]
fn ts_oracle_thinking_is_dropped_rather_than_reported() {
    assert_eq!(
        acp_tool(&json!({"sessionUpdate": "tool_call", "kind": "think"})),
        None
    );
}

#[test]
fn ts_oracle_an_unknown_kind_is_generic_activity_never_its_own_name() {
    assert_eq!(
        acp_tool(&json!({"sessionUpdate": "tool_call", "kind": "something_new_in_2027"})),
        Some("AcpTool")
    );
    let d = Filter::default().decide(Some("PostToolUse"), Some("AcpTool"));
    assert!(d.report);
    assert_eq!(d.verb.map(|v| v.text()), Some("worked"));
}

#[test]
fn ts_oracle_a_repeat_update_for_one_call_is_not_counted_again() {
    assert_eq!(
        acp_tool(
            &json!({"sessionUpdate": "tool_call_update", "kind": "read", "status": "completed"})
        ),
        None
    );
}

#[test]
fn ts_oracle_everything_that_is_not_a_tool_call_is_ignored() {
    for kind in [
        "agent_message_chunk",
        "plan",
        "user_message_chunk",
        "agent_thought_chunk",
    ] {
        assert_eq!(acp_tool(&json!({"sessionUpdate": kind})), None);
    }
    assert_eq!(acp_tool(&Value::Null), None);
    assert_eq!(acp_tool(&json!({})), None);
}

#[test]
fn ts_oracle_the_title_is_never_carried_because_the_agent_wrote_it() {
    // Native `acp_tool` reads only `sessionUpdate` + `kind` and returns a fixed
    // verb from the kind table; no title/rawInput field ever reaches the room.
    assert_eq!(
        acp_tool(&json!({
            "sessionUpdate": "tool_call",
            "kind": "read",
            "title": "Read /home/customer/.env with the API keys",
            "rawInput": {"arguments": {"path": "/home/customer/.env"}}
        })),
        Some("Read")
    );
}

/* ── "a turn that only failed must not read as a turn that worked" ── */

#[test]
fn ts_oracle_nothing_succeeded_is_said_plainly() {
    assert_eq!(
        build_summary(Kind::Done, &Counts::default(), 16, None).unwrap(),
        "finished, but nothing succeeded — 16 failed attempts"
    );
}

#[test]
fn ts_oracle_one_failed_attempt_is_singular() {
    assert_eq!(
        build_summary(Kind::Done, &Counts::default(), 1, None).unwrap(),
        "finished, but nothing succeeded — 1 failed attempt"
    );
}

#[test]
fn ts_oracle_mixed_work_and_failure_reports_both() {
    let mut counts = Counts::default();
    counts.add(Verb::Read, 3).unwrap();
    assert_eq!(
        build_summary(Kind::Done, &counts, 2, None).unwrap(),
        "finished — read ×3, 2 failed"
    );
}

#[test]
fn ts_oracle_a_clean_turn_is_unchanged() {
    let mut counts = Counts::default();
    counts.add(Verb::Read, 3).unwrap();
    counts.add(Verb::Edited, 1).unwrap();
    assert_eq!(
        build_summary(Kind::Done, &counts, 0, None).unwrap(),
        "finished — read ×3, edited"
    );
    assert_eq!(
        build_summary(Kind::Done, &Counts::default(), 0, None).unwrap(),
        "finished"
    );
    assert_eq!(
        build_summary(Kind::Start, &Counts::default(), 0, None).unwrap(),
        "started"
    );
}

#[test]
fn ts_oracle_failures_reach_a_step_line_too() {
    assert_eq!(
        build_summary(Kind::Step, &Counts::default(), 4, None).unwrap(),
        "4 failed attempts"
    );
    let mut counts = Counts::default();
    counts.add(Verb::Read, 1).unwrap();
    assert_eq!(
        build_summary(Kind::Step, &counts, 1, None).unwrap(),
        "read, 1 failed"
    );
}

#[test]
fn ts_oracle_an_empty_step_is_still_nothing_to_say() {
    assert_eq!(build_summary(Kind::Step, &Counts::default(), 0, None), None);
}

#[test]
fn ts_oracle_a_failure_is_read_from_the_update_kind_activity_counting_drops() {
    assert!(acp_failed(
        &json!({"sessionUpdate": "tool_call_update", "status": "failed"})
    ));
    assert_eq!(
        acp_tool(&json!({"sessionUpdate": "tool_call_update", "status": "failed", "kind": "read"})),
        None
    );
}

#[test]
fn ts_oracle_only_failed_counts_as_failure() {
    for status in ["pending", "in_progress", "completed", "", ""] {
        assert!(!acp_failed(
            &json!({"sessionUpdate": "tool_call_update", "status": status})
        ));
    }
    assert!(!acp_failed(
        &json!({"sessionUpdate": "tool_call", "status": "failed"})
    ));
}

#[test]
fn ts_oracle_no_reason_travels_with_the_count() {
    let line = build_summary(Kind::Done, &Counts::default(), 3, None).unwrap();
    for banned in ["error", "ENOENT", "/home", "denied", "refused"] {
        assert!(
            !line.to_lowercase().contains(&banned.to_lowercase()),
            "{line} leaks {banned}"
        );
    }
}

/* ── "work that produced no answer" (the delivered===0 correction) ── */

fn counts_worked(n: u32) -> Counts {
    let mut c = Counts::default();
    c.add(Verb::Worked, n).unwrap();
    c
}

#[test]
fn ts_oracle_activity_with_nothing_sent_says_so() {
    assert_eq!(
        build_summary(Kind::Done, &counts_worked(16), 0, Some(0)).unwrap(),
        "finished — worked ×16, but sent nothing"
    );
}

#[test]
fn ts_oracle_activity_with_something_sent_is_unchanged() {
    assert_eq!(
        build_summary(Kind::Done, &counts_worked(16), 0, Some(1)).unwrap(),
        "finished — worked ×16"
    );
}

#[test]
fn ts_oracle_unknown_delivery_says_nothing_about_it() {
    assert_eq!(
        build_summary(Kind::Done, &counts_worked(2), 0, None).unwrap(),
        "finished — worked ×2"
    );
}

#[test]
fn ts_oracle_a_turn_that_did_nothing_is_not_accused_of_sending_nothing() {
    assert_eq!(
        build_summary(Kind::Done, &Counts::default(), 0, Some(0)).unwrap(),
        "finished"
    );
}

#[test]
fn ts_oracle_failures_still_take_precedence() {
    let mut counts = Counts::default();
    counts.add(Verb::Read, 2).unwrap();
    assert_eq!(
        build_summary(Kind::Done, &counts, 1, Some(0)).unwrap(),
        "finished — read ×2, 1 failed"
    );
}

#[test]
fn ts_oracle_no_cause_is_offered_for_the_silence() {
    let line = build_summary(Kind::Done, &counts_worked(3), 0, Some(0)).unwrap();
    for banned in ["tool", "mcp", "backend", "permission", "unreachable"] {
        assert!(
            !line.to_lowercase().contains(&banned.to_lowercase()),
            "{line} leaks {banned}"
        );
    }
}
