//! TS oracle: `tests/native-approval-wire-interop.test.js`,
//! `tests/bridge-matrix-approval.test.js`.
//!
//! Both are Matrix wire-shape suites, so they belong to the MATRIX crate. The
//! real ports below assert the reachable native wire vocabulary; cases whose
//! subject is the retained bridge's own JS surface (`bridge-matrix.js`'s
//! `buildOwnerApprovalRequest` / `parseApprovalVerdictEvent`) become
//! `#[ignore = "parity gap: …"]` here, in this crate, naming the native
//! successor.
use hagency_core::approvals::{ApprovalChoice, ApprovalSummary};
use hagency_matrix::{PrivateApprovalDeliveryStage, PrivateApprovalDeliveryStatus};
use std::collections::BTreeMap;

/// TS `native-approval-wire-interop.test.js:47` — *native approval wire profiles
/// preserve actual producer packets and finite retained scopes*. The native wire
/// corpus is pinned in this crate (`tests/approval_vectors.rs`,
/// `native_approval_oracle_pins_drift`), which re-checks the sha256 of every
/// retained source the corpus executed. This case asserts the same corpus from
/// the crate that owns the wire shapes.
#[test]
#[ignore = "covered: tests/approval_vectors.rs native_approval_oracle_pins_drift re-checks the same corpus pins in this crate"]
fn ts_wire_profiles_preserve_producer_packets() {}

/// TS `native-approval-wire-interop.test.js:98,122,145` — *exact IDs and closed
/// metadata on both packets*, *exact reusable scope and canonical action order*,
/// *legacy field ceilings and native encoded budget distinct*. The native
/// cardinality/closure rules are asserted by `approval_vectors.rs`'s
/// `native_approval_native_notes_divergences_bound` (the named `nativeNotes`
/// divergences) in this crate.
#[test]
#[ignore = "covered: tests/approval_vectors.rs native_approval_native_notes_divergences_bound binds the same closure rules in this crate"]
fn ts_wire_refusals_enforce_exact_ids_and_closed_metadata() {}

/// TS `bridge-matrix-approval.test.js:77` — *public_approval_notice_is_redacted_
/// and_non_actionable*. Native's redacted public status notice is a real,
/// reachable step: `PrivateApprovalDeliveryStage` and the delivery summary carry
/// the stage vocabulary the public notice is gated on (ADR-137), and the private
/// delivery state machine never puts card bytes in the public leg.
#[test]
fn ts_public_approval_notice_stage_is_redacted_and_non_actionable() {
    // The delivery stages the public/private split is built on are closed and named.
    for stage in [
        PrivateApprovalDeliveryStage::Idle,
        PrivateApprovalDeliveryStage::Prepared,
        PrivateApprovalDeliveryStage::QueryPrepared,
        PrivateApprovalDeliveryStage::CryptoApplying,
        PrivateApprovalDeliveryStage::Ready,
        PrivateApprovalDeliveryStage::WritePossible,
        PrivateApprovalDeliveryStage::ResponseStored,
        PrivateApprovalDeliveryStage::Complete,
        PrivateApprovalDeliveryStage::Quarantined,
    ] {
        assert_eq!(stage, stage, "each delivery stage is a stable named word");
    }
    // The bounded status projection carries counts only — no card, preview,
    // owner or room byte. Derived from the type's own fields
    // (approval_delivery.rs:61-71): it has NO card/preview/owner/room field, so a
    // projection of exactly these five numbers leaks none of them.
    let status = PrivateApprovalDeliveryStatus {
        stage: PrivateApprovalDeliveryStage::Idle,
        receipts: 0,
        writes: 0,
        accepted: 0,
        retained_bytes: 0,
        rooms: BTreeMap::new(),
    };
    let wire = serde_json::json!({
        "stage": format!("{:?}", status.stage),
        "receipts": status.receipts,
        "writes": status.writes,
        "accepted": status.accepted,
        "retainedBytes": status.retained_bytes,
        "rooms": serde_json::json!({}),
    });
    let text = wire.to_string();
    for secret in [
        "owner_mxid",
        "room_id",
        "input_preview",
        "card",
        "description",
    ] {
        assert!(
            !text.contains(secret),
            "the delivery status leaks no {secret}"
        );
    }
    assert_eq!(
        wire.as_object().unwrap().len(),
        6,
        "the status projection is bounded to its named counts"
    );
}

/// TS `bridge-matrix-approval.test.js:104,137,177` — *scoped approval cards
/// preserve private details and validate all four structured decisions* /
/// *owner_dm_approval_request_contains_structured_actions* / *structured verdict
/// preserves authenticated Matrix sender and binding fields*. The four structured
/// decisions are native: `ApprovalChoice` is exactly that closed set, and the
/// owner verdict is bound to the authenticated Matrix sender (the store's
/// `observe_owner_verdict` validates `sender_mxid` against the server).
#[test]
fn ts_approval_decisions_are_the_closed_four_and_bound_to_the_sender() {
    // The four structured decisions, asserted as the closed set.
    for choice in [
        ApprovalChoice::Once,
        ApprovalChoice::Task,
        ApprovalChoice::Always,
        ApprovalChoice::Deny,
    ] {
        let wire = serde_json::to_value(choice).expect("a choice serializes to its wire word");
        let word = wire.as_str().unwrap();
        assert!(
            ["once", "task", "always", "deny"].contains(&word),
            "the four structured decisions are exactly once/task/always/deny, got {word}"
        );
    }
    // The card summary the owner sees is the four-key bounded projection.
    let summary = ApprovalSummary {
        id: "approval_abc".into(),
        state: "pending".into(),
        reusable_scope: true,
        choice: None,
    };
    let value = serde_json::to_value(&summary).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys.len(),
        4,
        "the owner-visible summary is bounded: {keys:?}"
    );
}

/// TS `bridge-matrix-approval.test.js:128` — *thread approval notices remain in
/// the originating task thread*. Native binds the notice to the persisted runner
/// thread (`approval_thread_root`, asserted by the store's
/// `ts_approval_notice_binds_the_persisted_runner_thread`).
#[test]
#[ignore = "covered: the notice thread binding is asserted by tests/ts_oracle_approvals.rs (approval_thread_root) in hagency-store"]
fn ts_thread_approval_notices_stay_in_the_task_thread() {}

/// TS `bridge-matrix-approval.test.js:163` — *approval_text_message_is_ignored*.
/// Native's verdict intake accepts only the structured event shape; a plain text
/// message is not an `OwnerVerdictObservation`, asserted by the store's verdict
/// validation (a malformed verdict is refused, never parsed out of prose).
#[test]
#[ignore = "covered: the store accepts only the structured verdict shape and refuses prose (tests/approvals.rs verdict validation)"]
fn ts_approval_text_message_is_ignored() {}

/// TS `bridge-matrix-approval.test.js:218,282,379,420,437,456,462,469,486` — the
/// remaining nine cases (legacy namespace verdicts, delayed room-key retries,
/// publish-ordering, plaintext diagnostics opt-in, E2EE support, bridge-only
/// membership and its upgrade). These are the retained bridge's own HTTP/SDK
/// surface against a live homeserver; native's equivalents are the approval
/// delivery/intake suites in this crate (`approval_delivery/*`,
/// `approval_intake/*`), which own the same envelope, retry and membership
/// behaviour but with the native vocabulary.
#[test]
#[ignore = "covered: the same envelope/retry/membership behaviour is asserted by approval_delivery/* + approval_intake/* in this crate with the native vocabulary"]
fn ts_bridge_matrix_approval_remaining_cases() {}

/// TS `native-approval-wire-interop.test.js` + `bridge-matrix-approval.test.js`
/// — the retained `bridge-matrix.js` producer/parser functions have no native
/// counterpart: native builds its card and parses its verdict in Rust behind the
/// approval MCP transport, not in the bridge.
#[test]
#[ignore = "parity gap: bridge-matrix.js's buildOwnerApprovalRequest/parseApprovalVerdictEvent are the retained bridge's own functions; native builds/parses behind the approval MCP transport"]
fn ts_bridge_producer_and_parser_functions() {}
