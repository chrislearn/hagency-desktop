//! ADR-183 B and C on the private card path: recipients are the owner's
//! devices signed by the pinned anchor and the list may change; a delivery or
//! recipient failure denies the card and fences nothing; a refused enrollment
//! is retried from the top.
use super::*;
use crate::ApprovalRoomRecipients;
use std::collections::BTreeSet;

fn engagement(f: &Fixture) -> String {
    f.base.identity.transport.engagement_id.clone()
}
async fn capture(f: &Fixture) -> ApprovalRoomCapture {
    let authority = f
        .base
        .store
        .approval_room_authority(engagement(f))
        .await
        .unwrap();
    f.base
        .store
        .approval_room_capture(authority)
        .await
        .unwrap()
        .unwrap()
}
async fn census(f: &Fixture) -> ApprovalRoomRecipients {
    f.collector
        .private_approval_delivery_status()
        .await
        .unwrap()
        .rooms[&engagement(f)]
}
async fn denied(f: &Fixture, request_id: &str) {
    let summary = f
        .base
        .store
        .approval_summary(request_id.to_owned())
        .await
        .unwrap();
    assert_eq!(summary.state, "decided");
    assert_eq!(summary.choice, Some(ApprovalChoice::Deny));
}

/// Given an approval enrollment completed with one verified owner device,
/// when the owner adds a second device cross-signed by the pinned identity
/// and a card is sent, the card is encrypted to both devices without a
/// restart: the send itself claims the new device's Olm session.
#[tokio::test]
async fn native_new_signed_device_joins_recipients() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    assert_eq!(f.peer.claims, 1);
    assert_eq!(
        census(&f).await,
        ApprovalRoomRecipients {
            recipients: 1,
            unverified_devices: 0
        }
    );
    f.peer.add_device("HUMAN_TWO", true).await;
    let card = f.card(1, true).await;
    let expected = card.content().clone();
    let mut ciphertext = None;
    let sent = drive_with(
        f.collector
            .send_private_approval_card(card, &CancellationToken::new()),
        &mut f.fake,
        &mut f.peer,
        |r, _, _| {
            if r.method == "PUT" && r.target.contains("/send/m.room.encrypted/") {
                ciphertext = Some(serde_json::from_slice::<Value>(&r.body).unwrap());
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(sent.state, PrivateApprovalDeliveryState::Accepted);
    assert_eq!(f.peer.claims, 2, "one session claim for the new device");
    assert_eq!(
        f.peer.shared_devices,
        BTreeSet::from([crypto::HUMAN_DEVICE.to_owned(), "HUMAN_TWO".to_owned()])
    );
    assert_eq!(f.peer.events.len(), 1);
    assert_eq!(f.peer.events[0]["content"], expected);
    let on_new_device = f
        .peer
        .decrypt_on("HUMAN_TWO", ciphertext.unwrap(), ROOM.try_into().unwrap())
        .await;
    assert_eq!(on_new_device["content"], expected);
    assert_eq!(
        census(&f).await,
        ApprovalRoomRecipients {
            recipients: 2,
            unverified_devices: 0
        }
    );
    assert!(capture(&f).await.available);
    f.close().await;
}

/// Given a completed enrollment, when the owner adds a device the pinned
/// identity has not signed, the enrollment still completes, the card goes to
/// the verified device only, the status counts `unverified_devices=1`, and
/// nothing is fenced.
#[tokio::test]
async fn native_unsigned_device_is_excluded_not_fatal() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    let before = capture(&f).await;
    f.peer.add_device("HUMAN_STRAY", false).await;
    f.enroll().await.unwrap();
    assert_eq!(
        census(&f).await,
        ApprovalRoomRecipients {
            recipients: 1,
            unverified_devices: 1
        }
    );
    let card = f.card(1, true).await;
    let expected = card.content().clone();
    let sent = f.send(card).await.unwrap();
    assert_eq!(sent.state, PrivateApprovalDeliveryState::Accepted);
    // The fixture refuses a room key for any device the owner never signed.
    assert_eq!(
        f.peer.shared_devices,
        BTreeSet::from([crypto::HUMAN_DEVICE.to_owned()])
    );
    assert_eq!(f.peer.events.len(), 1);
    assert_eq!(f.peer.events[0]["content"], expected);
    let after = capture(&f).await;
    assert!(after.available);
    assert_eq!(after.digest, before.digest);
    f.close().await;
}

/// Given an owner whose only devices are unsigned, when a card is sent, the
/// card is refused with Recipients and the request is denied fail-closed,
/// the approval room stays available, and the worker continues: once a
/// verified device is back the next card goes out.
#[tokio::test]
async fn native_no_verified_device_refuses_the_card_only() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    let before = capture(&f).await;
    f.peer.add_device("HUMAN_STRAY", false).await;
    f.peer.park_device(crypto::HUMAN_DEVICE);
    let card = f.card(1, true).await;
    let request_id = card.target().request_id.clone();
    assert_eq!(f.send(card).await, Err(Error::Recipients));
    denied(&f, &request_id).await;
    assert_eq!(f.peer.shares, 0);
    assert!(f.peer.events.is_empty());
    let after = capture(&f).await;
    assert!(after.available, "a recipient refusal fences nothing");
    assert_eq!(after.digest, before.digest);
    assert_eq!(
        census(&f).await,
        ApprovalRoomRecipients {
            recipients: 0,
            unverified_devices: 1
        }
    );
    f.peer.restore_device(crypto::HUMAN_DEVICE);
    let next = f.card(2, true).await;
    let expected = next.content().clone();
    assert_eq!(
        f.send(next).await.unwrap().state,
        PrivateApprovalDeliveryState::Accepted
    );
    assert_eq!(
        f.peer.shared_devices,
        BTreeSet::from([crypto::HUMAN_DEVICE.to_owned()])
    );
    assert_eq!(f.peer.events[0]["content"], expected);
    f.close().await;
}

/// Given an owner whose cross-signing master key differs from the pinned one,
/// when the enrollment verifies, it refuses with Recipients (ADR-137): the
/// anchor is kept, only the device-list equality was dropped.
#[tokio::test]
async fn native_changed_anchor_is_refused() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    let before = capture(&f).await;
    f.peer.reset_identity().await;
    assert_eq!(f.enroll().await, Err(Error::Recipients));
    let card = f.card(1, true).await;
    let request_id = card.target().request_id.clone();
    assert_eq!(f.send(card).await, Err(Error::Recipients));
    denied(&f, &request_id).await;
    assert_eq!(f.peer.shares, 0);
    let after = capture(&f).await;
    assert!(after.available);
    assert_eq!(after.digest, before.digest);
    f.close().await;
}

/// Given an approval room whose send fails, when the card is refused, the
/// request is denied fail-closed, the room row is untouched, and the next
/// card is attempted. A failure before the attempt starts (here the send's
/// own `/keys/query`) leaves no custody behind; a failure of the room PUT
/// itself keeps ADR-112's retained attempt as the custody word — and still
/// touches no room row.
#[tokio::test]
async fn native_delivery_failure_fences_nothing() {
    let mut f = Fixture::new().await;
    f.enroll().await.unwrap();
    let before = capture(&f).await;
    let card = f.card(1, true).await;
    let request_id = card.target().request_id.clone();
    let mut refused = 0usize;
    let result = drive_with(
        f.collector
            .send_private_approval_card(card, &CancellationToken::new()),
        &mut f.fake,
        &mut f.peer,
        |r, _, reply| {
            if r.target.ends_with("/keys/query") {
                refused += 1;
                reply.0 = 500;
            }
        },
    )
    .await;
    assert_eq!(result, Err(Error::Remote(500)));
    assert_eq!(refused, 1);
    denied(&f, &request_id).await;
    let after = capture(&f).await;
    assert!(after.available, "a delivery failure fences nothing");
    assert_eq!(after.digest, before.digest);
    let next = f.card(2, true).await;
    let expected = next.content().clone();
    assert_eq!(
        f.send(next).await.unwrap().state,
        PrivateApprovalDeliveryState::Accepted
    );
    assert_eq!(f.peer.events[0]["content"], expected);
    let third = f.card(3, true).await;
    let request_id = third.target().request_id.clone();
    let mut sends = 0usize;
    let result = drive_with(
        f.collector
            .send_private_approval_card(third, &CancellationToken::new()),
        &mut f.fake,
        &mut f.peer,
        |r, _, reply| {
            if r.method == "PUT" && r.target.contains("/send/m.room.encrypted/") {
                sends += 1;
                reply.0 = 500;
            }
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(sends, 1, "a failed send is never retried");
    denied(&f, &request_id).await;
    let after = capture(&f).await;
    assert!(after.available);
    assert_eq!(after.digest, before.digest);
    let status = f
        .collector
        .private_approval_delivery_status()
        .await
        .unwrap();
    assert_eq!(status.stage, PrivateApprovalDeliveryStage::WritePossible);
    f.fake.quiesced(f.fake.requests(), &fixture::limits()).await;
    common::shutdown_domain(&f.base.store, "delivery-failure-fences-nothing").await;
    f.fake.close().await;
}

/// After `enroll_fresh_account` refuses — a refused verify during the first
/// enrollment, then a transport failure on a later re-verify — calling it
/// again from the top completes once the fact changes; neither refusal
/// fences the approval room.
#[tokio::test]
async fn native_enrollment_retries_after_a_refusal() {
    let mut f = Fixture::new().await;
    let mut refused = false;
    let result = drive_with(
        f.collector.enroll_fresh_account(&CancellationToken::new()),
        &mut f.fake,
        &mut f.peer,
        |r, peer, reply| {
            if r.target.ends_with("/keys/query") && peer.writes.len() == 4 && !refused {
                refused = true;
                reply.1["failures"] = json!({"example.test":{}});
            }
        },
    )
    .await;
    assert!(refused, "the post-upload verify query was refused");
    assert_eq!(result, Err(Error::Recipients));
    assert_eq!(f.peer.claims, 0);
    assert!(
        capture(&f).await.available,
        "a refused enrollment fences nothing"
    );
    // From the top: the record resumes at its refused verify (fresh Query,
    // then Verify), claims its sessions and completes.
    f.enroll().await.unwrap();
    assert_eq!(f.peer.claims, 1);
    assert_eq!(f.peer.writes.len(), 5);
    let mut failed = false;
    let result = drive_with(
        f.collector.enroll_fresh_account(&CancellationToken::new()),
        &mut f.fake,
        &mut f.peer,
        |r, _, reply| {
            if r.target.ends_with("/whoami") && !failed {
                failed = true;
                reply.0 = 500;
            }
        },
    )
    .await;
    assert!(failed);
    assert_eq!(result, Err(Error::Remote(500)));
    assert!(
        capture(&f).await.available,
        "a transport failure fences nothing"
    );
    f.enroll().await.unwrap();
    let card = f.card(1, true).await;
    let expected = card.content().clone();
    assert_eq!(
        f.send(card).await.unwrap().state,
        PrivateApprovalDeliveryState::Accepted
    );
    assert_eq!(f.peer.events[0]["content"], expected);
    f.close().await;
}
