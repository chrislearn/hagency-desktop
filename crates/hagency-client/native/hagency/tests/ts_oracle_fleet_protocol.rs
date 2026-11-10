//! TS oracle: `tests/fleet-protocol.test.js` (+ `bridge-fleet-protocol.test.js`).
//!
//! Each case below carries the TS case title it ports. The native decision
//! surface is `hagency::bootstrap::probe` (`decide` / `receipt_from_event`),
//! ported from `lib/fleet-protocol.js`; the TS test drives the same ladder
//! through `createFleetProtocol(...).handle({path:'/api/fleet/v1/probe'})`.
//!
//! Cases whose TS feature has no native equivalent are recorded as
//! `#[ignore = "parity gap: ..."]` rather than silently dropped; deliberately
//! out-of-scope ones are listed in `.peer/report-62.md` as skipped.
use hagency::bootstrap::probe::{PROBE_EVENT, ProbeError, decide, receipt_from_event};
use hagency_core::authority::Registration;
use serde_json::{Value, json};

const ROOM: &str = "!reception:example.test";
const EVENT: &str = "$probe-event";
const CHALLENGE: &str = "0123456789abcdef0123456789abcdef";

fn registration() -> Registration {
    let fleet = format!("hf_{}", "a".repeat(32));
    Registration {
        fleet_id: fleet.clone(),
        generation: 1,
        server_name: "example.test".into(),
        reception_room_id: String::new(),
        representative_mxid: format!("@{fleet}_representative:example.test"),
        approval_bot_mxid: "@approval:example.test".into(),
    }
}

fn probe_event(reg: &Registration, room: &str, event: &str, challenge: &str) -> Value {
    json!({
        "type": PROBE_EVENT,
        "sender": reg.representative_mxid.clone(),
        "room_id": room,
        "event_id": event,
        "content": {"fleetId": reg.fleet_id.clone(), "challenge": challenge},
    })
}

fn body(reg: &Registration, event: &str, challenge: &str, room: &str) -> Value {
    json!({"fleetId": reg.fleet_id, "sourceEventId": event,
        "challenge": challenge, "sourceRoomId": room})
}

/// Invite-only, unencrypted, representative joined (TS `plaintextPrivate` +
/// the joined_members read).
fn invite_room(reg: &Registration) -> Value {
    json!({
        "joined": {reg.representative_mxid.clone(): {}},
        "join_rules": {"join_rule": "invite"},
        "encryption": null,
    })
}

/// TS: `fleet probe requires exact durable push receipt and never dispatches a
/// task` (`fleet-protocol.test.js:99`). Without the matching delivery receipt
/// the ladder stops at `probe_pending` and nothing is bound.
#[test]
fn ts_fleet_probe_requires_exact_durable_push_receipt() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let body = body(&reg, EVENT, CHALLENGE, ROOM);
    assert_eq!(
        decide(&reg, None, &body, &event, &invite_room(&reg)),
        Err(ProbeError::ProbePending),
        "no receipt means probe_pending, and no task is dispatched"
    );
}

/// TS: `fleet requests reject source target owner and cross-fleet tampering`
/// (`fleet-protocol.test.js:123`) — the fleet-scope half.
#[test]
fn ts_fleet_request_rejects_cross_fleet_tampering() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).expect("valid probe");
    let mut tampered = body(&reg, EVENT, CHALLENGE, ROOM);
    tampered["fleetId"] = json!(format!("hf_{}", "b".repeat(32)));
    assert_eq!(
        decide(&reg, Some(&receipt), &tampered, &event, &invite_room(&reg)),
        Err(ProbeError::WrongFleet),
        "a request scoped to another fleet is refused"
    );
}

/// TS: the probe's `sourceEventId` / `challenge` / `sourceRoomId` must equal
/// the receipt's, or it is still pending (`fleet-protocol.test.js:99,123`).
#[test]
fn ts_fleet_probe_rejects_body_that_does_not_match_the_receipt() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).unwrap();
    for tampered in [
        body(&reg, "$other", CHALLENGE, ROOM),
        body(&reg, EVENT, &"f".repeat(32), ROOM),
        body(&reg, EVENT, CHALLENGE, "!elsewhere:example.test"),
    ] {
        assert_eq!(
            decide(&reg, Some(&receipt), &tampered, &event, &invite_room(&reg)),
            Err(ProbeError::ProbePending),
            "a body that does not match the receipt is not bound: {tampered}"
        );
    }
}

/// TS: re-reading the source event must reproduce the receipt
/// (`fleet-protocol.test.js:115`).
#[test]
fn ts_fleet_probe_reverifies_the_source_event() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).unwrap();
    let other = probe_event(&reg, ROOM, "$another", CHALLENGE);
    assert_eq!(
        decide(
            &reg,
            Some(&receipt),
            &body(&reg, EVENT, CHALLENGE, ROOM),
            &other,
            &invite_room(&reg)
        ),
        Err(ProbeError::ProbeMismatch)
    );
}

/// TS: the representative must remain joined in the reception
/// (`fleet-protocol.test.js:123`, representative_absent).
#[test]
fn ts_fleet_probe_requires_the_representative_joined() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).unwrap();
    let absent = json!({"joined": {}, "join_rules": {"join_rule": "invite"}});
    assert_eq!(
        decide(
            &reg,
            Some(&receipt),
            &body(&reg, EVENT, CHALLENGE, ROOM),
            &event,
            &absent
        ),
        Err(ProbeError::RepresentativeAbsent)
    );
}

/// TS: reception and project rooms must be invite-only and unencrypted
/// (`fleet-protocol.test.js:66`, `plaintextPrivate`).
#[test]
fn ts_fleet_probe_requires_invite_only_unencrypted_room() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).unwrap();
    let body = body(&reg, EVENT, CHALLENGE, ROOM);
    for room in [
        json!({"joined": {reg.representative_mxid.clone(): {}}, "join_rules": {"join_rule": "public"}}),
        json!({"joined": {reg.representative_mxid.clone(): {}}, "join_rules": {"join_rule": "invite"},
               "encryption": {"algorithm": "m.megolm.v1.aes-sha2"}}),
    ] {
        assert_eq!(
            decide(&reg, Some(&receipt), &body, &event, &room),
            Err(ProbeError::RoomPolicy),
            "a room that is not invite-only + plaintext is refused"
        );
    }
}

/// TS: an already-bound registration refuses a different reception
/// (`fleet-protocol.test.js:123`, reception_conflict).
#[test]
fn ts_fleet_probe_refuses_a_second_reception() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).unwrap();
    let mut bound = registration();
    bound.reception_room_id = "!other:example.test".into();
    assert_eq!(
        decide(
            &bound,
            Some(&receipt),
            &body(&bound, EVENT, CHALLENGE, ROOM),
            &event,
            &invite_room(&bound)
        ),
        Err(ProbeError::ReceptionConflict)
    );
}

/// TS: binding the same reception twice is idempotent, and the probe binds the
/// room it read (`fleet-protocol.test.js:115`).
#[test]
fn ts_fleet_probe_binds_the_reception_round_trip() {
    let reg = registration();
    let event = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    let receipt = receipt_from_event(&reg, &event).unwrap();
    let bound = decide(
        &reg,
        Some(&receipt),
        &body(&reg, EVENT, CHALLENGE, ROOM),
        &event,
        &invite_room(&reg),
    )
    .expect("valid probe binds");
    assert_eq!(bound.source_room_id, ROOM);
    assert_eq!(bound.challenge, CHALLENGE);
}

/// TS: probe event shape — sender, challenge bounds, event id
/// (`lib/fleet-protocol.js:105-108`).
#[test]
fn ts_fleet_probe_event_shape_is_enforced() {
    let reg = registration();
    let good = probe_event(&reg, ROOM, EVENT, CHALLENGE);
    assert!(receipt_from_event(&reg, &good).is_some());
    let mut wrong_sender = good.clone();
    wrong_sender["sender"] = json!("@intruder:example.test");
    assert_eq!(receipt_from_event(&reg, &wrong_sender), None);
    assert_eq!(
        receipt_from_event(&reg, &probe_event(&reg, ROOM, EVENT, "short")),
        None
    );
    let mut wrong_type = good.clone();
    wrong_type["type"] = json!("m.room.message");
    assert_eq!(receipt_from_event(&reg, &wrong_type), None);
}

/// TS: `fleet observes project names without trusting request display metadata`
/// (`fleet-protocol.test.js:56`). Native reads the room name from the REAL
/// `m.room.name` state event (`hagency-matrix/src/collector.rs`), so the
/// TS fixture's spoofed request display metadata has no native counterpart to
/// assert against at this seam — the collector test target owns that read.
#[test]
#[ignore = "parity gap: room-name observation is a collector/Matrix concern (hagency-matrix), not the pure probe ladder; needs a Matrix fixture, not probe::decide"]
fn ts_fleet_observes_room_name_without_display_metadata() {}

/// TS: `outbound probe receipt requires configured authenticated edge`
/// (`fleet-protocol.test.js:66`). The probe ladder is entered with the receipt
/// already recorded; the edge/push mode distinction lives in the Matrix
/// collector's configuration, which this pure seam does not model.
#[test]
#[ignore = "parity gap: edge-vs-push probe mode is a Matrix transport/collector concern; probe::decide takes the receipt as given"]
fn ts_fleet_outbound_probe_requires_authenticated_edge() {}

/// TS: `verified request forwards only the exact Matrix authorization context`
/// and `fleet source verification binds the project Agent definition`
/// (`fleet-protocol.test.js:85,115`). Native binds the project binding through
/// the Matrix collector (`com.hagency.admin.binding.v1`) and provisions the
/// agent definition through `bootstrap::provision` — a different seam.
#[test]
#[ignore = "parity gap: fleet request forwarding + agent-definition binding are provision/collector seams (hagency-matrix), not the probe ladder"]
fn ts_fleet_request_forwards_exact_authorization_context() {}

/// TS: `fleet callback routes require their own appservice token and expose no
/// generic proxy` (`fleet-protocol.test.js:148`). This is an HTTP-surface
/// assertion; native's fleet routes are mounted by `bootstrap::fleet` and
/// exercised by the `hagency` fixture harnesses.
#[test]
#[ignore = "parity gap: appservice-token route gating is asserted at the HTTP surface (hagency tests/fixtures), not the pure probe ladder"]
fn ts_fleet_callback_routes_require_their_own_token() {}
