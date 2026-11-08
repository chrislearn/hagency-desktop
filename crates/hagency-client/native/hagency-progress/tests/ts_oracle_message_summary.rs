//! TS test oracle — message summary (task #76).
//!
//! Maps the retained `tests/message-summary.test.js` cases onto the native
//! `hagency_progress` summary module. TS `lib/message-summary.js` defines
//! `buildSummary(text, limit = SUMMARY_LIMIT)` with SUMMARY_LIMIT=240,
//! whitespace collapse, word-boundary cut, and a trailing "…" when cut.
//!
//! Native `hagency_progress::summary::build_summary` is a PROGRESS digest
//! ("finished — read ×3, 1 failed"), NOT the TS message-summary function.
//! The native service has no `tell` subcommand and no 240-character
//! truncation for user messages; these tests assert the TS behaviour and are
//! `#[ignore = "parity gap: …"]` — listed in report-76.md.

/// TS `lib/message-summary.js:15` — the longest summary that passes through whole.
const SUMMARY_LIMIT: usize = 240;

/// TS `lib/message-summary.js:23-31` — the reference implementation.
fn build_summary(text: &str, limit: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= limit {
        return collapsed;
    }
    let cut = collapsed[..limit - 1]
        .rfind(' ')
        .map(|i| &collapsed[..i])
        .unwrap_or(&collapsed[..limit - 1]);
    format!("{cut}…")
}

/// TS `message-summary.test.js:15` — a 1..240-character message passes through whole.
#[test]
#[ignore = "parity gap: native has no message-summary truncation (no tell subcommand, no 240 limit)"]
fn ts_message_summary_passes_through_whole() {
    for n in [1, 71, 72, 73, 100, 239, 240] {
        let text = "x".repeat(n);
        assert_eq!(build_summary(&text, SUMMARY_LIMIT), text);
    }
}

/// TS `message-summary.test.js:20` — the limit itself is not truncated.
#[test]
#[ignore = "parity gap: native has no message-summary truncation"]
fn ts_message_summary_limit_not_truncated() {
    let text = "y".repeat(SUMMARY_LIMIT);
    assert_eq!(build_summary(&text, SUMMARY_LIMIT), text);
    assert!(!build_summary(&text, SUMMARY_LIMIT).contains('…'));
}

/// TS `message-summary.test.js:26` — one character over the limit is cut and marked.
#[test]
#[ignore = "parity gap: native has no message-summary truncation"]
fn ts_message_summary_one_over_cut_and_marked() {
    let text = format!("{}end", "word ".repeat(60));
    let summary = build_summary(&text, SUMMARY_LIMIT);
    assert!(summary.len() <= SUMMARY_LIMIT);
    assert!(summary.ends_with('…'));
}

/// TS `message-summary.test.js:33` — the cut lands on a word boundary, never mid-word.
#[test]
#[ignore = "parity gap: native has no message-summary truncation"]
fn ts_message_summary_cut_on_word_boundary() {
    let text = format!("{}omega", "alpha bravo charlie delta ".repeat(20));
    let summary = build_summary(&text, SUMMARY_LIMIT);
    let cut = &summary[..summary.len() - 1];
    assert!(text.starts_with(cut));
    assert!(text[cut.len()..].starts_with(char::is_whitespace));
}

/// TS `message-summary.test.js:40` — whitespace is collapsed so a multi-line body reads as one line.
#[test]
#[ignore = "parity gap: native has no message-summary truncation"]
fn ts_message_summary_whitespace_collapsed() {
    assert_eq!(
        build_summary("one\n\ntwo   three\t four", SUMMARY_LIMIT),
        "one two three four"
    );
}

/// TS `message-summary.test.js:45` — a single word longer than the limit is still cut to fit.
#[test]
#[ignore = "parity gap: native has no message-summary truncation"]
fn ts_message_summary_single_word_longer_than_limit() {
    let summary = build_summary(&"z".repeat(400), SUMMARY_LIMIT);
    assert!(summary.len() <= SUMMARY_LIMIT);
    assert!(summary.ends_with('…'));
}

/// TS `message-summary.test.js:52` — null/undefined/'' becomes an empty string, not "null".
#[test]
#[ignore = "parity gap: native has no message-summary truncation"]
fn ts_message_summary_empty_input() {
    assert_eq!(build_summary("", SUMMARY_LIMIT), "");
}
