//! Typing indicator and 👀 receipt reaction (TS `bridge-matrix.js:354-369`,
//! `:10514-10668`, `:10884-10885`).
//!
//! The pure wire-shape tests are the ones this sandbox can execute: every
//! SQLite-backed fixture dies at `DomainRepository::open` with `Io(EPERM)`
//! here (the documented sandbox denial, `docs/progress.md:8084-8096`), so the
//! fixture-level lifecycle test below is CI's to run.
mod common;
use common::*;
use hagency_core::replies::RoomPrivacy;
use hagency_matrix::{
    AGENT_ACK_REACTION, AGENT_TYPING_MAX_MS, AGENT_TYPING_REFRESH_MS, AGENT_TYPING_TIMEOUT_MS,
    Collector, HostConfig, HostRoom, ack_request, typing_request,
};
use serde_json::{Value, json};

const ROOM: &str = "!direct:example.test";
const AGENT: &str = "@worker:example.test";
const EVENT: &str = "$human_event:example.test";

/// The ported constants are the TS ones (`bridge-matrix.js:354-369`). A refresh
/// must sit comfortably under the timeout or the notification flickers off.
#[test]
fn native_matrix_presence_constants_match_retained_product() {
    assert_eq!(AGENT_TYPING_TIMEOUT_MS, 45_000);
    assert_eq!(AGENT_TYPING_REFRESH_MS, 30_000);
    assert_eq!(AGENT_TYPING_MAX_MS, 2 * 60_000);
    assert_eq!(AGENT_ACK_REACTION, "\u{1F440}");
    // A refresh must sit comfortably under the timeout or the notification
    // flickers off between refreshes (`bridge-matrix.js:352-353`).
    const { assert!(AGENT_TYPING_REFRESH_MS < AGENT_TYPING_TIMEOUT_MS) };
    const { assert!(AGENT_TYPING_TIMEOUT_MS < AGENT_TYPING_MAX_MS) };
}

/// `setAgentTyping` (`:10527`): ephemeral, addressed as the agent, with a
/// timeout so a crash cannot leave the agent typing forever.
#[test]
fn native_matrix_presence_typing_request_matches_retained_product() {
    let (segments, body) = typing_request(AGENT, ROOM, true);
    assert_eq!(
        segments.join("/"),
        "_matrix/client/v3/rooms/!direct:example.test/typing/@worker:example.test"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"typing": true, "timeout": 45_000})
    );

    let (segments, body) = typing_request(AGENT, ROOM, false);
    assert_eq!(
        segments.join("/"),
        "_matrix/client/v3/rooms/!direct:example.test/typing/@worker:example.test"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"typing": false})
    );
}

/// `ackAgentReceipt` (`:10568`): one `m.annotation` on the human's event, with
/// a transaction id derived from the event so a retry cannot double-react.
#[test]
fn native_matrix_presence_ack_request_matches_retained_product() {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("ack:{EVENT}:{AGENT}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let expected_txn = format!("ack_{}", &hex[..24]);

    let (segments, body) = ack_request(AGENT, ROOM, EVENT);
    assert_eq!(
        segments.join("/"),
        format!("_matrix/client/v3/rooms/{ROOM}/send/m.reaction/{expected_txn}")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({
            "m.relates_to": {
                "rel_type": "m.annotation",
                "event_id": EVENT,
                "key": "\u{1F440}",
            }
        })
    );

    // The transaction id is a function of the event, not of the moment: the
    // same handoff replayed produces the same id, so the homeserver collapses
    // it into one reaction.
    assert_eq!(
        ack_request(AGENT, ROOM, EVENT),
        ack_request(AGENT, ROOM, EVENT)
    );
    assert_ne!(
        ack_request(AGENT, ROOM, "$other"),
        ack_request(AGENT, ROOM, EVENT)
    );
}

fn config(f: &Fixture, endpoint: &str) -> HostConfig {
    HostConfig::new(
        f.identity.clone(),
        endpoint,
        TOKEN,
        f.root.path().join("sdk"),
        [42; 32],
        vec![HostRoom {
            room_id: ROOM.into(),
            generation: 1,
            privacy: RoomPrivacy::Direct {
                human_mxid: "@owner:example.test".into(),
            },
        }],
        common::limits(),
    )
    .unwrap()
    .with_root_pem(include_bytes!("fixtures/ca.pem"))
    .unwrap()
}

/// `beginAgentWork` → `endAgentWork` (`:10603`, `:10634`) over a real fixture:
/// the handoff acknowledges and starts typing as the agent; the agent's reply
/// withdraws the notification. CI-authoritative (see the module note).
#[tokio::test]
async fn native_matrix_presence_work_acknowledges_then_ends() {
    let f = Fixture::new();
    let mut fake = Fake::start(true).await;
    let c = Collector::new(config(&f, &fake.endpoint), f.store.clone()).unwrap();

    let work = c.begin_agent_work(ROOM, EVENT);

    // Acknowledge first, then appear busy — as the agent.
    let request = fake.next().await;
    assert_eq!(request.method, "PUT");
    assert_eq!(
        request.target,
        format!("/_matrix/client/v3/rooms/{ROOM}/send/m.reaction/{}", {
            let (segments, _) = ack_request(AGENT, ROOM, EVENT);
            segments.last().unwrap().clone()
        })
    );
    assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"m.relates_to": {"rel_type": "m.annotation", "event_id": EVENT, "key": "\u{1F440}"}})
    );
    request.json(200, json!({"event_id": "$reaction"}));

    let request = fake.next().await;
    assert_eq!(request.method, "PUT");
    assert_eq!(
        request.target,
        format!("/_matrix/client/v3/rooms/{ROOM}/typing/{AGENT}")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"typing": true, "timeout": 45_000})
    );
    request.json(200, json!({}));

    // The agent spoke: the wait ends and the notification is withdrawn.
    let ((), ()) = common::scripted(work.end(), async {
        let request = fake.next().await;
        assert_eq!(request.method, "PUT");
        assert_eq!(
            request.target,
            format!("/_matrix/client/v3/rooms/{ROOM}/typing/{AGENT}")
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap(),
            json!({"typing": false})
        );
        request.json(200, json!({}));
    })
    .await;
    // Nothing is sent AFTER the end. `end` itself is the last word — no
    // refresh, and no second withdrawal from the dropped handle.
    fake.quiesced(fake.requests(), &common::limits()).await;
}
