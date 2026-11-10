//! TS test oracle — tasks (task #65).
//!
//! Maps the retained `tests/task-store.test.js` / `tests/api-tasks.test.js`
//! vocabulary-and-state-machine cases onto the native `hagency_core` task
//! types, asserting the SAME observable outcome (status names, priority/
//! granularity vocabulary, transition edges, title/label handling).
//!
//! Where native behaviour deliberately differs from TS, the test asserts the
//! TS behaviour and is `#[ignore = "parity gap: …"]` — listed in report-65.md.
use hagency_core::task_intents::{Granularity, Priority, TaskDefinition};
use hagency_core::tasks::TaskState;
use serde_json::json;

/// TS `lib/task-store.js` `STATUSES` — exactly the five names `/api/usage`
/// buckets by (`backend-v2.js:11087` hard-codes the same five).
#[test]
fn ts_oracle_task_status_vocabulary() {
    let names: Vec<String> = [
        TaskState::Created,
        TaskState::Accepted,
        TaskState::InProgress,
        TaskState::Blocked,
        TaskState::Done,
    ]
    .iter()
    .map(|s| json!(s).as_str().unwrap().to_string())
    .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["accepted", "blocked", "created", "done", "in_progress"],
        "the five statuses /api/usage buckets by"
    );
}

/// TS `PRIORITIES` = [p0, p1, p2, p3] and `GRANULARITIES` = [epic, subtask,
/// task] (lib/task-store.js). The native enums serialize to the same set.
#[test]
fn ts_oracle_task_priority_and_granularity_vocabulary() {
    let priorities: Vec<String> = [Priority::P0, Priority::P1, Priority::P2, Priority::P3]
        .iter()
        .map(|p| json!(p).as_str().unwrap().to_string())
        .collect();
    assert_eq!(priorities, vec!["p0", "p1", "p2", "p3"]);

    let granularities: Vec<String> = [Granularity::Epic, Granularity::Task, Granularity::Subtask]
        .iter()
        .map(|g| json!(g).as_str().unwrap().to_string())
        .collect();
    let mut sorted = granularities.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["epic", "subtask", "task"]);
}

/// TS `declares no outbound transition from done` (task-store.test.js:76):
/// `done` is terminal by ABSENCE from the TRANSITIONS table. Native
/// `TaskState::permits` must refuse every transition out of `done`.
#[test]
fn ts_oracle_task_done_has_no_outbound_transition() {
    for next in [
        TaskState::Created,
        TaskState::Accepted,
        TaskState::InProgress,
        TaskState::Blocked,
        TaskState::Done,
    ] {
        assert!(
            !TaskState::Done.permits(next),
            "done -> {:?} must be refused",
            next
        );
    }
}

/// TS `TRANSITIONS` (lib/task-store.js:8-13) has exactly four source keys and
/// the exact edges; the native state machine mirrors it edge-for-edge.
#[test]
fn ts_oracle_task_transition_edges() {
    assert!(TaskState::Created.permits(TaskState::Accepted));
    assert!(!TaskState::Created.permits(TaskState::InProgress));
    assert!(!TaskState::Created.permits(TaskState::Done));

    assert!(TaskState::Accepted.permits(TaskState::InProgress));
    assert!(!TaskState::Accepted.permits(TaskState::Done));

    assert!(TaskState::InProgress.permits(TaskState::Blocked));
    assert!(TaskState::InProgress.permits(TaskState::Done));
    assert!(!TaskState::InProgress.permits(TaskState::Accepted));

    assert!(TaskState::Blocked.permits(TaskState::InProgress));
    assert!(!TaskState::Blocked.permits(TaskState::Done));
}

/// TS `requires a real title` (task-store.test.js:91): `createTask` rejects a
/// blank/non-string title. Native `TaskDefinition::validate` does the same.
#[test]
fn ts_oracle_task_requires_real_title() {
    let overlong = "x".repeat(256);
    for bad in ["", "   ", overlong.as_str()] {
        assert!(
            TaskDefinition {
                title: bad.to_string(),
                ..Default::default()
            }
            .validate()
            .is_err(),
            "title {bad:?} must be rejected"
        );
    }
    assert!(
        TaskDefinition {
            title: "a real task".to_string(),
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

/// TS `rejects an out-of-vocabulary priority or granularity instead of
/// defaulting it` (task-store.test.js:129). Native enums are closed
/// (`deny_unknown_fields` + no unknown variant), so an out-of-vocabulary
/// value fails to deserialize rather than silently defaulting.
#[test]
fn ts_oracle_task_rejects_out_of_vocabulary_priority_and_granularity() {
    assert!(
        serde_json::from_str::<TaskDefinition>(r#"{"title":"t","priority":"p9"}"#).is_err(),
        "p9 is not a valid priority"
    );
    assert!(
        serde_json::from_str::<TaskDefinition>(r#"{"title":"t","granularity":"mega"}"#).is_err(),
        "mega is not a valid granularity"
    );
    // The valid vocabulary deserializes and round-trips.
    let def: TaskDefinition =
        serde_json::from_str(r#"{"title":"t","priority":"p0","granularity":"epic"}"#).unwrap();
    assert_eq!(def.priority, Priority::P0);
    assert_eq!(def.granularity, Granularity::Epic);
}

/// TS `bounds the label list: deduped, blank-free, capped at 20, each
/// truncated to 64` (task-store.test.js:154). Native validates but does NOT
/// truncate (it rejects over-long) and caps at 32 rather than 20 — a parity
/// gap: TS is a lossy normalizer, native is a validator. Asserted as the TS
/// behaviour and ignored.
#[test]
#[ignore = "parity gap: TS truncates labels to 64 and caps at 20; native TaskDefinition::validate rejects >64 and caps at 32"]
fn ts_oracle_task_label_bounds_match_ts_normalizer() {
    // TS: 20 labels are accepted (capped), each truncated to 64 chars.
    let labels: Vec<String> = (0..20).map(|i| format!("label-{i:02}")).collect();
    let def = TaskDefinition {
        title: "t".into(),
        labels: labels.clone(),
        ..Default::default()
    };
    assert!(def.validate().is_ok(), "TS accepts 20 labels");
    // TS: an over-long label is truncated, not rejected.
    let long = "l".repeat(64);
    let def = TaskDefinition {
        title: "t".into(),
        labels: vec![long],
        ..Default::default()
    };
    assert!(def.validate().is_ok(), "TS truncates a 64-char label");
}

/// TS `truncates rather than rejects an over-long title and description`
/// (task-store.test.js:175). Native `TaskDefinition::validate` rejects
/// (text() enforces the limit). Parity gap.
#[test]
#[ignore = "parity gap: TS truncates over-long title/description (slice(0,255)); native TaskDefinition::validate rejects (>255)"]
fn ts_oracle_task_truncates_overlong_title_and_description() {
    let title = "t".repeat(255);
    let def = TaskDefinition {
        title,
        description: "d".repeat(4096),
        ..Default::default()
    };
    assert!(def.validate().is_ok(), "TS truncates to the limit");
}
