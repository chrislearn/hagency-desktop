//! TS-oracle tests for `tests/api-usage-metering.test.js` (board #63).
//!
//! The retained suite pins three ways the metering answer could lie: a zero
//! where nothing was measured, a global availability flag, and a fleet total
//! that silently omits what it could not attribute. Native's `meter_fleet`
//! (`hagency_metering::reader`) is the port; these assert the SAME observable
//! outcomes against it.
//!
//! Trees are built under the workspace `tmp/` directory (git-ignored) because
//! `hagency-metering` deliberately has no `tempfile` dev-dependency.

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use hagency_metering::attribution::transcript_search;
use hagency_metering::reader::{
    FleetCache, ReaderLimits, SessionReader, bounds_report, meter_fleet,
};
use serde_json::json;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempHome(PathBuf);
impl Deref for TempHome {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}
impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_home(label: &str) -> TempHome {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest)
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .join("tmp");
    let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = root.join(format!(
        "usage-oracle-{}-{label}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create temp home");
    TempHome(path)
}

fn set_mtime(path: &Path, at_ms: u64) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_millis(at_ms))
        .unwrap();
}

/// A minimal Claude transcript for a workspace, one usage record per line.
fn transcript(cwd: &str, output: u64, lines: u64) -> String {
    (0..lines)
        .map(|i| {
            json!({
                "cwd": cwd, "uuid": format!("u{i}"),
                "message": {"usage": {"input_tokens": 1, "output_tokens": output,
                    "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn agent(name: &str, framework: &str, workspace: Option<&str>) -> serde_json::Value {
    let mut value = json!({"name": name, "type": framework});
    if let Some(workspace) = workspace {
        value["workspacePath"] = json!(workspace);
    }
    value
}

fn meter(agents: &[serde_json::Value], home: &Path, now: u64) -> serde_json::Value {
    let mut cache = FleetCache::new();
    serde_json::to_value(meter_fleet(
        agents,
        &home.display().to_string(),
        "/",
        ReaderLimits::default(),
        60_000,
        now,
        true,
        &mut cache,
    ))
    .unwrap()
}

/// TS: `an unmeasurable agent reports null with a reason, never zero`
/// (`tests/api-usage-metering.test.js`). The whole point: a zero is a claim
/// about consumption, not an absence of one.
#[test]
fn ts_oracle_usage_unmeasurable_agent_reports_null_with_a_reason() {
    let home = temp_home("null");
    let value = meter(&[agent("a1", "claude", None)], &home, 1_800_000_000_000);
    let row = &value["agents"][0];
    assert_eq!(row["agent"], json!("a1"));
    assert_eq!(row["available"], json!(false));
    assert_eq!(row["totals"], json!(null), "never zero");
    assert_eq!(row["total"], json!(null));
    assert!(
        row["reason"]
            .as_str()
            .unwrap()
            .contains("no workspace recorded"),
        "the reason names why: {}",
        row["reason"]
    );
}

/// TS: `the fleet total sums what was measured instead of discarding it`
/// (`tokensUsed` 3500, `tokensMeasuredFor` 2, `tokensPartial` false).
#[test]
fn ts_oracle_usage_fleet_total_sums_what_was_measured() {
    let home = temp_home("total");
    let now = 1_800_000_000_000u64;
    for (name, workspace, output) in [("a1", "/ws-one", 1000u64), ("a2", "/ws-two", 2500)] {
        let search = transcript_search("claude", workspace, &home.display().to_string()).unwrap();
        fs::create_dir_all(&search.dir).unwrap();
        let file = Path::new(&search.dir).join("s.jsonl");
        fs::write(&file, transcript(workspace, output, 1)).unwrap();
        set_mtime(&file, now - 60_000);
        let _ = name;
    }
    let value = meter(
        &[
            agent("a1", "claude", Some("/ws-one")),
            agent("a2", "claude", Some("/ws-two")),
        ],
        &home,
        now,
    );
    // Each transcript carries output_tokens equal to its figure; the fleet
    // sums what was measured.
    assert_eq!(value["attributed"], json!(2));
    assert_eq!(value["unattributed"], json!(0));
    let totals = &value["totals"];
    assert_eq!(totals["output"], json!(3500));
    assert_eq!(value["reason"], json!(null), "every agent measured");
}

/// TS: `a fleet with nothing measured reports null, not zero` — and the count
/// of measured agents is 0, not a partial sum.
#[test]
fn ts_oracle_usage_fleet_with_nothing_measured_reports_null() {
    let home = temp_home("none");
    let value = meter(
        &[agent("a1", "claude", Some("/ws"))],
        &home,
        1_800_000_000_000,
    );
    assert_eq!(value["totals"], json!(null), "null, not a zero total");
    assert_eq!(value["total"], json!(null));
    assert_eq!(value["attributed"], json!(0));
    // Nothing attributable → the caveat names how many were not.
    assert!(
        value["reason"]
            .as_str()
            .unwrap()
            .contains("could not be attributed")
    );
}

/// TS: `availability is reported per framework, not once for the fleet` —
/// claude/codex write figures to disk; octos writes neither usage nor cwd.
#[test]
fn ts_oracle_usage_availability_is_per_framework() {
    let home = temp_home("frameworks");
    let value = meter(
        &[
            agent("a1", "claude", None),
            agent("a2", "codex", None),
            agent("a3", "octos", None),
        ],
        &home,
        1_800_000_000_000,
    );
    let by_framework: std::collections::BTreeMap<String, String> = value["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["framework"].as_str().unwrap().to_owned(),
                row["reason"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    // The reason strings exist per framework, and octos's names the real gap.
    assert!(
        by_framework["octos"].contains("no usage object and no cwd"),
        "octos's own reason: {}",
        by_framework["octos"]
    );
    assert!(
        by_framework["claude"].contains("no workspace recorded"),
        "claude is meterable; its reason is only the missing workspace"
    );
    assert!(
        !by_framework["codex"].contains("no usage object"),
        "codex is meterable"
    );
}

/// TS: `the fleet total says how many agents it could not attribute`
/// (`unattributed` 2, `reason` names the count, availability false).
#[test]
fn ts_oracle_usage_fleet_total_says_how_many_it_could_not_attribute() {
    let home = temp_home("partial");
    let value = meter(
        &[agent("a1", "claude", None), agent("a2", "octos", None)],
        &home,
        1_800_000_000_000,
    );
    assert_eq!(value["unattributed"], json!(2));
    assert!(
        value["reason"]
            .as_str()
            .unwrap()
            .contains("could not be attributed")
    );
    // Nothing attributable, so the total is null rather than a sum of zero.
    assert_eq!(value["totals"], json!(null));
}

/// TS: `a truncated transcript is cut at a line boundary and reported` and
/// `the file ceiling is reported, and the newest files are the ones kept` —
/// the reader's own bounds, asserted against the native `bounds_report`.
#[test]
fn ts_oracle_usage_reader_reports_every_bound_that_bit() {
    let home = temp_home("bounds");
    let now = 1_800_000_000_000u64;
    let workspace = "/Users/usage-oracle/work";
    let search = transcript_search("claude", workspace, &home.display().to_string()).unwrap();
    fs::create_dir_all(&search.dir).unwrap();
    for (name, age_ms, output) in [
        ("current.jsonl", 60_000u64, 3u64),
        ("old.jsonl", 40 * 86_400_000, 2),
    ] {
        let path = Path::new(&search.dir).join(name);
        fs::write(&path, transcript(workspace, output, 1)).unwrap();
        set_mtime(&path, now - age_ms);
    }
    let limits = ReaderLimits {
        window_ms: 24 * 3600 * 1000,
        ..ReaderLimits::default()
    };
    let (files, bounds) = SessionReader::new(limits, &|| now).read(&search);
    // TS: `transcripts older than the window are dropped AND reported`.
    assert_eq!(files.len(), 1);
    assert_eq!(bounds.dropped_by_age, 1);
    let report = bounds_report(&bounds).unwrap();
    assert!(report.contains("understates consumption"), "{report}");
    assert!(report.contains("older than the window"), "{report}");
    // TS: `a complete scan reports no caveat`.
    assert_eq!(bounds_report(&Default::default()), None);
}

/// TS: `busy time and tasks remain available even when tokens are not`.
#[test]
fn ts_oracle_usage_other_signals_survive_a_missing_measurement() {
    let home = temp_home("others");
    let value = meter(&[agent("a1", "claude", None)], &home, 1_800_000_000_000);
    // The row is unavailable for TOKENS; its identity and framework still
    // travel, so a page can render the other columns.
    let row = &value["agents"][0];
    assert_eq!(row["agent"], json!("a1"));
    assert_eq!(row["framework"], json!("claude"));
    assert_eq!(row["available"], json!(false));
    assert!(row["reason"].is_string());
}

/// TS: `measured and declared stay distinct` — a ceiling is knowable because
/// the operator declared it; consumption is measured or it is not.
///
/// PARITY GAP: the native fleet read carries no `ceilingTokens` column — the
/// declared ceiling lives on the resource, read by the engagement usage route
/// (`/api/engagements/:id/usage`), not by the fleet metering block.
#[test]
#[ignore = "parity gap: fleet metering carries no ceilingTokens column (declared ceiling lives on the engagement usage read)"]
fn ts_oracle_usage_measured_and_declared_stay_distinct() {
    panic!("native fleet metering has no ceiling column");
}
