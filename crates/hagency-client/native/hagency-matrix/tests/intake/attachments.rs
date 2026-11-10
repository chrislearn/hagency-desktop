use super::*;
use crate::attachments::Manifest;
use hagency_core::tasks::{DispatchInput, RunnerCapability};

fn file(id: &str, image: bool, mention: bool) -> Value {
    // Independent fixture descriptor; actual room event ciphertext still comes
    // exclusively from real Olm/Megolm SDK encryption and verified key exchange.
    let vectors: Value =
        serde_json::from_str(include_str!("../../../fixtures/media.json")).unwrap();
    let mut descriptor = vectors["vectors"][0]["descriptor"].clone();
    descriptor["url"] = json!("mxc://media.remote/fixture_file");
    json!({"event_id":format!("${id}"),"content":{
        "msgtype":if image {"m.image"} else {"m.file"},"body":"中文报告.pdf","filename":"中文报告.pdf",
        "info":{"mimetype":"application/pdf","size":0},"file":descriptor,
        "m.mentions":{"user_ids":if mention {vec!["@worker:example.test"]} else {vec![]}}
    }})
}
async fn ready(direct: bool) -> (common::Fixture, common::Fake, Collector) {
    let f = common::Fixture::new();
    let mut fake = common::Fake::start(true).await;
    let c = Collector::new(
        config(&f, &fake.endpoint, f.identity.clone(), 1, direct),
        f.store.clone(),
    )
    .unwrap();
    prime(&c, &f, &mut fake, true).await;
    (f, fake, c)
}
async fn packet(c: &Collector, values: Vec<Value>, verified: bool, token: &str) -> Value {
    let mut value = c
        .inner
        .owner
        .lock()
        .await
        .as_ref()
        .unwrap()
        .attachment_fixture(values, verified)
        .await;
    value["next_batch"] = json!(token);
    value
}
async fn manifests(c: &Collector) -> Vec<Manifest> {
    c.inner
        .owner
        .lock()
        .await
        .as_ref()
        .unwrap()
        .attachment_inspect()
        .await
}
async fn restart(
    c: Collector,
    f: &common::Fixture,
    fake: &common::Fake,
    direct: bool,
) -> Collector {
    c.inner
        .owner
        .lock()
        .await
        .take()
        .unwrap()
        .close()
        .await
        .unwrap();
    drop(c);
    Collector::new(
        config(f, &fake.endpoint, f.identity.clone(), 1, direct),
        f.store.clone(),
    )
    .unwrap()
}
async fn cap(f: &common::Fixture) -> RunnerCapability {
    f.store
        .create_canonical_task(
            "attachment_task".into(),
            "root".into(),
            "Read attachment".into(),
            now(),
        )
        .await
        .unwrap();
    let sequence = f
        .store
        .inbox("root".into(), 0, 100, None)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.message.sequence)
        .collect();
    f.store
        .enqueue_inbox_dispatch(
            DispatchInput {
                id: "attachment_run".into(),
                session_id: "root".into(),
                task_id: Some("attachment_task".into()),
                resources: vec![],
                payload: json!({"instruction":"Read original file"}),
            },
            sequence,
        )
        .await
        .unwrap();
    let cap = f
        .store
        .claim_dispatch("attachment_runner".into(), now(), 60_000, 120_000, 1)
        .await
        .unwrap()
        .unwrap();
    f.store.start_dispatch(cap.clone(), now()).await.unwrap();
    cap
}
async fn finish(c: Collector, f: common::Fixture, fake: common::Fake) {
    c.close().await.unwrap();
    f.store.shutdown().await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn native_matrix_attachment_plaintext_room_admitted() {
    // TS parity (bridge-matrix.js:6799-6831): a plaintext room's m.image /
    // m.file is archived like any other message — the room's lack of
    // encryption is not a refusal. Wire shape follows lib/matrix-file.js:38:
    // plaintext carries content.url, not content.file.
    let f = common::Fixture::new();
    let mut fake = common::Fake::start(true).await;
    let c = Collector::new(
        config(&f, &fake.endpoint, f.identity.clone(), 1, false),
        f.store.clone(),
    )
    .unwrap();
    prime(&c, &f, &mut fake, false).await;
    let value = json!({"event_id":"$plain_image","sender":"@owner:example.test","type":"m.room.message","origin_server_ts":now(),
        "content":{"msgtype":"m.image","body":"photo.png","filename":"photo.png",
            "url":"mxc://example.test/photo","info":{"mimetype":"image/png","size":12}}});
    let result = run(&c, &mut fake, sync("plain", vec![value]), false)
        .await
        .unwrap();
    assert_eq!((result.admitted, result.rejected), (1, 0));
    // The message is visible to the agent's session like any timeline event.
    let inbox = f.store.inbox("root".into(), 0, 100, None).await.unwrap();
    assert_eq!(inbox.len(), 1);
    // TS parity: the plaintext attachment is retained in manifest custody too
    // (no crypto device/session), and its metadata reaches the ticket store.
    assert_eq!(rows(&f, "matrix_attachments"), 1);
    let retained = manifests(&c).await;
    assert_eq!(retained.len(), 1);
    assert!(retained[0].content_digest.len() == 64);
    assert_eq!(status(&c, &mut fake).await.stage, "idle");
    fake.quiesced(fake.requests(), &common::limits()).await;
    finish(c, f, fake).await;
}

#[tokio::test]
async fn native_matrix_attachment_verified_metadata() {
    for direct in [true, false] {
        let (f, mut fake, c) = ready(direct).await;
        let value = packet(
            &c,
            vec![file("file", false, false), file("image", true, true)],
            true,
            "files",
        )
        .await;
        let result = run(&c, &mut fake, value, true).await.unwrap();
        assert_eq!((result.admitted, result.rejected), (2, 0));
        let inbox = f.store.inbox("root".into(), 0, 100, None).await.unwrap();
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[0].wake, direct);
        assert!(inbox[1].wake);
        let public = serde_json::to_string(&inbox).unwrap();
        for secret in ["mxc://", "key_ops", "A256CTR", "ciphertext", "fixture_file"] {
            assert!(!public.contains(secret));
        }
        let values = manifests(&c).await;
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|m| m.sdk_identity.len() == 64
            && m.id.len() == 64
            && m.content_digest.len() == 64));
        assert_eq!(rows(&f, "matrix_attachments"), 2);
        assert_eq!(status(&c, &mut fake).await.stage, "idle");
        // Scripted intake consumed only whoami/sync/state. No eager media GET.
        fake.quiesced(fake.requests(), &common::limits()).await;
        finish(c, f, fake).await;
    }
}

#[tokio::test]
async fn native_matrix_attachment_restart_replay() {
    let (f, mut fake, c) = ready(true).await;
    c.inner
        .handoff_fault
        .store(2, std::sync::atomic::Ordering::SeqCst);
    let value = packet(
        &c,
        vec![file("restart", false, false)],
        true,
        "file_restart",
    )
    .await;
    assert_eq!(
        run(&c, &mut fake, value.clone(), true).await,
        Err(Error::OutcomeUnknown)
    );
    assert_eq!(rows(&f, "matrix_attachments"), 1);
    let before = serde_json::to_value(manifests(&c).await).unwrap();
    let c = restart(c, &f, &fake, true).await;
    assert_eq!(resume(&c, &mut fake, true).await.unwrap().replayed, 1);
    assert_eq!(rows(&f, "session_inputs"), 1);
    assert_eq!(serde_json::to_value(manifests(&c).await).unwrap(), before);
    assert_eq!(status(&c, &mut fake).await.stage, "idle");
    let c = restart(c, &f, &fake, true).await;
    assert_eq!(status(&c, &mut fake).await.stage, "idle");
    assert_eq!(serde_json::to_value(manifests(&c).await).unwrap(), before);
    let mut repeated = value;
    repeated["next_batch"] = json!("file_exact_replay");
    let result = run(&c, &mut fake, repeated, true).await.unwrap();
    assert_eq!((result.admitted, result.replayed), (0, 1));
    fake.quiesced(fake.requests(), &common::limits()).await;
    finish(c, f, fake).await;
}

#[tokio::test]
async fn native_matrix_attachment_privacy_refusals() {
    let (f, mut fake, c) = ready(true).await;
    // An unverified owner device is no longer a refusal for an agent (TS
    // parity, `sdk::trust_requirement`); its admission is asserted in
    // `native_matrix_intake_crypto_verified_human_dm_no_mention_and_spoof_refusal`.
    let mut invalid = vec![];
    // "plaintext_url" left this list: TS parity (lib/matrix-file.js:38,
    // `content.file?.url || content.url`) — when an encrypted event also
    // carries a top-level content.url the descriptor wins and the stray url is
    // ignored, so it is no longer a refusal. Asserted positively below.
    for (i, mode) in ["descriptor", "filename", "path", "mime", "size", "oversize"]
        .iter()
        .enumerate()
    {
        let mut value = file(&format!("bad_{i}"), false, false);
        match *mode {
            "descriptor" => value["content"]["file"]["key"]["alg"] = json!("none"),
            "filename" => value["content"]["filename"] = json!("../private"),
            "path" => value["content"]["file"]["url"] = json!("mxc://evil/%2e%2e"),
            "mime" => value["content"]["info"]["mimetype"] = json!("bad\nvalue"),
            "size" => value["content"]["info"]["size"] = json!(-1),
            _ => value["content"]["body"] = json!("x".repeat(20_000)),
        }
        invalid.push(value);
    }
    let value = packet(&c, invalid, true, "malformed_files").await;
    assert_eq!(run(&c, &mut fake, value, true).await.unwrap().rejected, 6);
    let mut plain = file("plain", false, false);
    plain["sender"] = json!("@owner:example.test");
    plain["type"] = json!("m.room.message");
    plain["origin_server_ts"] = json!(now());
    plain["verified"] = json!(true);
    assert_eq!(
        run(&c, &mut fake, sync("plaintext_file", vec![plain]), true)
            .await
            .unwrap()
            .rejected,
        1
    );
    assert!(manifests(&c).await.is_empty());
    assert_eq!(rows(&f, "matrix_attachments"), 0);
    // Seed exactly the old committed Unsupported disposition for an actual
    // verified file, then replay unchanged source under a later sync token.
    c.inner
        .handoff_fault
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let mut old = packet(&c, vec![file("legacy", false, false)], true, "legacy_file").await;
    assert_eq!(
        run(&c, &mut fake, old.clone(), true).await,
        Err(Error::Busy)
    );
    c.inner
        .owner
        .lock()
        .await
        .as_ref()
        .unwrap()
        .attachment_legacy_refusal()
        .await;
    c.inner
        .handoff_fault
        .store(0, std::sync::atomic::Ordering::SeqCst);
    old["next_batch"] = json!("legacy_again");
    let r = run(&c, &mut fake, old, true).await.unwrap();
    assert_eq!((r.admitted, r.rejected), (0, 1));
    assert_eq!(rows(&f, "matrix_attachments"), 0);
    fake.quiesced(fake.requests(), &common::limits()).await;
    finish(c, f, fake).await;
}

#[tokio::test]
async fn native_matrix_attachment_manifest_bounds() {
    let (f, mut fake, c) = ready(true).await;
    let mut replay = None;
    for n in 0..2 {
        let values = (0..64)
            .map(|i| file(&format!("bound_{n}_{i}"), false, false))
            .collect();
        let value = packet(&c, values, true, &format!("bound_batch_{n}")).await;
        replay = Some(value.clone());
        let observation = IntakeHttpObservation::new(n);
        let result = run_observed(&c, &mut fake, value, true, Some(&observation)).await;
        let result = result.unwrap_or_else(|error| {
            panic!(
                "attachment manifest batch {n}, phase {}, original intake result: {error:?}",
                observation.last.get()
            )
        });
        assert_eq!(result.admitted, 64);
    }
    let mut replay = replay.unwrap();
    replay["next_batch"] = json!("full_exact_replay");
    replay["rooms"]["join"]["!project:example.test"]["timeline"]["events"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    assert_eq!(run(&c, &mut fake, replay, true).await.unwrap().replayed, 1);
    let before = serde_json::to_value(manifests(&c).await).unwrap();
    assert_eq!(before.as_array().unwrap().len(), 128);
    let value = packet(
        &c,
        vec![file("overflow", false, false)],
        true,
        "overflow_batch",
    )
    .await;
    let cancel = CancellationToken::new();
    let (result, ()) = tokio::join!(c.intake(plan(), &cancel), async {
        fake.next().await.json(200, common::who());
        fake.next().await.json(200, value);
    });
    assert_eq!(result, Err(Error::Capacity));
    assert_eq!(serde_json::to_value(manifests(&c).await).unwrap(), before);
    assert_eq!(rows(&f, "matrix_attachments"), 128);
    assert_eq!(status(&c, &mut fake).await.stage, "quarantined");
    fake.quiesced(fake.requests(), &common::limits()).await;
    finish(c, f, fake).await;
}

#[tokio::test]
async fn native_matrix_attachment_lookup_scope() {
    let (f, mut fake, mut c) = ready(true).await;
    let value = packet(&c, vec![file("lookup", false, false)], true, "lookup_batch").await;
    run(&c, &mut fake, value, true).await.unwrap();
    let cap = cap(&f).await;
    let ticket = f
        .store
        .authorize_attachment(cap.clone(), "$lookup".into())
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let mut held = vec![];
    for _ in 0..8 {
        held.push(
            c.attachment_manifest(cap.clone(), ticket.clone(), &cancel)
                .await
                .unwrap(),
        );
    }
    assert!(matches!(
        c.attachment_manifest(cap.clone(), ticket.clone(), &cancel)
            .await,
        Err(Error::Capacity)
    ));
    // Old handles outlive the SDK owner. Its reopen cannot reset their pool.
    c.inner
        .owner
        .lock()
        .await
        .take()
        .unwrap()
        .close()
        .await
        .unwrap();
    assert!(matches!(
        c.attachment_manifest(cap.clone(), ticket.clone(), &cancel)
            .await,
        Err(Error::Capacity)
    ));
    let first = held.pop().unwrap();
    assert_eq!(first.media_id().to_mxc(), "mxc://media.remote/fixture_file");
    let descriptor = first
        .descriptor()
        .expect("encrypted-room fixture carries a descriptor")
        .private_event_json()
        .to_vec();
    drop(first);
    let next = c
        .attachment_manifest(cap.clone(), ticket.clone(), &cancel)
        .await
        .unwrap();
    assert_eq!(
        next.descriptor()
            .expect("encrypted-room fixture carries a descriptor")
            .private_event_json(),
        descriptor
    );
    drop(held);
    drop(next);
    let lock = c.inner.owner.lock().await;
    let interrupted = CancellationToken::new();
    let (result, ()) = tokio::join!(
        c.attachment_manifest(cap.clone(), ticket.clone(), &interrupted),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            interrupted.cancel();
        }
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    drop(lock);
    // Use a real configured short deadline while holding the actual owner lock.
    // Only this local read limit changes; production defaults are unchanged.
    std::sync::Arc::get_mut(&mut c.inner)
        .unwrap()
        .config
        .limits
        .sdk = std::time::Duration::from_millis(100);
    let lock = c.inner.owner.lock().await;
    assert!(matches!(
        c.attachment_manifest(cap.clone(), ticket.clone(), &cancel)
            .await,
        Err(Error::Timeout)
    ));
    drop(lock);
    std::sync::Arc::get_mut(&mut c.inner)
        .unwrap()
        .config
        .limits
        .sdk = common::limits().sdk;
    assert!(
        c.attachment_manifest(cap.clone(), ticket.clone(), &cancel)
            .await
            .is_ok()
    );
    let mut wrong = cap.clone();
    wrong.fence += 1;
    assert!(
        c.attachment_manifest(wrong, ticket.clone(), &cancel)
            .await
            .is_err()
    );
    c.inner
        .handoff_fault
        .store(6, std::sync::atomic::Ordering::SeqCst);
    let (result, ()) = tokio::join!(
        c.attachment_manifest(cap.clone(), ticket.clone(), &cancel),
        async {
            c.inner.handoff_reached.notified().await;
            f.store
                .observe_matrix_room(MatrixRoomObservation {
                    engagement_id: f.identity.transport.engagement_id.clone(),
                    registration_generation: 1,
                    transport_generation: 1,
                    room_id: "!project:example.test".into(),
                    generation: 2,
                    privacy: RoomPrivacy::Group {},
                    invite_only: true,
                    encrypted: true,
                    joined: [
                        "@owner:example.test".into(),
                        "@worker:example.test".into(),
                        "@other:example.test".into(),
                    ]
                    .into(),
                })
                .await
                .unwrap();
            c.inner.handoff_continue.notify_one();
        }
    );
    assert!(
        result.is_err(),
        "promotion after private lookup must fence the result"
    );
    c.inner
        .handoff_fault
        .store(0, std::sync::atomic::Ordering::SeqCst);
    assert!(c.attachment_manifest(cap, ticket, &cancel).await.is_err());
    fake.quiesced(fake.requests(), &common::limits()).await;
    finish(c, f, fake).await;
}

#[tokio::test]
async fn native_matrix_attachment_storage_failure() {
    let (f, mut fake, c) = ready(true).await;
    let value = packet(
        &c,
        vec![file("failed_commit", false, false)],
        true,
        "failed_commit",
    )
    .await;
    c.inner
        .owner
        .lock()
        .await
        .as_ref()
        .unwrap()
        .attachment_commit_fault()
        .await;
    let cancel = CancellationToken::new();
    let (result, ()) = tokio::join!(c.intake(plan(), &cancel), async {
        fake.next().await.json(200, common::who());
        fake.next().await.json(200, value);
    });
    assert_eq!(result, Err(Error::OutcomeUnknown));
    assert_eq!(rows(&f, "matrix_attachments"), 0);
    assert_eq!(rows(&f, "admitted_messages"), 0);
    let db =
        rusqlite::Connection::open(f.root.path().join("sdk/matrix-sdk-state.sqlite3")).unwrap();
    db.execute_batch("DROP TRIGGER attachment_commit_abort")
        .unwrap();
    drop(db);
    let c = restart(c, &f, &fake, true).await;
    assert_eq!(status(&c, &mut fake).await.stage, "sdk_outcome_unknown");
    assert!(
        manifests(&c).await.is_empty(),
        "rolled-back manifests cannot appear after reopen"
    );
    // Preserved raw Applying custody must not become an empty successful batch.
    // Failed collection also fenced the exact old device incarnation. Inspection
    // above remains possible, but normal intake refuses before another HTTP call.
    assert_eq!(c.intake(plan(), &cancel).await, Err(Error::Generation));
    fake.quiesced(fake.requests(), &common::limits()).await;
    finish(c, f, fake).await;
}
