//! TS-oracle tests for `tests/resource-allocation-budget.test.js` (board #63).
//!
//! The retained suite is the parity oracle; these assert the SAME observable
//! outcome the JS asserts — the pool/seat ledger figures — against
//! `hagency_core::allocation::resource_budget`, which is the native port of
//! `lib/resource-allocation-budget.js`.

use hagency_core::allocation::{Input, SeatStatus, resource_budget};
use serde_json::json;

fn budget(input: serde_json::Value) -> serde_json::Value {
    let parsed: Input = serde_json::from_value(input).unwrap();
    serde_json::to_value(resource_budget(&parsed).unwrap()).unwrap()
}

/// TS: `pool ledger counts unprovisioned reservations once and releases ended
/// or failed allocations` (`tests/resource-allocation-budget.test.js`).
///
/// `ended` and `failed` commitments release; a `pending` reservation with a
/// `planned` phase holds; an approval that was never allocated (null tokens)
/// contributes nothing.
#[test]
fn ts_oracle_pool_ledger_counts_reservations_once_and_releases_ended_or_failed() {
    let commitments = json!([
        {"id": "other-pool", "presetId": "high", "allocatedTokens": 4000, "seatId": "shared", "state": "active"},
        {"id": "other-account", "presetId": "private", "allocatedTokens": 9000, "seatId": "private", "state": "active"},
        {"id": "active", "presetId": "medium", "allocatedTokens": 100, "seatId": "shared", "state": "active"},
        {"id": "reserved", "presetId": "medium", "allocatedTokens": 200, "seatId": "shared", "state": "pending", "fulfillment": {"phase": "planned"}},
        {"id": "ended", "presetId": "medium", "allocatedTokens": 10000, "seatId": "shared", "state": "ended"},
        {"id": "failed", "presetId": "medium", "allocatedTokens": 10000, "seatId": "shared", "state": "pending", "fulfillment": {"phase": "failed"}},
        {"id": "not-approved", "presetId": "medium", "allocatedTokens": null, "seatId": "shared", "state": "pending"}
    ]);
    let base = json!({
        "preset": {"id": "medium", "ceiling": {"tokens": 1000, "period": "monthly"}},
        "seatId": "shared",
        "commitments": commitments,
    });
    let value = budget(base.clone());
    assert_eq!(value["remainingTokens"], json!(700));
    assert_eq!(value["pool"]["committed"], json!(300));
    assert_eq!(value["pool"]["remaining"], json!(700));
    assert_eq!(value["seat"]["committed"], json!(4300));
    assert_eq!(value["seat"]["quota"], json!(null));
    assert_eq!(value["seat"]["remaining"], json!(null));

    // Excluding the reservation releases it from the pool but REPORTS it as
    // `reserved` — the figure the caller is about to re-commit.
    let mut excluded = base;
    excluded["excludeEngagementId"] = json!("reserved");
    let value = budget(excluded);
    assert_eq!(value["reserved"], json!(200));
    assert_eq!(value["remainingTokens"], json!(900));
    assert_eq!(value["pool"]["committed"], json!(100));
    assert_eq!(value["seat"]["committed"], json!(4100));
}

/// TS: `a declared account quota cannot be bypassed with a different pool
/// period` (`tests/resource-allocation-budget.test.js`).
#[test]
fn ts_oracle_declared_quota_cannot_be_bypassed_with_a_different_period() {
    let base = json!({
        "preset": {"id": "medium", "ceiling": {"tokens": 1000, "period": "monthly"}},
        "seatId": "shared",
        "commitments": [],
    });
    // A daily declaration against a monthly pool is a PERIOD MISMATCH: the
    // figure goes unknown rather than falling back to the pool ceiling.
    let mut daily = base.clone();
    daily["declaration"] = json!({"quotaTokens": 100, "period": "daily"});
    let value = budget(daily);
    assert_eq!(value["remainingTokens"], json!(null));
    assert_eq!(value["seat"]["status"], json!("period_mismatch"));
    assert_eq!(
        resource_budget(&serde_json::from_value::<Input>(base.clone()).unwrap())
            .map(|b| b.seat.status)
            .unwrap(),
        SeatStatus::Undeclared
    );
    // Matching period: the quota bounds the answer.
    let mut monthly = base.clone();
    monthly["declaration"] = json!({"quotaTokens": 100, "period": "monthly"});
    assert_eq!(budget(monthly)["remainingTokens"], json!(100));
    // A ZERO quota is a real allocation, not "unset".
    let mut zero = base.clone();
    zero["declaration"] = json!({"quotaTokens": 0, "period": "monthly"});
    assert_eq!(budget(zero)["remainingTokens"], json!(0));
    // Auto-join against a declared-but-unknown seat is unknown, not unlimited.
    let mut auto = base;
    auto["declaration"] = json!({});
    auto["forAutoJoin"] = json!(true);
    assert_eq!(budget(auto)["remainingTokens"], json!(null));
}
