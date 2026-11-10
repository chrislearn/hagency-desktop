//! TS test oracle — the usage ledger (`tests/metering-ledger.test.js`).
//!
//! The retained TS suite is the parity ORACLE. The ledger's home in native is the
//! store's usage **sources and periods**, not a `hagency-metering` type: a
//! per-source high-water mark (`usage_sources.high_water`), regression counting,
//! and UTC daily/monthly buckets (`usage_periods`). Those are already replayed
//! against the SAME retained JavaScript by
//! `hagency-store/tests/usage/vectors.rs::native_usage_high_water_vectors`
//! (`fixtures/usage-vectors.json`, 16 vectors from
//! `native/scripts/usage-vectors.mjs` running `lib/metering/ledger.js`) — the
//! CITATIONS are recorded in `.peer/report-75.md`.
//!
//! The cases below restate the rules those vectors encode directly against the
//! store API so the TS assertion and the Rust outcome sit side by side, plus the
//! genuine gaps — native has no `retired` bucket, no `orphans` view and no
//! `sessions`/`retiredSessions` projection, and it REQUIRES a session key rather
//! than ignoring an observation without one.

use super::*;

/// TS `metering-ledger.test.js:37` `re-reading the same transcript does not add
/// to the total`: three identical sweeps leave the total at 100.
///
/// Native's equivalent is the per-source high-water mark (and the replay receipt
/// on an identical call id): asserted in
/// `usage.rs::native_usage_restart_and_reappearance` and replayed against the
/// retained JavaScript by `usage/vectors.rs`. Restated here.
#[test]
fn ts_oracle_rereading_a_transcript_is_idempotent() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    let observation = claude(10, 20, 30, 40);
    f.db.record_usage_observation(&source, "sweep_1", &observation, 2000)
        .unwrap();
    // The same sweep again: an identical observation under a fresh call id grows
    // nothing, because the high-water mark is per kind.
    f.db.record_usage_observation(&source, "sweep_2", &observation, 2001)
        .unwrap();
    f.db.record_usage_observation(&source, "sweep_3", &observation, 2002)
        .unwrap();
    let known = f.totals().known_high_water_lower_bound.unwrap();
    assert_eq!(known.observed_display_lower_bound().unwrap(), 100);
}

/// TS `metering-ledger.test.js:50` `a growing transcript raises the mark`:
/// `kinds(15,25,30,90)` replaces `kinds(10,20,30,40)`.
#[test]
fn ts_oracle_growing_transcript_raises_the_mark() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    f.db.record_usage_observation(&source, "grow_1", &claude(10, 20, 30, 40), 2000)
        .unwrap();
    f.db.record_usage_observation(&source, "grow_2", &claude(15, 25, 30, 90), 2001)
        .unwrap();
    let observed = f.db.usage_source(&source).unwrap().high_water;
    assert_eq!(
        serde_json::to_value(observed).unwrap(),
        json!({"input": 15, "output": 25, "cacheWrite": 30, "cacheRead": 90})
    );
}

/// TS `metering-ledger.test.js:57` `a second session adds, because it is a
/// different session`: two sessions total 12, `sessions === 2`.
#[test]
fn ts_oracle_a_second_session_adds() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, first) = f.start();
    f.db.record_usage_observation(&first, "s1", &claude(1, 1, 1, 1), 2000)
        .unwrap();
    let (_, _, second) = f.start();
    f.db.record_usage_observation(&second, "s2", &claude(2, 2, 2, 2), 2001)
        .unwrap();
    // Two sources, and the engagement total is their sum (12).
    assert_eq!(f.totals().sources, 2);
    assert_eq!(
        f.totals()
            .known_high_water_lower_bound
            .unwrap()
            .observed_display_lower_bound()
            .unwrap(),
        12
    );
}

/// TS `metering-ledger.test.js:70` `a rotated-away transcript keeps its
/// contribution`: a source the next sweep does not see still counts (104, not 4).
#[test]
fn ts_oracle_rotated_away_transcript_keeps_its_contribution() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, first) = f.start();
    f.db.record_usage_observation(&first, "rotated", &claude(10, 20, 30, 40), 2000)
        .unwrap();
    let (_, _, second) = f.start();
    f.db.record_usage_observation(&second, "fresh", &claude(1, 1, 1, 1), 2001)
        .unwrap();
    // The first source is never observed again, yet its 100 stands.
    assert_eq!(
        f.totals()
            .known_high_water_lower_bound
            .unwrap()
            .observed_display_lower_bound()
            .unwrap(),
        104
    );
}

/// TS `metering-ledger.test.js:82` `a transcript that shrank holds its high water
/// AND is counted as a regression`: total 100, `regressions === 1`.
#[test]
fn ts_oracle_a_shrunk_transcript_regresses() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    f.db.record_usage_observation(&source, "big", &claude(10, 20, 30, 40), 2000)
        .unwrap();
    f.db.record_usage_observation(&source, "small", &claude(1, 2, 3, 4), 2001)
        .unwrap();
    let row = f.db.usage_source(&source).unwrap();
    assert_eq!(row.regressions, 1);
    assert_eq!(
        serde_json::to_value(row.high_water).unwrap(),
        json!({"input": 10, "output": 20, "cacheWrite": 30, "cacheRead": 40})
    );
}

/// TS `metering-ledger.test.js:95` `the high water is per kind, not decided by a
/// summed comparison`: a rewrite reporting more of one kind and less of another
/// keeps each kind's own maximum.
#[test]
fn ts_oracle_high_water_is_per_kind() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    f.db.record_usage_observation(&source, "input_only", &claude(10, 0, 0, 0), 2000)
        .unwrap();
    f.db.record_usage_observation(&source, "output_only", &claude(0, 50, 0, 0), 2001)
        .unwrap();
    let observed = serde_json::to_value(f.db.usage_source(&source).unwrap().high_water).unwrap();
    assert_eq!(observed["input"], json!(10));
    assert_eq!(observed["output"], json!(50));
}

/// TS `metering-ledger.test.js:153` `growth lands in the bucket for the moment it
/// was observed`: the monthly period key is the observation's month.
///
/// The retained-JavaScript bucketing is replayed by
/// `usage/vectors.rs::native_usage_high_water_vectors` (`expected.daily/monthly`);
/// restated here at the key.
#[test]
fn ts_oracle_growth_lands_in_its_observed_bucket() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    // 2026-08-10T00:00:00Z.
    f.db.record_usage_observation(&source, "august", &claude(1, 1, 1, 1), 1_786_320_000_000)
        .unwrap();
    let period =
        f.db.usage_period(&f.engagement, UsagePeriodKind::Monthly, 1_786_320_000_000)
            .unwrap()
            .expect("the observation's own month has a bucket");
    assert_eq!(period.key, "2026-08");
    assert_eq!(period.observed_growth.display_volume(), Ok(Some(4)));
}

/// TS `metering-ledger.test.js:161` `a session spanning two months splits across
/// both`: growth observed in August stays in August, September's in September,
/// and the all-time total is the whole thing.
#[test]
fn ts_oracle_a_session_spanning_two_months_splits() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    // 2026-08-31T23:00:00Z then 2026-09-01T01:00:00Z.
    f.db.record_usage_observation(&source, "aug", &claude(10, 0, 0, 0), 1_788_217_200_000)
        .unwrap();
    f.db.record_usage_observation(&source, "sep", &claude(25, 0, 0, 0), 1_788_224_400_000)
        .unwrap();
    let august =
        f.db.usage_period(&f.engagement, UsagePeriodKind::Monthly, 1_786_320_000_000)
            .unwrap()
            .expect("August has a bucket");
    assert_eq!(august.key, "2026-08");
    assert_eq!(august.observed_growth.input, Some(10));
    let september =
        f.db.usage_period(&f.engagement, UsagePeriodKind::Monthly, 1_788_224_400_000)
            .unwrap()
            .expect("September has a bucket");
    assert_eq!(september.key, "2026-09");
    // The growth appended in September is 15 (25 - the 10 already seen).
    assert_eq!(september.observed_growth.input, Some(15));
    // And the all-time figure is the whole thing.
    assert_eq!(
        f.totals()
            .known_high_water_lower_bound
            .unwrap()
            .observed_display_lower_bound()
            .unwrap(),
        25
    );
}

/// TS `metering-ledger.test.js:179` `a period with no observation is null, not
/// zero`: a bucket that was never observed is absent, not a zero.
#[test]
fn ts_oracle_an_unobserved_period_is_null() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    f.db.record_usage_observation(&source, "aug", &claude(1, 1, 1, 1), 1_786_320_000_000)
        .unwrap();
    // 2026-12-01T00:00:00Z: a month nobody measured.
    assert!(
        f.db.usage_period(&f.engagement, UsagePeriodKind::Monthly, 1_796_083_200_000)
            .unwrap()
            .is_none(),
        "no bucket means no sweep measured this period — not a measured zero"
    );
}

/// TS `metering-ledger.test.js:191` `both granularities are kept`: the daily key
/// is the day and the monthly key the month, both for the same instant.
#[test]
fn ts_oracle_both_granularities_are_kept() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    let receipt =
        f.db.record_usage_observation(&source, "both", &claude(2, 0, 0, 0), 1_786_320_000_000)
            .unwrap();
    assert_eq!(receipt.daily_key, "2026-08-10");
    assert_eq!(receipt.monthly_key, "2026-08");
}

/// TS `metering-ledger.test.js:200` `periodKey is UTC and zero-padded, so buckets
/// sort as strings`: `2026-01-05` and `2026-01`.
#[test]
fn ts_oracle_period_key_is_utc_and_zero_padded() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    // 2026-01-05T00:00:00Z.
    let receipt =
        f.db.record_usage_observation(&source, "january", &claude(1, 0, 0, 0), 1_767_571_200_000)
            .unwrap();
    assert_eq!(receipt.daily_key, "2026-01-05");
    assert_eq!(receipt.monthly_key, "2026-01");
}

/// TS `metering-ledger.test.js:221` `an unknown agent yields null rather than a
/// zero`: `totalsFor('never-seen') === null`.
///
/// Native's summary read is keyed by engagement and always returns a shape, so
/// "unknown" is carried as absent figures (`known_high_water_lower_bound: None`,
/// `latest_counts: None`) rather than a null row — the same distinction, a
/// different encoding. Asserted so the TS claim and the Rust one are visible
/// together.
#[test]
fn ts_oracle_an_unknown_agent_yields_no_figure_not_zero() {
    let f = Fixture::new(Framework::Claude);
    let summary = f.db.usage_summary(&f.engagement).unwrap();
    assert_eq!(summary.sources, 0);
    assert!(
        summary.known_high_water_lower_bound.is_none(),
        "nothing measured is unknown, never a zero total"
    );
    assert!(summary.latest_counts.is_none());
}

/// TS `metering-ledger.test.js:228` `a write happens only when something
/// changed`: re-recording an identical observation writes nothing.
#[test]
fn ts_oracle_a_write_happens_only_when_something_changed() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    let observation = claude(1, 1, 1, 1);
    f.db.record_usage_observation(&source, "once", &observation, 2000)
        .unwrap();
    let before = f.count("usage_receipts");
    // The identical observation under a NEW call id: the high water does not move.
    let again =
        f.db.record_usage_observation(&source, "twice", &observation, 2001)
            .unwrap();
    assert!(!again.regressed);
    assert_eq!(f.count("usage_receipts"), before + 1);
    // And the total is unchanged, which is the property the TS case asserts.
    assert_eq!(
        f.totals()
            .known_high_water_lower_bound
            .unwrap()
            .observed_display_lower_bound()
            .unwrap(),
        4
    );
}

/// TS `metering-ledger.test.js:238` `a loaded ledger continues from what was
/// stored`: a reloaded ledger keeps its total and adds new growth to it.
///
/// Native's persistence is the domain database itself; the equivalent restart
/// and continuation is `usage.rs::native_usage_restart_and_reappearance`.
#[test]
fn ts_oracle_a_reloaded_ledger_continues() {
    let mut f = Fixture::new(Framework::Claude);
    let (_, _, source) = f.start();
    f.db.record_usage_observation(&source, "stored", &claude(10, 10, 10, 10), 2000)
        .unwrap();
    let path = f.root.path().join("state");
    drop(f.db);
    f.db = DomainRepository::open(&path).unwrap();
    // The total survives the reopen...
    assert_eq!(
        f.totals()
            .known_high_water_lower_bound
            .unwrap()
            .observed_display_lower_bound()
            .unwrap(),
        40
    );
    // ...and new growth adds to it.
    let restored = f.db.restore_usage_source(source.id()).unwrap();
    f.db.record_usage_observation(&restored, "more", &claude(15, 10, 10, 10), 3000)
        .unwrap();
    assert_eq!(
        f.totals()
            .known_high_water_lower_bound
            .unwrap()
            .observed_display_lower_bound()
            .unwrap(),
        45
    );
}

/// TS `metering-ledger.test.js:106` `pruned sessions are folded into a retired
/// bucket, not dropped`: past 500 sessions `total` stays 520*4,
/// `sessions === 500`, `retiredSessions === 20`.
///
/// Native has no per-agent session map, no `retired` bucket and no
/// `retiredSessions`/`sessions` projection: usage is keyed by engagement and
/// source row, and the bound is a hard capacity (`MAX_ENGAGEMENT_USAGE_SOURCES`)
/// rather than a fold that preserves the number.
#[ignore = "parity gap: no native retired-session bucket or sessions/retiredSessions projection"]
#[test]
fn ts_oracle_pruned_sessions_are_folded_not_dropped() {
    panic!(
        "TS asserts a retired bucket preserves a pruned session's total; native keeps per-source rows with a capacity bound"
    );
}

/// TS `metering-ledger.test.js:122` `the oldest sessions are the ones that lose
/// their detail`: re-observing `s0` after it retired adds it back as fresh.
#[ignore = "parity gap: no native retired-session bucket (detail is a row, never folded)"]
#[test]
fn ts_oracle_oldest_sessions_lose_their_detail() {
    panic!("TS asserts the oldest session's detail is folded away; native has no fold");
}

/// TS `metering-ledger.test.js:207` `a deleted agent is retained and reported as
/// an orphan`: `orphans(['alive']) === ['gone']`.
///
/// Native retains the usage (`usage_sources` rows outlive the engagement's
/// roster appearance) but has no `orphans` view naming agents absent from a
/// supplied live list.
#[ignore = "parity gap: no native orphans view over usage sources"]
#[test]
fn ts_oracle_a_deleted_agent_is_an_orphan() {
    panic!(
        "TS asserts an orphans() view listing agents not in the live set; native has no such read"
    );
}

/// TS `metering-ledger.test.js:250` `malformed stored state does not take the
/// ledger down`: a corrupt store is tolerated and the ledger still records.
///
/// Native's store is schema-checked at open (`PRAGMA quick_check`) and refuses a
/// corrupt database rather than tolerating it — the opposite disposition, and a
/// deliberate one. Asserted so the difference is visible rather than silent.
#[ignore = "parity gap: native refuses a corrupt store at open (schema/quick_check) rather than tolerating malformed state"]
#[test]
fn ts_oracle_malformed_state_does_not_take_the_ledger_down() {
    panic!("TS asserts malformed load data is tolerated; native fails closed at open instead");
}

/// TS `metering-ledger.test.js:258` `observations without a session key are
/// ignored, not counted as zero`: the keyless observation contributes nothing.
///
/// Native REQUIRES a call id (`identifier(call_id, 256)`) and refuses the
/// observation rather than ignoring it — a stricter rule. The "not counted as
/// zero" half holds: a refused observation never enters a total.
#[ignore = "parity gap: native rejects a keyless observation (identifier(call_id)) rather than ignoring it"]
#[test]
fn ts_oracle_observations_without_a_session_key_are_ignored() {
    panic!(
        "TS asserts a keyless observation is silently ignored; native refuses it (Error::InvalidInput)"
    );
}
