//! TS test oracle — display sanitising (task #76).
//!
//! Maps the retained `tests/sanitize-display.test.js` cases onto the native
//! `hagency_matrix_format` crate. TS `lib/push-relay-core.js:652-656` defines
//! `sanitizeForDisplay(text)` which strips ANSI escape sequences and C0/C1
//! control characters before pushing to a tmux pane.
//!
//! Native has no push-relay or tmux pane injection; the closest surface is
//! `hagency_matrix_format::links` which handles markdown link sanitisation
//! for Matrix HTML, not terminal display. These tests assert the TS behaviour
//! and are `#[ignore = "parity gap: …"]` — listed in report-76.md.

/// TS `lib/push-relay-core.js:652-656` — the reference implementation.
fn sanitize_for_display(text: &str) -> String {
    text.chars()
        .filter(|c| {
            // Strip ANSI escape sequences (simplified: just the CSI introducer)
            // and C0/C1 control characters except tab, newline, CR.
            !matches!(c, '\x1b' | '\x00'..='\x08' | '\x0b'..='\x0c' | '\x0e'..='\x1f' | '\x7f' | '\u{80}'..='\u{9f}')
        })
        .collect()
}

/// TS `sanitize-display.test.js:13` — passes through normal text unchanged.
#[test]
#[ignore = "parity gap: native has no display sanitiser for terminal injection"]
fn ts_sanitize_passes_normal_text() {
    assert_eq!(sanitize_for_display("Hello world"), "Hello world");
}

/// TS `sanitize-display.test.js:17` — returns empty string for non-string input.
#[test]
#[ignore = "parity gap: native has no display sanitiser"]
fn ts_sanitize_non_string_input() {
    // Rust type system prevents non-string input; this case is N/A in native.
    // TS: sanitizeForDisplay(null) === '' etc.
}

/// TS `sanitize-display.test.js:23` — strips C0 control characters (except tab, newline, CR).
#[test]
#[ignore = "parity gap: native has no display sanitiser"]
fn ts_sanitize_strips_c0() {
    assert_eq!(sanitize_for_display("hello\x00world"), "helloworld");
    assert_eq!(sanitize_for_display("alert\x07bell"), "alertbell");
    assert_eq!(sanitize_for_display("back\x08space"), "backspace");
}

/// TS `sanitize-display.test.js:30` — preserves tab, newline, and carriage return.
#[test]
#[ignore = "parity gap: native has no display sanitiser"]
fn ts_sanitize_preserves_whitespace() {
    assert_eq!(sanitize_for_display("line1\nline2"), "line1\nline2");
    assert_eq!(sanitize_for_display("col1\tcol2"), "col1\tcol2");
    assert_eq!(sanitize_for_display("win\r\nline"), "win\r\nline");
}

/// TS `sanitize-display.test.js:36` — strips ANSI escape sequences.
#[test]
#[ignore = "parity gap: native has no display sanitiser"]
fn ts_sanitize_strips_ansi() {
    assert_eq!(sanitize_for_display("\x1b[31mred text\x1b[0m"), "red text");
    assert_eq!(
        sanitize_for_display("\x1b[1;32mbold green\x1b[0m"),
        "bold green"
    );
}

/// TS `sanitize-display.test.js:41` — strips C1 control characters.
#[test]
#[ignore = "parity gap: native has no display sanitiser"]
fn ts_sanitize_strips_c1() {
    assert_eq!(
        sanitize_for_display("test\u{80}data\u{9f}end"),
        "testdataend"
    );
}

/// TS `sanitize-display.test.js:45` — handles combined injection attempt.
#[test]
#[ignore = "parity gap: native has no display sanitiser"]
fn ts_sanitize_combined_injection() {
    let malicious = "normal\x1b[31m\x07\x00 injected \x1b[0mtext";
    assert_eq!(sanitize_for_display(malicious), "normal injected text");
}
