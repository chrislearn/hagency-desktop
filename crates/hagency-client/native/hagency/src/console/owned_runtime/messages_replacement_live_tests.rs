//! Explicit opt-in acceptance against a private real Server/Pasion/Palpo
//! fixture: Agent reports whose send outcome is unknown when the owner signs in
//! again on the same computer. Registering the same installation again keeps
//! the Hagency device and raises its generation, so the Agent gets a new Matrix
//! device with a new crypto store while its outbox stays where it was. The new
//! device settles each report from the room's history (ADR 0013). No model runs.
use super::*;
use crate::console::device_execution::{
    DeviceIdentity, DeviceOperation as Op, DeviceResponse as Response, LeaseRef, Takeover,
};
use crate::console::owned_runtime::{e2ee, send_personal_message, PinnedConsole, StartConfig};
use crate::native_owner::{Command, MatrixAccessToken, MatrixTokenSource, NativeError, NativeOwner};
use matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent;
use matrix_sdk::{
    authentication::matrix::MatrixSession, config::SyncSettings, store::RoomLoadSettings, Client,
    SessionMeta, SessionTokens,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{path::Path, sync::Arc, time::Duration};

const QUIET: &str = "REPLACEMENT_QUIET_REPORT";
const RECENT: &str = "REPLACEMENT_RECENT_REPORT";
const LANDED: &str = "REPLACEMENT_LANDED_REPORT";

/// The host's SDK sign-in. `reenroll` is the host's one-time permission for a
/// new sign-in to register this installation again.
struct Source {
    grant: Value,
    reenroll: AtomicBool,
}
impl MatrixTokenSource for Source {
    fn permits_explicit_reenrollment(&self) -> bool {
        self.reenroll.load(Ordering::Acquire)
    }
    fn consume_explicit_reenrollment(&self) -> bool {
        self.reenroll.swap(false, Ordering::AcqRel)
    }
    fn access_token(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<MatrixAccessToken, NativeError>> + Send + '_>,
    > {
        Box::pin(async {
            Ok(MatrixAccessToken {
                access_token: self.grant["accessToken"].as_str().unwrap().into(),
                client_id: self.grant["clientId"].as_str().unwrap().into(),
            })
        })
    }
}

fn segment(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

async fn matrix(base: &str, token: &str, method: reqwest::Method, path: &str, body: Value) -> Value {
    let mut request = reqwest::Client::new()
        .request(method.clone(), format!("{base}/_matrix/client/v3/{path}"))
        .bearer_auth(token);
    if method != reqwest::Method::GET {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert!(status.is_success(), "isolated Matrix operation failed: {status}");
    value
}

/// One device generation's runtime: its pin, a live lease kept renewed, and
/// the Agent's Matrix device for that generation.
struct Generation {
    console: crate::console::Console,
    transport: PinnedConsole,
    pin: DeviceIdentity,
    lease: LeaseRef,
    heartbeat: tokio::task::JoinHandle<()>,
    device: Arc<e2ee::AgentDevice>,
    message_root: std::path::PathBuf,
}
impl Generation {
    async fn start(
        native: &NativeOwner,
        config: &StartConfig,
        cancel: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        let console = native.test_console().clone();
        let authorized = console.authorized_device().await.unwrap();
        let pin = DeviceIdentity::from(&authorized);
        let transport = PinnedConsole {
            console: console.clone(),
            pin: pin.clone(),
        };
        let Response::History(history) = console
            .execution_api(Op::History {
                agent_id: config.agent_id.clone(),
                cursor: None,
                snapshot: None,
            })
            .await
            .unwrap()
        else {
            panic!("history");
        };
        assert_eq!(history.snapshot.count, 0, "this fixture must never execute a model");
        let Response::Lease(lease) = console
            .execution_api(Op::Acquire {
                agent_id: config.agent_id.clone(),
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
        let (renew_console, renew_lease) = (console.clone(), lease.clone());
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
        let device = e2ee::device(&transport, &lease, config, cancel).await.unwrap();
        let message_root = e2ee::message_root(&transport, &lease, config).unwrap();
        Self {
            console,
            transport,
            pin,
            lease,
            heartbeat,
            device,
            message_root,
        }
    }
    fn matrix_device(&self) -> String {
        self.device.approval_client().device_id().unwrap().to_string()
    }
    async fn end(self) {
        self.heartbeat.abort();
        let _ = self.heartbeat.await;
        self.console
            .execution_api(Op::Release { lease: self.lease })
            .await
            .unwrap();
    }
}

/// Sign out, then sign in again on the same owner root with the host's
/// explicit permission to register this installation again.
async fn sign_in_again(
    native: NativeOwner,
    state: &Path,
    origin: &str,
    owner: &str,
    source: &Arc<Source>,
) -> NativeOwner {
    native.shutdown_checked().await.unwrap();
    // The owner root stays held until this handle is gone.
    drop(native);
    source.reenroll.store(true, Ordering::Release);
    let native = NativeOwner::open_with_matrix(
        state,
        origin,
        owner,
        source.clone() as Arc<dyn MatrixTokenSource>,
    )
    .await
    .unwrap();
    assert!(
        !source.reenroll.load(Ordering::Acquire),
        "the new sign-in spends its permission to register again"
    );
    native
}

/// The server logs the earlier generation's Agent device out: only the new one
/// keeps published keys.
async fn wait_replaced(base: &str, token: &str, puppet: &str, old: &str, new: &str) {
    let until = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let keys = matrix(
            base,
            token,
            reqwest::Method::POST,
            "keys/query",
            json!({"device_keys":{puppet:[]}}),
        )
        .await;
        let devices: Vec<String> = keys["device_keys"][puppet]
            .as_object()
            .map(|devices| devices.keys().cloned().collect())
            .unwrap_or_default();
        if devices.iter().any(|d| d == new) && !devices.iter().any(|d| d == old) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "the server did not replace Agent device {old} with {new}: {devices:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn copies(observed: &Mutex<Vec<(String, String)>>, body: &str) -> Vec<String> {
    observed
        .lock()
        .unwrap()
        .iter()
        .filter(|(text, _)| text == body)
        .map(|(_, event)| event.clone())
        .collect()
}

async fn wait_for_copy(observed: &Mutex<Vec<(String, String)>>, body: &str) {
    let until = tokio::time::Instant::now() + Duration::from_secs(30);
    while copies(observed, body).is_empty() {
        assert!(tokio::time::Instant::now() < until, "{body} never decrypted for the owner");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires HAGENCY_DAEMON_CRYPTO_FIXTURE pointing to a private disposable loopback Server/Pasion/Palpo"]
async fn real_palpo_signing_in_again_settles_unknown_reports_from_room_history() {
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
    let state = root.join("owner");
    let owner = fixture["daemon"]["owner"].as_str().unwrap();
    let human_token = fixture["human"]["accessToken"].as_str().unwrap();
    let origin = format!("{base}/");
    let source = Arc::new(Source {
        grant: fixture["daemon"].clone(),
        reenroll: AtomicBool::new(false),
    });
    let mut native = NativeOwner::open_with_matrix(
        &state,
        &origin,
        owner,
        source.clone() as Arc<dyn MatrixTokenSource>,
    )
    .await
    .unwrap();
    let mut agent = json!({});
    let key = format!(
        "ReportReplacement{}",
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
    let first = Generation::start(&native, &config, &mut cancel).await;

    // The owner's own client receives and decrypts every Agent report.
    let who = matrix(base, human_token, reqwest::Method::GET, "account/whoami", Value::Null).await;
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
    let observed = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let output = observed.clone();
    let sender = puppet.to_owned();
    human.add_event_handler(move |event: OriginalSyncRoomMessageEvent| {
        let output = output.clone();
        let sender = sender.clone();
        async move {
            if event.sender.as_str() == sender && event.content.body().starts_with("REPLACEMENT_") {
                output
                    .lock()
                    .unwrap()
                    .push((event.content.body().to_owned(), event.event_id.to_string()));
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
                "Replacement route anchor; no model execution",
            ),
        )
        .await
        .unwrap()
        .response
        .event_id
        .to_string();
    // As at runtime, the Agent has read the encrypted request before it reports.
    let ready_until = tokio::time::Instant::now() + Duration::from_secs(20);
    while !matches!(first.device.read_candidate(room, &request, owner).await, Ok(Some(_))) {
        assert!(tokio::time::Instant::now() < ready_until, "Agent failed to decrypt the route anchor");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let dispatch = |name: &str| hagency_agent_local::inbox::Dispatch {
        id: format!("replacement_{name}_dispatch"),
        binding_id: config.binding_id.clone(),
        agent_id: agent_id.into(),
        event_id: request.clone(),
        room_id: room.into(),
        requester_mxid: owner.into(),
        thread_root: room.into(),
        body: String::new(),
        state: "completed".into(),
        binding_generation: binding["generation"]
            .as_i64()
            .or_else(|| binding["bindingGeneration"].as_i64())
            .expect("binding generation"),
        dispatch_epoch: Some(first.lease.epoch),
        dispatch_device_id: Some(first.pin.id.clone()),
        execution_id: Some(format!("replacement_{name}_execution")),
        outcome: None,
    };
    let (quiet, recent, landed) = (dispatch("quiet"), dispatch("recent"), dispatch("landed"));
    let execution = |d: &hagency_agent_local::inbox::Dispatch| d.execution_id.clone().unwrap();
    let route = |d: &hagency_agent_local::inbox::Dispatch| Route::personal(d, &execution(d)).unwrap();

    // Generation 1 begins two report sends and stops before either request
    // leaves the host: one began six minutes ago, the other just now.
    let first_device = first.matrix_device();
    let now = attempt_now().unwrap();
    for (d, text, began_ms) in [(&quiet, QUIET, now - 6 * 60 * 1000), (&recent, RECENT, now)] {
        let outbox = Outbox::open_route(&first.message_root, route(d)).unwrap();
        outbox.record(&format!("octos:report:{}", d.id), text).unwrap();
        let pending = outbox.next().unwrap().unwrap();
        assert!(matches!(
            outbox.begin_send(pending.sequence, puppet, &first_device, true, began_ms),
            Ok(Begin::Send(_))
        ));
    }
    let first_pin = first.pin.clone();
    let first_root = first.message_root.clone();
    first.end().await;

    native = sign_in_again(native, &state, &origin, owner, &source).await;
    let second = Generation::start(&native, &config, &mut cancel).await;
    let second_device = second.matrix_device();
    assert_eq!(second.pin.id, first_pin.id, "signing in again keeps the Hagency device");
    assert_eq!(second.pin.generation, first_pin.generation + 1);
    assert_eq!(second.message_root, first_root, "the outbox stays where it was");
    assert_ne!(second_device, first_device, "a new generation gets a new Agent Matrix device");
    wait_replaced(base, human_token, puppet, &first_device, &second_device).await;

    // The quiet attempt: the history holds no copy, so the new device sends it once.
    let outbox = Outbox::open_route(&second.message_root, route(&quiet)).unwrap();
    assert!(
        send_personal_message(&second.transport, &second.lease, &quiet, &execution(&quiet),
            &outbox, &second.device, &mut cancel).await == Ok(true),
        "an attempt the room's history shows never landed is sent from the new device"
    );
    assert!(outbox.next().unwrap().is_none(), "the resend is recorded");
    wait_for_copy(&observed, QUIET).await;

    // The recent attempt: the earlier device's request could still land.
    let outbox = Outbox::open_route(&second.message_root, route(&recent)).unwrap();
    let pending = outbox.next().unwrap().unwrap();
    let Ok(Begin::Reconcile(unsettled)) =
        outbox.begin_send(pending.sequence, puppet, &second_device, true, attempt_now().unwrap())
    else {
        panic!("an attempt from the earlier device is settled from history");
    };
    let (sender, since) = unsettled.scan().unwrap();
    let scan = second.device.report_scan(room, sender, since).await.unwrap();
    assert!(scan.complete, "the scan reaches back before the first attempt");
    assert_eq!(
        outbox.settle(&unsettled, &scan, puppet, &second_device, true, attempt_now().unwrap()),
        Ok(Settled::Unknown("attempt_recent"))
    );
    assert!(
        send_personal_message(&second.transport, &second.lease, &recent, &execution(&recent),
            &outbox, &second.device, &mut cancel).await.is_err(),
        "a recent attempt from the earlier device stays pending"
    );
    assert!(outbox.next().unwrap().is_some());

    // Generation 2's own send lands, but the host stops before recording it.
    let outbox = Outbox::open_route(&second.message_root, route(&landed)).unwrap();
    outbox.record(&format!("octos:report:{}", landed.id), LANDED).unwrap();
    let pending = outbox.next().unwrap().unwrap();
    let Ok(Begin::Send(report_id)) =
        outbox.begin_send(pending.sequence, puppet, &second_device, true, attempt_now().unwrap())
    else {
        panic!("a fresh report starts its first attempt");
    };
    let content = report_content(room, &landed.thread_root, &pending.text, &report_id).unwrap();
    let landed_event = second
        .device
        .send_room_message_if_state_matches(room, &pending.txn, content, true)
        .await
        .unwrap();
    wait_for_copy(&observed, LANDED).await;
    second.end().await;

    native = sign_in_again(native, &state, &origin, owner, &source).await;
    let third = Generation::start(&native, &config, &mut cancel).await;
    let third_device = third.matrix_device();
    assert_eq!(third.pin.generation, first_pin.generation + 2);
    assert_eq!(third.message_root, first_root);
    wait_replaced(base, human_token, puppet, &second_device, &third_device).await;

    // The new device cannot decrypt what the earlier one sent, and an unreadable
    // message from the Agent may be this report: it stays pending, unsent.
    let outbox = Outbox::open_route(&third.message_root, route(&landed)).unwrap();
    let pending = outbox.next().unwrap().unwrap();
    let Ok(Begin::Reconcile(unsettled)) =
        outbox.begin_send(pending.sequence, puppet, &third_device, true, attempt_now().unwrap())
    else {
        panic!("an attempt from the earlier device is settled from history");
    };
    let (sender, since) = unsettled.scan().unwrap();
    let scan = third.device.report_scan(room, sender, since).await.unwrap();
    assert!(
        scan.events.iter().any(|trace| trace.event_id == landed_event && trace.readable.is_none()),
        "the new device cannot read the earlier device's report"
    );
    assert_eq!(verdict(&scan, &report_id, &hash(LANDED)), Verdict::Unknown("history_unreadable"));
    assert!(
        send_personal_message(&third.transport, &third.lease, &landed, &execution(&landed),
            &outbox, &third.device, &mut cancel).await.is_err(),
        "an unreadable possible copy keeps the report pending"
    );
    assert!(outbox.next().unwrap().is_some());

    // Nothing was sent twice, and nothing went out in plaintext.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(copies(&observed, QUIET).len(), 1);
    assert_eq!(copies(&observed, RECENT).len(), 0);
    assert_eq!(copies(&observed, LANDED), vec![landed_event.clone()]);
    for id in copies(&observed, QUIET).iter().chain([&landed_event]) {
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
    third.end().await;
    sync.abort();
    let _ = sync.await;
    native.suspend().await.unwrap();
    println!(
        "PASS: signing in again on the same owner root keeps the Hagency device, raises its generation and replaces the Agent's Matrix device; the outbox stays; a quiet unlanded attempt is sent once from the new device, a recent one stays pending, and a landed but unrecorded report stays pending because the new device cannot read it; no duplicates, no plaintext, no model execution"
    );
}
