//! Task #12's end-to-end: the invite poller against the local fake
//! homeserver — the acceptance's "tests with mock homeserver". Three
//! TS-visible outcomes are asserted, each traced from a scripted sync
//! round through `poll_round` to the store:
//!
//! 1. an invitation from anyone but the room's project owner becomes a
//!    PENDING DECISION (`bridge-matrix.js:8051-8063`: remembered, never
//!    joined, never a log line);
//! 2. an invitation from the project's recorded owner takes the
//!    trusted-inviter arm — the agent joins (`:8064-8082`);
//! 3. a console accept queues the join and the next poll round performs
//!    it; a decline leaves best-effort (`:9060-9128`, `:9135-9147`).
#[path = "../../hagency-matrix/tests/common/mod.rs"]
mod matrix;

use hagency::bootstrap::invites::poll_round;
use hagency_matrix::{CancellationToken, Collector};
use matrix::Fixture;
use serde_json::{Value, json};

/// The seeded project's room and owner (`hagency-store/tests/common/
/// mod.rs:24-28`): the store-held trusted-inviter set for these tests.
const PROJECT_ROOM: &str = "!project:example.test";
const OWNER: &str = "@owner:example.test";
/// The agent name the seeded engagement carries.
const AGENT: &str = "Worker";

fn invite_sync(room: &str, inviter: &str) -> Value {
    json!({
        "next_batch": "c1",
        "rooms": {"invite": {room: {"invite_state": {"events": [
            {"type":"m.room.member","state_key":"@worker:example.test","sender":inviter,
             "origin_server_ts": 42, "content": {"membership":"invite"}}
        ]}}}}
    })
}

fn empty_sync() -> Value {
    json!({"next_batch":"c1","rooms":{"join":{}}})
}

#[tokio::test]
async fn untrusted_invite_becomes_a_pending_decision() {
    let f = Fixture::new();
    let mut fake = matrix::Fake::start(false).await;
    let collector = Collector::new(f.config(&fake.endpoint), f.store.clone()).unwrap();
    let cancel = CancellationToken::new();

    let (result, _) = tokio::join!(poll_round(&collector, &f.store, &cancel), async {
        let request = fake.next().await;
        assert_eq!(request.method, "GET");
        assert!(
            request
                .target
                .starts_with("/_matrix/client/v3/sync?timeout=0&filter=")
        );
        request.json(
            200,
            invite_sync("!mystery:example.test", "@stranger:example.test"),
        );
        // No join follows: an untrusted invitation is a decision, not
        // a membership. The fake answers nothing further.
    });
    result.unwrap();
    let list = f.store.pending_invites().await.unwrap();
    assert_eq!(list.len(), 1, "the invitation is recorded exactly once");
    let record = &list[0];
    assert_eq!(record.room_id, "!mystery:example.test");
    assert_eq!(record.agent_name, AGENT);
    assert_eq!(record.inviter.as_deref(), Some("@stranger:example.test"));
    assert_eq!(record.project_server, "example.test");
    assert_eq!(record.state, "pending");
    // The worklists stay empty: nothing was decided.
    assert!(
        f.store
            .join_pending_invites(AGENT.into())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        f.store
            .leave_pending_invites(AGENT.into())
            .await
            .unwrap()
            .is_empty()
    );
    f.store.shutdown().await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn owner_invite_takes_the_trusted_inviter_arm_and_joins() {
    let f = Fixture::new();
    let mut fake = matrix::Fake::start(false).await;
    let collector = Collector::new(f.config(&fake.endpoint), f.store.clone()).unwrap();
    let cancel = CancellationToken::new();

    let (result, _) = tokio::join!(poll_round(&collector, &f.store, &cancel), async {
        let request = fake.next().await;
        assert!(request.target.starts_with("/_matrix/client/v3/sync?"));
        request.json(200, invite_sync(PROJECT_ROOM, OWNER));
        // The join itself, on the agent's own token.
        let request = fake.next().await;
        assert_eq!(request.method, "POST");
        assert!(request.target.contains("/_matrix/client/v3/join/"));
        assert!(request.target.contains("project"));
        assert_eq!(
            request.headers.get("authorization"),
            Some(&format!("Bearer {}", matrix::TOKEN))
        );
        request.json(200, json!({"room_id": PROJECT_ROOM}));
    });
    result.unwrap();
    // Trusted means joined, never a pending decision.
    assert!(f.store.pending_invites().await.unwrap().is_empty());
    assert!(
        f.store
            .join_pending_invites(AGENT.into())
            .await
            .unwrap()
            .is_empty()
    );
    f.store.shutdown().await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn console_accept_queues_the_join_and_the_next_poll_performs_it() {
    let f = Fixture::new();
    let mut fake = matrix::Fake::start(false).await;
    let collector = Collector::new(f.config(&fake.endpoint), f.store.clone()).unwrap();
    let cancel = CancellationToken::new();

    // The decision the console recorded: accepted, join still owed.
    assert!(
        f.store
            .remember_pending_invite(
                "!dm:example.test".into(),
                AGENT.into(),
                Some("@stranger:example.test".into()),
                "direct".into(),
                42,
            )
            .await
            .unwrap()
    );
    f.store
        .settle_pending_invite(
            "!dm:example.test".into(),
            AGENT.into(),
            true,
            false,
            "operator".into(),
        )
        .await
        .unwrap()
        .expect("the record exists");
    assert_eq!(
        f.store
            .join_pending_invites(AGENT.into())
            .await
            .unwrap()
            .len(),
        1
    );

    let (result, _) = tokio::join!(poll_round(&collector, &f.store, &cancel), async {
        let request = fake.next().await;
        assert!(request.target.starts_with("/_matrix/client/v3/sync?"));
        request.json(200, empty_sync());
        let request = fake.next().await;
        assert_eq!(request.method, "POST");
        assert!(request.target.contains("/_matrix/client/v3/join/"));
        request.json(200, json!({"room_id": "!dm:example.test"}));
    });
    result.unwrap();
    // The join is no longer owed, and the record stays the console's
    // decision (accepted by the operator, not re-attributed).
    assert!(
        f.store
            .join_pending_invites(AGENT.into())
            .await
            .unwrap()
            .is_empty()
    );
    let record = f
        .store
        .pending_invite("!dm:example.test".into(), AGENT.into())
        .await
        .unwrap()
        .expect("the record survives its decision");
    assert_eq!(record.state, "accepted");
    assert_eq!(record.decided_by.as_deref(), Some("operator"));
    f.store.shutdown().await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn console_decline_leaves_best_effort_and_the_record_remembers_no() {
    let f = Fixture::new();
    let mut fake = matrix::Fake::start(false).await;
    let collector = Collector::new(f.config(&fake.endpoint), f.store.clone()).unwrap();
    let cancel = CancellationToken::new();

    assert!(
        f.store
            .remember_pending_invite(
                "!dm:example.test".into(),
                AGENT.into(),
                Some("@stranger:example.test".into()),
                "direct".into(),
                42,
            )
            .await
            .unwrap()
    );
    f.store
        .settle_pending_invite(
            "!dm:example.test".into(),
            AGENT.into(),
            false,
            false,
            "operator".into(),
        )
        .await
        .unwrap();
    assert_eq!(
        f.store
            .leave_pending_invites(AGENT.into())
            .await
            .unwrap()
            .len(),
        1
    );

    let (result, _) = tokio::join!(poll_round(&collector, &f.store, &cancel), async {
        let request = fake.next().await;
        assert!(request.target.starts_with("/_matrix/client/v3/sync?"));
        request.json(200, empty_sync());
        let request = fake.next().await;
        assert_eq!(request.method, "POST");
        assert!(request.target.contains("/_matrix/client/v3/rooms/"));
        assert!(request.target.contains("/leave"));
        request.json(200, json!({}));
    });
    result.unwrap();
    assert!(
        f.store
            .leave_pending_invites(AGENT.into())
            .await
            .unwrap()
            .is_empty()
    );
    // "No" is remembered, never deleted — the poll cannot resurrect it.
    let record = f
        .store
        .pending_invite("!dm:example.test".into(), AGENT.into())
        .await
        .unwrap()
        .expect("a decline is remembered");
    assert_eq!(record.state, "declined");
    assert!(f.store.pending_invites().await.unwrap().is_empty());
    f.store.shutdown().await.unwrap();
    fake.close().await;
}

/// ADR-187: the agent's own owner is a trusted inviter even into a room that
/// is no project's (a room the owner created), so the agent joins instead of
/// parking the invitation for the console.
#[tokio::test]
async fn agent_owner_invite_into_a_non_project_room_joins() {
    let f = Fixture::new();
    let mut fake = matrix::Fake::start(false).await;
    let collector = Collector::new(f.config(&fake.endpoint), f.store.clone()).unwrap();
    let cancel = CancellationToken::new();
    let (result, _) = tokio::join!(poll_round(&collector, &f.store, &cancel), async {
        let request = fake.next().await;
        assert!(request.target.starts_with("/_matrix/client/v3/sync?"));
        request.json(200, invite_sync("!side:example.test", OWNER));
        let request = fake.next().await;
        assert_eq!(request.method, "POST");
        assert!(request.target.contains("/_matrix/client/v3/join/"));
        request.json(200, json!({"room_id": "!side:example.test"}));
    });
    result.unwrap();
    assert!(
        f.store.pending_invites().await.unwrap().is_empty(),
        "joined, not parked"
    );
    f.store.shutdown().await.unwrap();
    fake.close().await;
}
