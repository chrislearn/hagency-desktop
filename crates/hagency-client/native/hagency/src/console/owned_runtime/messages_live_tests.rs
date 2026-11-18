//! Explicit opt-in acceptance against a private real Server/Pasion/Palpo fixture.
use super::*;
use crate::console::owned_runtime::{e2ee, send_personal_message, PinnedConsole, StartConfig};
use crate::console::device_execution::{
    DeviceIdentity, DeviceOperation as Op, DeviceResponse as Response, Takeover,
};
use crate::native_owner::{Command, MatrixAccessToken, MatrixTokenSource, NativeError, NativeOwner};
use matrix_sdk::{
    Client, SessionMeta, SessionTokens, authentication::matrix::MatrixSession,
    config::SyncSettings, store::RoomLoadSettings,
};
use matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

struct Source(Value);
impl MatrixTokenSource for Source {
    fn access_token(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<MatrixAccessToken, NativeError>> + Send + '_>,
    > {
        Box::pin(async {
            Ok(MatrixAccessToken {
                access_token: self.0["accessToken"].as_str().unwrap().into(),
                client_id: self.0["clientId"].as_str().unwrap().into(),
            })
        })
    }
}
fn segment(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}
async fn matrix(
    base: &str,
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Value,
) -> Value {
    let mut request = reqwest::Client::new()
        .request(method.clone(), format!("{base}/_matrix/client/v3/{path}"))
        .bearer_auth(token);
    if method != reqwest::Method::GET {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert!(
        status.is_success(),
        "isolated Matrix operation failed: {status}"
    );
    value
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires HAGENCY_DAEMON_CRYPTO_FIXTURE pointing to a private disposable loopback Server/Pasion/Palpo"]
async fn real_palpo_delivery_only_report_authorization_and_revocation() {
    hagency_platform::legacy_writer::test_support::initialize_process_root().unwrap();
    let fixture_path =
        std::env::var_os("HAGENCY_DAEMON_CRYPTO_FIXTURE").expect("explicit private fixture");
    let fixture: Value = serde_json::from_reader(
        hagency_store::private::open(Path::new(&fixture_path), false).unwrap(),
    )
    .unwrap();
    let base = fixture["base"].as_str().unwrap();
    let url = reqwest::Url::parse(base).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let temp = hagency_platform::legacy_writer::test_support::initialized_tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let owner = fixture["daemon"]["owner"].as_str().unwrap();
    let human_token = fixture["human"]["accessToken"].as_str().unwrap();
    let origin = format!("{base}/");
    let native = NativeOwner::open_with_matrix(
        &root.join("owner"),
        &origin,
        owner,
        Arc::new(Source(fixture["daemon"].clone())),
    )
    .await
    .unwrap();
    let mut agent = json!({});
    let key = format!(
        "ReportDelivery{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    for _ in 0..100 {
        let value = native
            .execute(Command::CreateAgent {
                input: json!({"displayName":key,"idempotencyKey":key}),
            })
            .await
            .unwrap();
        if value["commandState"] == "active" {
            agent = value["creation"]["agent"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let agent_id = agent["id"].as_str().expect("actual provisioned Agent");
    let puppet = agent["puppetMxid"].as_str().unwrap();
    native
        .execute(Command::EnsureAgentOwnerDirect {
            agent_id: agent_id.into(),
        })
        .await
        .unwrap();
    let mut binding = json!({});
    for _ in 0..100 {
        let value = native
            .execute(Command::Bindings {
                agent_id: agent_id.into(),
            })
            .await
            .unwrap();
        if let Some(found) = value["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["state"] == "active" && b["scopeKind"] == "owner_direct")
        {
            binding = found.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let room = binding["roomId"].as_str().expect("real active binding");
    matrix(
        base,
        human_token,
        reqwest::Method::PUT,
        &format!("rooms/{}/state/m.room.encryption", segment(room)),
        json!({"algorithm":"m.megolm.v1.aes-sha2"}),
    )
    .await;
    let console = native.test_console().clone();
    let device = console.authorized_device().await.unwrap();
    let transport = PinnedConsole {
        console: console.clone(),
        pin: DeviceIdentity::from(&device),
    };
    let Response::History(history) = console
        .execution_api(Op::History {
            agent_id: agent_id.into(),
            cursor: None,
            snapshot: None,
        })
        .await
        .unwrap()
    else {
        panic!("history");
    };
    assert_eq!(
        history.snapshot.count, 0,
        "delivery-only fixture must never execute a model"
    );
    let Response::Lease(lease) = console
        .execution_api(Op::Acquire {
            agent_id: agent_id.into(),
            ttl_ms: 30_000,
            takeover: Takeover::Never,
            history_snapshot: history.snapshot,
        })
        .await
        .unwrap()
    else {
        panic!("lease");
    };
    let lease = lease.reference();
    let renew_console = console.clone();
    let renew_lease = lease.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            renew_console
                .execution_api(Op::Renew {
                    lease: renew_lease.clone(),
                    ttl_ms: 30_000,
                })
                .await
                .unwrap();
        }
    });
    let config = StartConfig {
        agent_id: agent_id.into(),
        binding_id: binding["id"].as_str().unwrap().into(),
        profile: hagency_agent_local::codex::Profile {
            executable: "/usr/bin/false".into(),
            home: root.join("home"),
            codex_home: root.join("provider"),
            cwd: root.join("work"),
            model: "no-model-called".into(),
            effort: "low".into(),
            shared_auth: false,
            web_search: false,
        },
        credential_ref: "no-provider".into(),
        takeover: Takeover::Never,
        host_files: false,
        shared_workspace: false,
        shared_workspace_pin: None,
        owner_direct: true,
        recovery_device_id: None,
        recovery_consent_id: None,
    };
    let (_stop, mut cancel) = tokio::sync::watch::channel(false);
    let agent_device = e2ee::device(&transport, &lease, &config, &mut cancel)
        .await
        .unwrap();
    let who = matrix(
        base,
        human_token,
        reqwest::Method::GET,
        "account/whoami",
        Value::Null,
    )
    .await;
    let human = Client::builder()
        .homeserver_url(base)
        .sqlite_store(root.join("receiver"), Some("disposable-report-receiver"))
        .build()
        .await
        .unwrap();
    human
        .matrix_auth()
        .restore_session(
            MatrixSession {
                meta: SessionMeta {
                    user_id: owner.try_into().unwrap(),
                    device_id: who["device_id"].as_str().unwrap().into(),
                },
                tokens: SessionTokens {
                    access_token: human_token.into(),
                    refresh_token: None,
                },
            },
            RoomLoadSettings::default(),
        )
        .await
        .unwrap();
    let observed = Arc::new(Mutex::new(Vec::<String>::new()));
    let output = observed.clone();
    let sender = puppet.to_owned();
    human.add_event_handler(move |event: OriginalSyncRoomMessageEvent| {
        let output = output.clone();
        let sender = sender.clone();
        async move {
            if event.sender.as_str() == sender && event.content.body() == "DELIVERY_ONLY_REPORT" {
                output.lock().unwrap().push(event.event_id.to_string());
            }
        }
    });
    human
        .sync_once(SyncSettings::default().timeout(Duration::ZERO))
        .await
        .unwrap();
    let syncing = human.clone();
    let sync = tokio::spawn(async move {
        syncing
            .sync(SyncSettings::default().timeout(Duration::from_secs(1)))
            .await
    });
    let room_id: matrix_sdk::ruma::OwnedRoomId = room.try_into().unwrap();
    let request = human
        .get_room(&room_id)
        .unwrap()
        .send(
            matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(
                "Delivery route anchor; no model execution",
            ),
        )
        .await
        .unwrap()
        .response
        .event_id
        .to_string();
    // Match the real runtime prerequisite: the Agent has received and
    // authenticated the encrypted route anchor before it emits any report.
    // No provider work or dispatch admission is needed for this observation.
    let ready_until = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if matches!(
            agent_device.read_candidate(room, &request, owner).await,
            Ok(Some(_))
        ) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < ready_until,
            "Agent failed to decrypt the route anchor"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let dispatch = hagency_agent_local::inbox::Dispatch {
        id: "delivery_only_dispatch".into(),
        binding_id: config.binding_id.clone(),
        agent_id: agent_id.into(),
        event_id: request,
        room_id: room.into(),
        requester_mxid: owner.into(),
        thread_root: room.into(),
        body: String::new(),
        state: "completed".into(),
        binding_generation: binding["generation"]
            .as_i64()
            .or_else(|| binding["bindingGeneration"].as_i64())
            .expect("binding generation"),
        dispatch_epoch: Some(lease.epoch),
        dispatch_device_id: Some(device.device_id().into()),
        execution_id: Some("delivery_only_execution".into()),
        outcome: None,
    };
    let route = Route::personal(&dispatch, "delivery_only_execution").unwrap();
    let message_root = e2ee::message_root(&transport, &lease, &config).unwrap();
    let outbox = Outbox::open_route(&message_root, route.clone()).unwrap();
    for id in [
        "octos:report:live-a",
        "octos:report:live-a",
        "octos:report:live-b",
    ] {
        outbox.record(id, "DELIVERY_ONLY_REPORT").unwrap();
    }
    while outbox.next().unwrap().is_some() {
        send_personal_message(
            &transport,
            &lease,
            &dispatch,
            "delivery_only_execution",
            &outbox,
            &agent_device,
            &mut cancel,
        )
        .await
        .unwrap();
    }
    drop(outbox);
    let recovered = Outbox::open_route(&message_root, route).unwrap();
    recovered
        .record("octos:report:live-a", "DELIVERY_ONLY_REPORT")
        .unwrap();
    assert!(
        recovered.next().unwrap().is_none(),
        "restart retains delivery receipts"
    );
    let limit = tokio::time::Instant::now() + Duration::from_secs(30);
    while observed.lock().unwrap().len() < 2 {
        assert!(
            tokio::time::Instant::now() < limit,
            "reports failed to decrypt"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let events = observed.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0], events[1]);
    for id in &events {
        let wire = matrix(
            base,
            human_token,
            reqwest::Method::GET,
            &format!("rooms/{}/event/{}", segment(room), segment(id)),
            Value::Null,
        )
        .await;
        assert_eq!(wire["type"], "m.room.encrypted");
        assert!(wire["content"]["body"].is_null());
    }
    matrix(
        base,
        human_token,
        reqwest::Method::PUT,
        &format!("rooms/{}/state/m.room.join_rules", segment(room)),
        json!({"join_rule":"public"}),
    )
    .await;
    matrix(
        base,
        human_token,
        reqwest::Method::POST,
        &format!("rooms/{}/leave", segment(room)),
        json!({}),
    )
    .await;
    recovered
        .record("octos:report:revoked", "MUST_NOT_DELIVER_AFTER_LEAVE")
        .unwrap();
    assert!(
        send_personal_message(
            &transport,
            &lease,
            &dispatch,
            "delivery_only_execution",
            &recovered,
            &agent_device,
            &mut cancel
        )
        .await
        .is_err()
    );
    matrix(
        base,
        human_token,
        reqwest::Method::POST,
        &format!("join/{}", segment(room)),
        json!({}),
    )
    .await;
    assert!(
        send_personal_message(
            &transport,
            &lease,
            &dispatch,
            "delivery_only_execution",
            &recovered,
            &agent_device,
            &mut cancel
        )
        .await
        .is_err(),
        "rejoining must not revive the old binding generation"
    );
    assert!(
        recovered.next().unwrap().is_some(),
        "revoked report is retained undelivered"
    );
    heartbeat.abort();
    let _ = heartbeat.await;
    console.execution_api(Op::Release { lease }).await.unwrap();
    sync.abort();
    let _ = sync.await;
    native.suspend().await.unwrap();
    println!(
        "PASS: Desktop encrypted v2 outbox -> delivery-only Server authorization -> real Palpo -> SDK decryption; equal-text identity, duplicate replay, receipt recovery and permanent membership fence; no model execution"
    );
}
