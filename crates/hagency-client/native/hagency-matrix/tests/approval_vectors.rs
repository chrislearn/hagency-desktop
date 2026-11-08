//! Approval wire + client-origin oracle pins (brief 29, PC-C4 lane).
//!
//! `tests/fixtures/approval-vectors.json` was recorded by executing the
//! JavaScript implementation's approval producers (`bridge-matrix.js`) and the
//! console proxy's write-origin predicate. The JavaScript implementation has
//! been removed, so the recorded vectors are frozen test data.
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/approval-vectors.json"
    );
    serde_json::from_str(std::fs::read_to_string(path).unwrap().as_str()).unwrap()
}

/// The recorded corpus keeps its provenance and its shape.
#[test]
fn native_approval_oracle_pins_drift() {
    let fixture = fixture();
    assert_eq!(
        fixture["source"].as_str().unwrap(),
        "bridge-matrix.js + mockup/app/api/hagency/[...path]/route.js"
    );
    // The corpus the other selector replays is the regenerated one.
    assert_eq!(fixture["requests"].as_array().unwrap().len(), 5);
    assert_eq!(fixture["notices"].as_array().unwrap().len(), 2);
    assert_eq!(fixture["verdicts"].as_array().unwrap().len(), 16);
    assert_eq!(fixture["origin"].as_array().unwrap().len(), 6);
}

/// The three named `nativeNotes` divergences (ADR-143): each is asserted
/// explicitly, never left implicit in prose. The native-acceptance and
/// packet-refusal halves are bound in-crate (`approval_batch` matcher test,
/// `approval_delivery::state` validator test); this selector binds the
/// fixture side: the rows exist, carry the shapes the notes name, and the
/// cited schema is the contract the notices satisfy.
#[test]
fn native_approval_native_notes_divergences_bound() {
    let fixture = fixture();
    let notes = fixture["nativeNotes"].as_array().unwrap();
    let rules: Vec<&str> = notes.iter().map(|n| n["rule"].as_str().unwrap()).collect();
    assert_eq!(
        rules,
        ["request-id-length", "packet-kind", "public-status-schema"],
        "the fixture's nativeNotes rule list changed"
    );
    // request-id-length: the forty-hex row exists, is forty lowercase hex
    // after the prefix, and the RETAINED oracle refuses it (expected null);
    // native acceptance is the in-crate matcher test.
    let row = fixture["verdicts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "forty-hex-request-id")
        .expect("forty-hex-request-id verdict row");
    assert_eq!(row["expected"], Value::Null, "retained side must refuse it");
    let id = row["event"]["content"]["com.agentchat.approval"]["request_id"]
        .as_str()
        .unwrap();
    let hex = id.strip_prefix("approval_").unwrap();
    assert_eq!(hex.len(), 40);
    assert!(
        hex.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    // packet-kind: every notice row is a status packet under the same event
    // key as a request; `Frozen::validate` refusing it is the in-crate test.
    for notice in fixture["notices"].as_array().unwrap() {
        assert_eq!(
            notice["expected"]["msgtype"],
            "com.agentchat.approval.status.v1"
        );
        assert_eq!(
            notice["expected"]["com.agentchat.approval"]["kind"],
            "status"
        );
    }
    // public-status-schema: the cited schema exists and is the contract —
    // every notice row satisfies its consts and body bound.
    let schema: Value = serde_json::from_str(
        std::fs::read_to_string(root().join("schemas/approval/public-status-v1.schema.json"))
            .expect("cited public-status schema exists")
            .as_str(),
    )
    .unwrap();
    let properties = &schema["properties"];
    assert_eq!(
        properties["msgtype"]["const"],
        "com.agentchat.approval.status.v1"
    );
    assert_eq!(properties["body"]["maxLength"], 512);
    let detail = &properties["com.agentchat.approval"]["properties"];
    assert_eq!(detail["kind"]["const"], "status");
    assert_eq!(detail["state"]["const"], "waiting_for_owner");
    for notice in fixture["notices"].as_array().unwrap() {
        let expected = &notice["expected"];
        assert_eq!(expected["msgtype"], properties["msgtype"]["const"]);
        assert_eq!(
            expected["com.agentchat.approval"]["kind"],
            detail["kind"]["const"]
        );
        assert_eq!(
            expected["com.agentchat.approval"]["state"],
            detail["state"]["const"]
        );
        assert!(expected["body"].as_str().unwrap().len() <= 512);
    }
}
