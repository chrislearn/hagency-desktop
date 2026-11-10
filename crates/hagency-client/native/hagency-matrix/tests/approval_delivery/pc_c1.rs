use super::*;
use crate::approval_delivery::public;

/// PC-C1 selector 1: the redacted public status notice is content-free and
/// non-actionable (ADR-137). The exact status packet posts once to the
/// project room through `PublicFrozen`, never to the private room, and a
/// notice addressed from stale or caller-influenced state is refused by the
/// destination re-derivation.
#[tokio::test]
async fn native_private_approval_public_status_notice() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    let card = f.card(1, true).await;
    let request_id = card.target().request_id.clone();

    // The packet is the exact status shape: three top-level keys, a five-key
    // status word, and no request material in any byte.
    let root = "$thread_root:example.test";
    let notice = public::PublicFrozen::new(&card, Some(root.into())).unwrap();
    let content = notice.content().unwrap();
    let value: Value = serde_json::from_str(&content).unwrap();
    let obj = value.as_object().unwrap();
    let status = value[public::STATUS_KEY].as_object().unwrap();
    // TS parity (bridge-matrix.js:2583-2596): the three fixed keys plus the
    // `m.relates_to` thread relation when the approval has a task root.
    assert_eq!(
        obj.len(),
        4,
        "three keys plus the thread relation: {content}"
    );
    assert_eq!(status.len(), 5, "exactly five status keys: {content}");
    assert_eq!(value["msgtype"], public::NOTICE_MSGTYPE);
    assert_eq!(value[public::STATUS_KEY]["state"], "waiting_for_owner");
    assert_eq!(
        value["m.relates_to"],
        json!({"rel_type":"m.thread","event_id":root,"is_falling_back":true,
            "m.in_reply_to":{"event_id":root}})
    );
    for forbidden in [
        "request_id",
        "requestId",
        "digest",
        "tool",
        "tool_name",
        "preview",
        "scope",
        "scope_key",
        "params",
        "command",
        "echo",
    ] {
        assert!(
            !content.contains(forbidden),
            "notice leaked request material `{forbidden}`: {content}"
        );
    }

    // Destination re-derivation: the live authority agrees, and a stale or
    // caller-influenced authority is refused.
    let authority = f
        .base
        .store
        .approval_room_authority(f.base.identity.transport.engagement_id.clone())
        .await
        .unwrap();
    assert!(notice.matches(&authority));
    let mut stale = authority.clone();
    stale.project_room_id = "!caller-influenced:example.test".to_owned();
    assert!(!notice.matches(&stale), "stale destination must be refused");

    // The notice posts exactly once, to the project room, never the private.
    let mut posted = Vec::new();
    drive_with(
        f.collector.send_private_approval_notice(
            card,
            Some(root.into()),
            None,
            &CancellationToken::new(),
        ),
        &mut f.fake,
        &mut f.peer,
        |r, _, _| posted.push((r.method.clone(), r.target.clone(), r.body.clone())),
    )
    .await
    .unwrap();
    assert_eq!(posted.len(), 1, "notice sent exactly once");
    assert_eq!(posted[0].0, "PUT");
    // The exact PUT path TS builds (sendAsAgentContent, bridge-matrix.js:10823):
    // m.room.message is the EVENT TYPE, never the msgtype; the transaction id
    // is deterministic per notice content.
    let target = &posted[0].1;
    let prefix =
        "/_matrix/client/v3/rooms/!project:example.test/send/m.room.message/approval_status_";
    assert!(target.starts_with(prefix), "exact PUT path: {target}");
    assert_eq!(
        target.trim_start_matches(prefix).len(),
        64,
        "transaction id is the content digest: {target}"
    );
    assert!(!target.contains("!private"), "{target}");
    // The exact body TS sends: the TS body text and the status detail under the
    // SHARED approval key, with the thread relation.
    let sent: Value = serde_json::from_slice(&posted[0].2).unwrap();
    assert_eq!(
        sent,
        json!({
            "msgtype": "com.agentchat.approval.status.v1",
            "body": format!("Agent {} is waiting for approval from its owner.",
                authority.agent_name),
            "com.agentchat.approval": {
                "version": 1,
                "kind": "status",
                "agent": authority.agent_name,
                "project": authority.project_id,
                "state": "waiting_for_owner",
            },
            "m.relates_to": {"rel_type":"m.thread","event_id":root,
                "is_falling_back":true,"m.in_reply_to":{"event_id":root}},
        }),
        "exact notice body"
    );

    // Board #99: the room string names the AGENT (its display name), never the
    // engagement id — the live defect read `Agent en_ae6b2f… is waiting …`. The
    // fixture's agent name is `Worker`; its engagement id is `en_<hash>`, so
    // the two are distinguishable and this assertion can fail.
    assert_eq!(authority.agent_name, "Worker", "the agent's own name");
    assert_eq!(
        sent["body"], "Agent Worker is waiting for approval from its owner.",
        "the notice names the agent"
    );
    assert_eq!(sent["com.agentchat.approval"]["agent"], "Worker");
    let body = sent["body"].as_str().unwrap();
    assert!(
        !body.contains(&authority.engagement_id),
        "the engagement id must never reach the room: {body}"
    );
    assert!(
        !content.contains(&authority.engagement_id),
        "no byte of the notice may carry the engagement id"
    );

    // Reading the notice confers no grant and no authority: the request is
    // still pending and no decision was written.
    let summary = f.base.store.approval_summary(request_id).await.unwrap();
    assert_eq!(summary.state, "pending");
    assert_eq!(summary.choice, None);

    f.close().await;
}

/// PC-C1 selector 2: a private send that fails denies the pending request
/// (D-PC-FC). The denial lands in the `owner_approvals` row every surface
/// serves, mints a kind-deny receipt carrying the named reason, and the
/// send is never retried.
#[tokio::test]
async fn native_private_approval_private_failure_denies_pending() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    let card = f.card(1, true).await;
    let request_id = card.target().request_id.clone();

    // Refuse the private send: every room or to-device write fails. The pump
    // must not retry, and the failure must deny — never leave it pending.
    let mut sends = 0usize;
    let result = drive_with(
        f.collector
            .send_private_approval_card(card, &CancellationToken::new()),
        &mut f.fake,
        &mut f.peer,
        |r, _, reply| {
            if r.method == "PUT"
                && (r.target.contains("/send/") || r.target.contains("/sendToDevice/"))
            {
                sends += 1;
                reply.0 = 500;
            }
        },
    )
    .await;
    assert!(result.is_err(), "a failed send must surface as an error");
    assert_eq!(sends, 1, "a failed send is never retried");

    // The denial landed in the row every surface reads.
    let summary = f
        .base
        .store
        .approval_summary(request_id.clone())
        .await
        .unwrap();
    assert_eq!(summary.state, "decided");
    assert_eq!(summary.choice, Some(ApprovalChoice::Deny));

    // The named reason is minted on the kind-deny receipt row.
    let conn = rusqlite::Connection::open(f.base.root.path().join("domain").join("domain.sqlite3"))
        .unwrap();
    let reason: String = conn
        .query_row(
            "SELECT denial_reason FROM approval_verdict_receipts WHERE request_id=?1",
            [&request_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        reason.contains("failed"),
        "reason names the failed send: {reason}"
    );

    f.close().await;
}
