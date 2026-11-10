//! TS test oracle — the seat store (`tests/seat-store.test.js`).
//!
//! The retained TS suite is the parity ORACLE: each test below asserts the SAME
//! observable outcome its TS case asserts. Where native differs — and here it
//! differs for almost the whole file — the test keeps asserting the TS outcome
//! and is `#[ignore]`d with a one-line reason. This task changes no product code.
//!
//! **Why nearly every case is a gap.** `lib/seat-store.js` derives a seat id
//! from an agent record: `authModeOf` (does the runtime profile carry an API
//! key), `seatIdentity` (a digest over `(server, credential home, key scope)`),
//! `buildSeats` (sum one seat's declared ceilings and compare them to a
//! declared quota) and `normalizeDeclaration`. A whole-tree grep finds no
//! native `auth_mode`, `credential_home`, `seat_identity`, `build_seats` or
//! `normalize_declaration`: native treats `seatId` as an **operator-supplied
//! configuration value** on `resources.config` (`Resource::seat_id`) and never
//! derives it. The two cases whose *rule* native does own — the period-matching
//! guard — are asserted below against `hagency_core::allocation::resource_budget`
//! and additionally covered by `fixtures/allocation.json` (29 vectors, replayed
//! by `allocation_vectors_match_javascript`).

use hagency_core::allocation::{Input, resource_budget};
use serde_json::{Value, json};

/// The TS helper `agent(name, over)` defaults: server `local`, type `claude`.
fn agent(server: &str, framework: &str, api_key: Option<Value>) -> Value {
    let runtime_profile = match api_key {
        Some(key) => json!({"primary": {"framework": framework, "apiKey": key}}),
        None => json!({"primary": {"framework": framework, "model": "claude-opus-5"}}),
    };
    json!({"name": "a", "server": server, "type": framework, "runtimeProfile": runtime_profile})
}

/// Build one `resource_budget` input the way `allocation.json` vectors do.
fn budget_input(declaration: Value, commitments: Vec<Value>) -> Input {
    serde_json::from_value(json!({
        "preset": {"id": "p1", "ceiling": {"tokens": 5_000_000, "period": "monthly"}},
        "seatId": "shared",
        "declaration": declaration,
        "commitments": commitments,
    }))
    .expect("a valid allocation input")
}

/* ─────────────────────────── the rule native owns ─────────────────────────── */

/// TS `seat-store.test.js:123` `refuses to compare a quota that states no
/// period`: a declaration with no period is NOT compared to the preset's, so
/// `seat.remaining === null` and nothing is declared as over-subscribed.
///
/// Native owns this rule in `resource_budget`: the `null-versus-missing-period`
/// and `missing-versus-null-period` vectors in `fixtures/allocation.json` pin
/// exactly it (`status: period_mismatch`, `remaining: null`), replayed by
/// `hagency-core/src/allocation.rs::allocation_vectors_match_javascript`. The
/// assertion is restated here so the TS case and the Rust outcome sit side by
/// side rather than only in the corpus.
#[test]
fn ts_oracle_quota_without_a_period_is_not_compared() {
    let budget = resource_budget(&budget_input(
        json!({"quotaTokens": 100}),
        vec![json!({"id": "a", "presetId": "p1", "seatId": "shared",
                    "allocatedTokens": 0, "state": "active"})],
    ))
    .unwrap();
    let value = serde_json::to_value(&budget).unwrap();
    assert_eq!(value["seat"]["quota"], json!(100));
    assert_eq!(value["seat"]["period"], Value::Null);
    assert_eq!(
        value["seat"]["remaining"],
        Value::Null,
        "a quota with no period is never compared, so its headroom is unknown"
    );
    assert_eq!(value["seat"]["status"], json!("period_mismatch"));
}

/// TS `seat-store.test.js:82` `does not add a DAILY ceiling to a MONTHLY one`:
/// only same-period ceilings are summed toward the quota.
///
/// Native owns the period-matching half in `resource_budget` (the 25
/// `quota-*` vectors in `fixtures/allocation.json` cover daily/monthly/null
/// against a monthly preset). The TS case's *aggregation across presets* —
/// `buildSeats` summing members' ceilings per seat — has no native twin, so
/// only the guard is asserted here; the aggregation is a gap below.
#[test]
fn ts_oracle_daily_ceiling_is_not_added_to_monthly() {
    // A monthly declaration against a monthly ceiling is comparable...
    let matched = resource_budget(&budget_input(
        json!({"quotaTokens": 10_000_000, "period": "monthly"}),
        vec![json!({"id": "a", "presetId": "p1", "seatId": "shared",
                    "allocatedTokens": 0, "state": "active"})],
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&matched).unwrap()["seat"]["status"],
        json!("declared")
    );
    // ...and a daily declaration against the same monthly ceiling is not.
    let mismatched = resource_budget(&budget_input(
        json!({"quotaTokens": 10_000_000, "period": "daily"}),
        vec![json!({"id": "a", "presetId": "p1", "seatId": "shared",
                    "allocatedTokens": 0, "state": "active"})],
    ))
    .unwrap();
    let value = serde_json::to_value(&mismatched).unwrap();
    assert_eq!(value["seat"]["status"], json!("period_mismatch"));
    assert_eq!(value["seat"]["remaining"], Value::Null);
}

/* ───────────────────────── auth mode (no native surface) ───────────────────────── */

/// TS `seat-store.test.js:38` `is subscription unless the runtime profile
/// carries an explicit key`: `authModeOf(withModel(...)) === 'subscription'`.
#[ignore = "parity gap: no native authModeOf (seatId is operator-supplied config, never derived from a runtime profile)"]
#[test]
fn ts_oracle_auth_mode_defaults_to_subscription() {
    let row = agent("local", "claude", None);
    panic!(
        "TS asserts authModeOf({row}) == 'subscription'; native has no authModeOf — \
         Resource carries no auth mode and nothing derives one"
    );
}

/// TS `seat-store.test.js:43` `is api-key when a key is set, including the
/// redacted true the API returns`: both `apiKey: 'sk-live'` and `apiKey: true`
/// yield `'api-key'`.
#[ignore = "parity gap: no native authModeOf (no apiKey→auth-mode derivation)"]
#[test]
fn ts_oracle_auth_mode_is_api_key_when_a_key_is_set() {
    let _live = agent("local", "claude", Some(json!("sk-live")));
    let _redacted = agent("local", "claude", Some(json!(true)));
    panic!(
        "TS asserts authModeOf(...) == 'api-key' for both the string key and the redacted `true`; \
         native has no authModeOf"
    );
}

/* ───────────────────────── seat identity (no native surface) ───────────────────────── */

/// TS `seat-store.test.js:50` `two agents on one host, framework and auth mode
/// get the SAME seat`: `a.seatId === b.seatId`.
#[ignore = "parity gap: no native seatIdentity (no server+credential-home+key digest derivation)"]
#[test]
fn ts_oracle_same_host_shares_one_seat() {
    let _a = agent("local", "claude", None);
    let _b = agent("local", "claude", None);
    panic!(
        "TS asserts two agents on one credential home share one seatId; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:58` `the machine's own hostname and "local" are ONE
/// server`: the aliases `local`, `os.hostname()` and `''` all collapse, while
/// a real remote (`mini1.lan`) does not.
#[ignore = "parity gap: no native seatIdentity (no server-alias collapse; native has no server-identity helper on this path)"]
#[test]
fn ts_oracle_local_aliases_collapse_to_one_server() {
    let _local = agent("local", "claude", None);
    let _hostname = agent("mini1.lan", "claude", None);
    panic!(
        "TS asserts local/hostname/empty collapse to ONE seat while a real host does not; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:147` `separates api-key mode from the subscription`:
/// a keyed agent and an unkeyed one on one host are different seats.
#[ignore = "parity gap: no native seatIdentity (no key-scope component in the key)"]
#[test]
fn ts_oracle_api_key_mode_separated_from_subscription() {
    let _keyed = agent("local", "claude", Some(json!("sk-live")));
    let _plain = agent("local", "claude", None);
    panic!(
        "TS asserts api-key and subscription agents on one host are different seats; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:158` `separates credential homes and hosts, not
/// frameworks as such`: claude / codex / remote claude are three distinct
/// seats, with `credentialHome` `~/.claude` and `~/.codex`.
#[ignore = "parity gap: no native seatIdentity (no credential-home table)"]
#[test]
fn ts_oracle_separates_credential_homes_and_hosts() {
    let _claude = agent("local", "claude", None);
    let _codex = agent("local", "codex", None);
    let _remote = agent("box2", "claude", None);
    panic!(
        "TS asserts three distinct seats with credentialHome ~/.claude and ~/.codex; native has no credential-home table"
    );
}

/// TS `seat-store.test.js:171` `gives codex and codex-acp ONE seat, because they
/// share ~/.codex`: same `credentialHome`, same `seatId`.
#[ignore = "parity gap: no native seatIdentity (framework→credential-home map absent)"]
#[test]
fn ts_oracle_codex_and_codex_acp_share_a_seat() {
    let _codex = agent("local", "codex", None);
    let _acp = agent("local", "codex-acp", None);
    panic!(
        "TS asserts codex and codex-acp share one seat via ~/.codex; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:178` `gives two DIFFERENT api keys different seats`,
/// and never exposes the key: neither `seatId` nor `keyScope` contains `sk-aaa`.
#[ignore = "parity gap: no native seatIdentity (no key digest, no keyScope field)"]
#[test]
fn ts_oracle_two_api_keys_are_two_seats() {
    let _aaa = agent("local", "claude", Some(json!("sk-aaa")));
    let _bbb = agent("local", "claude", Some(json!("sk-bbb")));
    panic!(
        "TS asserts two different keys are two seats and expose no key material; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:194` `says when a key split is approximate rather
/// than exact`: the redacted `true` yields `keyScope === 'key:redacted'`.
#[ignore = "parity gap: no native seatIdentity (no keyScope, so no redacted-key reporting)"]
#[test]
fn ts_oracle_redacted_key_is_reported_as_approximate() {
    let _redacted = agent("local", "claude", Some(json!(true)));
    panic!("TS asserts keyScope == 'key:redacted' for a redacted key; native has no keyScope");
}

/// TS `seat-store.test.js:205` `never exposes a path, and marks an unkeyed
/// digest as unkeyed`: `keyed === false` / `keyId` matches `/unkeyed/` without
/// a signing key, `keyed === true` / `keyId === 'k1'` with one, and no seat id
/// ever contains `/`, `home`, `Users` or `.claude`.
#[ignore = "parity gap: no native seatIdentity (no keyed/unkeyed digest, no keyId)"]
#[test]
fn ts_oracle_never_exposes_a_path() {
    let _plain = agent("local", "claude", None);
    panic!(
        "TS asserts keyed/keyId markers and that no seat id carries a path; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:220` `rotating the key changes the id, and the same
/// key reproduces it`: `k1`/`s1` is stable and differs from `k2`/`s2`.
#[ignore = "parity gap: no native seatIdentity (no keyed digest)"]
#[test]
fn ts_oracle_key_rotation_changes_the_id() {
    let _a = agent("local", "claude", None);
    panic!(
        "TS asserts a rotated key changes the seat id and the same key reproduces it; native derives no seat identity"
    );
}

/* ───────────────────────── over-subscription (buildSeats aggregation) ───────────────────────── */

/// TS `seat-store.test.js:236` `sums what has been promised out of one shared
/// seat`: two members of a 5M preset each declare 5M against one seat, so
/// `declaredTokens === 10_000_000` with two members.
#[ignore = "parity gap: no native buildSeats (no per-seat aggregation of member ceilings)"]
#[test]
fn ts_oracle_sums_promises_out_of_one_seat() {
    panic!(
        "TS asserts one seat with declaredTokens 10_000_000 across two members; native computes one resource's budget, not a seat's membership"
    );
}

/// TS `seat-store.test.js:251` `reports over-subscription only when a quota has
/// been declared`: undeclared → `quotaTokens`/`overSubscribed`/`headroomTokens`
/// all null; a tight quota → `true`/`-4_000_000`; a roomy one →
/// `false`/`10_000_000`.
#[ignore = "parity gap: no native buildSeats (no overSubscribed/headroomTokens projection over a seat's members)"]
#[test]
fn ts_oracle_over_subscription_needs_a_declared_quota() {
    panic!(
        "TS asserts null-not-false when no quota is declared and signed headroom otherwise; native has no seat-level projection"
    );
}

/// TS `seat-store.test.js:278` `counts members whose preset carries no ceiling
/// instead of treating them as 0`: `declaredTokens === 5_000_000` with
/// `membersWithoutCeiling === 2`.
#[ignore = "parity gap: no native buildSeats (no membersWithoutCeiling count)"]
#[test]
fn ts_oracle_count_members_without_a_ceiling() {
    panic!(
        "TS asserts membersWithoutCeiling == 2 rather than reading the ceiling sum as the whole story; native has no seat-level projection"
    );
}

/// TS `seat-store.test.js:295` `says on every seat that nothing is enforced`:
/// `enforced === false`.
#[ignore = "parity gap: no native buildSeats (no enforced marker; native computes no seat-level projection at all)"]
#[test]
fn ts_oracle_every_seat_says_nothing_is_enforced() {
    panic!(
        "TS asserts enforced == false on every seat; native has no seat-level projection to carry it"
    );
}

/* ───────────────────────── declarations (normalizeDeclaration) ───────────────────────── */

/// TS `seat-store.test.js:302` `keeps only the fields it understands`:
/// `{quotaTokens, period, planLabel, evil}` → `{quotaTokens, period, planLabel}`.
#[ignore = "parity gap: no native normalizeDeclaration (native stores the operator's Declaration verbatim with no field allowlist)"]
#[test]
fn ts_oracle_declaration_keeps_only_known_fields() {
    panic!(
        "TS asserts an unknown field is dropped from a declaration; native has no normalizeDeclaration"
    );
}

/// TS `seat-store.test.js:307` `rejects a nonsense period and a negative quota
/// rather than storing them`: `period === null`, `quotaTokens === null`.
///
/// Native's `Declaration` parses `quotaTokens` through `Tokens`, which rejects a
/// negative value (see `allocation_rejects_unsafe_token_arithmetic`), so the
/// negative half is covered — but the unknown-period half is not: native stores
/// any period string and only later reports `period_mismatch`.
#[ignore = "parity gap: no native period validator (Declaration accepts any period string; only Tokens rejects a negative quota)"]
#[test]
fn ts_oracle_declaration_rejects_nonsense_values() {
    // The negative-quota half IS native, asserted so the covered half is visible:
    let rejected = serde_json::from_value::<Input>(json!({
        "preset": {"id": "p1"}, "seatId": "shared",
        "declaration": {"quotaTokens": -5, "period": "fortnightly"},
        "commitments": [],
    }));
    assert!(rejected.is_err(), "a negative quota is rejected natively");
    panic!(
        "TS additionally asserts the nonsense period is nulled at store time; native keeps it and only reports period_mismatch later"
    );
}

/// TS `seat-store.test.js:313` `returns null when there is nothing to declare`:
/// `{}` and `{evil: 1}` both → `null`.
#[ignore = "parity gap: no native normalizeDeclaration (no null-when-empty collapse)"]
#[test]
fn ts_oracle_empty_declaration_is_null() {
    panic!(
        "TS asserts an empty or unknown-only declaration normalizes to null; native has no normalizeDeclaration"
    );
}

/* ───────────────────────── digest material (no native surface) ───────────────────────── */

/// TS `seat-store.test.js:320` `distinguishes triples that a naive separator
/// would merge`: `(box:claude, x)` and `(box, claude:x)` produce different ids,
/// as do the quote-separated pair.
#[ignore = "parity gap: no native seatIdentity (no JSON-encoded digest material)"]
#[test]
fn ts_oracle_digest_material_cannot_collide() {
    let _a = agent("box:claude", "x", None);
    let _b = agent("box", "claude:x", None);
    panic!(
        "TS asserts JSON-encoded digest material keeps two triples distinct; native derives no seat identity"
    );
}

/// TS `seat-store.test.js:333` `produces an id with no control characters in it`.
#[ignore = "parity gap: no native seatIdentity (no id to inspect)"]
#[test]
fn ts_oracle_seat_id_has_no_control_characters() {
    let _a = agent("local", "claude", None);
    panic!(
        "TS asserts the generated seat id carries no control characters; native derives no seat identity"
    );
}
