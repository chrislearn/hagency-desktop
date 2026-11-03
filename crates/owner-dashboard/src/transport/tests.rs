use super::*;
use crate::fixtures::{OWNER, ordinary};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};

/// A private, short directory (Unix socket paths are ~104 bytes on macOS).
struct TempHome(PathBuf);
impl TempHome {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(format!("/tmp/odt-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Handler = Box<dyn FnMut(Value) -> Option<Value> + Send>;

/// Serves frames like the daemon: one reply per request. A handler returning
/// `None` closes the connection without replying (lost response).
fn fake_daemon(home: &Path, mut handler: Handler) -> Arc<Mutex<Vec<Value>>> {
    let listener = UnixListener::bind(home.join(SOCKET)).unwrap();
    std::fs::set_permissions(home.join(SOCKET), std::fs::Permissions::from_mode(0o600)).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            loop {
                let Ok(payload) = read_frame(&mut stream) else { break };
                let request: Value = serde_json::from_slice(&payload).unwrap();
                log.lock().unwrap().push(request.clone());
                match handler(request) {
                    Some(reply) => write_frame(&mut stream, &serde_json::to_vec(&reply).unwrap()).unwrap(),
                    None => break,
                }
            }
        }
    });
    seen
}

fn ok(request: &Value, result: Value) -> Value {
    json!({"version": 1, "requestId": request["requestId"], "revision": 7, "result": result})
}

fn err(request: &Value, code: &str, retryable: bool) -> Value {
    json!({"version": 1, "requestId": request["requestId"], "revision": 7,
        "error": {"code": code, "message": "static text", "retryable": retryable}})
}

fn hello(owner: Option<&str>) -> Value {
    json!({"apiVersion": 1, "dashboardVersion": 1, "ownerId": owner, "server": "example.test", "hosting": "running"})
}

fn wait_events(connector: &DaemonConnector, n: usize) -> Vec<ConnectorEvent> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while out.len() < n && std::time::Instant::now() < deadline {
        match connector.try_recv() {
            Some(e) => out.push(e),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    out
}

#[test]
fn wire_shapes_match_the_daemon_protocol() {
    let request = Request {
        version: API_VERSION,
        request_id: "r-1".into(),
        owner_id: Some("@o:s".into()),
        expected_revision: Some(4),
        command: Command::Snapshot { period: Some(Period::Range { start_ms: 1, end_ms: 2 }), usage_filter: None },
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"version":1,"requestId":"r-1","ownerId":"@o:s","expectedRevision":4,
            "command":{"type":"snapshot","period":{"kind":"range","startMs":1,"endMs":2}}})
    );
    for (command, name) in [
        (Command::PauseHosting, "pauseHosting"),
        (Command::ResumeHosting, "resumeHosting"),
        (Command::StopHosting, "stopHosting"),
        (Command::RefreshProviders, "refreshProviders"),
        (Command::Hello, "hello"),
    ] {
        assert_eq!(serde_json::to_value(&command).unwrap(), json!({"type": name}));
    }
    assert_eq!(serde_json::to_value(Command::Snapshot { period: Some(Period::Week), usage_filter: None }).unwrap(), json!({"type":"snapshot","period":{"kind":"week"}}));
    assert_eq!(Period::from_usage(UsagePeriod::All), None);
    assert_eq!(Period::from_usage(UsagePeriod::Range { start_ms: 5, end_ms: 5 }), None);
    assert_eq!(Period::from_usage(UsagePeriod::Range { start_ms: 1, end_ms: 5 }), Some(Period::Range { start_ms: 1, end_ms: 5 }));
    let outcome: CommandOutcome = serde_json::from_value(json!({"state":"accepted","hosting":"draining","revision":3})).unwrap();
    assert_eq!(command_outcome(&outcome), CommandResult::Accepted);
    assert_eq!(outcome.hosting, HostingState::Draining);
}

#[test]
fn framing_round_trips_and_bounds() {
    let mut buffer = Vec::new();
    write_frame(&mut buffer, b"{}").unwrap();
    assert_eq!(&buffer[..4], &2u32.to_be_bytes());
    assert_eq!(read_frame(&mut buffer.as_slice()).unwrap(), b"{}");
    assert_eq!(write_frame(&mut Vec::new(), &vec![b' '; MAX_FRAME + 1]), Err(FrameError::TooLarge));
    assert_eq!(write_frame(&mut Vec::new(), b""), Err(FrameError::TooLarge));
    let oversized = ((MAX_FRAME as u32) + 1).to_be_bytes();
    assert_eq!(read_frame(&mut oversized.as_slice()), Err(FrameError::TooLarge));
    assert_eq!(read_frame(&mut [0u8, 0, 0, 0].as_slice()), Err(FrameError::TooLarge));
    assert_eq!(read_frame(&mut [0u8, 0].as_slice()), Err(FrameError::Closed));
}

#[test]
fn replies_are_validated_against_the_request() {
    let good = br#"{"version":1,"requestId":"a","revision":1,"result":{}}"#;
    assert!(decode_reply(good, "a").is_ok());
    assert_eq!(decode_reply(good, "b"), Err(TransportError::Protocol));
    let anonymous_error = br#"{"version":1,"requestId":"","revision":0,"error":{"code":"invalid_request","message":"","retryable":false}}"#;
    assert!(decode_reply(anonymous_error, "b").is_ok());
    assert_eq!(decode_reply(br#"{"version":2,"requestId":"a"}"#, "a"), Err(TransportError::Incompatible { version: Some(2) }));
    assert_eq!(
        decode_reply(br#"{"version":1,"requestId":"a","error":{"code":"unsupported_version","message":"","retryable":false}}"#, "a"),
        Err(TransportError::Incompatible { version: None })
    );
    assert_eq!(decode_reply(b"nope", "a"), Err(TransportError::Protocol));
    // Additive daemon fields are tolerated.
    assert!(decode_reply(br#"{"version":1,"requestId":"a","revision":1,"result":{},"future":true}"#, "a").is_ok());
}

#[test]
fn error_codes_map_to_honest_states() {
    let e = |code: &str, retryable| ErrorBody { code: code.into(), message: "m".into(), retryable };
    assert!(matches!(command_error(e("outcome_unknown", true)), CommandResult::Uncertain { .. }));
    assert!(matches!(command_error(e("in_progress", true)), CommandResult::Uncertain { .. }));
    assert!(matches!(command_error(e("invalid_reply", false)), CommandResult::Uncertain { .. }));
    assert_eq!(command_error(e("busy", true)), CommandResult::Rejected { code: "busy".into(), message: "m".into(), retryable: true });
    assert_eq!(status_for_error(&e("owner_scope_mismatch", false)), ConnectionStatus::OwnerMismatch);
    assert_eq!(status_for_transport(&TransportError::Unavailable), ConnectionStatus::DaemonUnavailable);
    assert!(new_request_id("x").chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert!(new_request_id(&"p".repeat(100)).len() <= 64);
}

#[test]
fn missing_or_public_socket_is_refused_before_sending() {
    let home = TempHome::new();
    assert_eq!(DaemonConnection::connect(&DaemonEndpoint::new(&home.0)).err(), Some(TransportError::Unavailable));
    let _listener = UnixListener::bind(home.0.join(SOCKET)).unwrap();
    std::fs::set_permissions(home.0.join(SOCKET), std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(DaemonConnection::connect(&DaemonEndpoint::new(&home.0)).err(), Some(TransportError::Unsafe));
    std::fs::set_permissions(home.0.join(SOCKET), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&home.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(DaemonConnection::connect(&DaemonEndpoint::new(&home.0)).err(), Some(TransportError::Unsafe));
    std::fs::set_permissions(&home.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(DaemonConnection::connect(&DaemonEndpoint::new(&home.0)).is_ok(), "own process passes the peer check");
}

#[test]
fn connector_discovers_owner_reads_snapshot_and_sends_commands() {
    let home = TempHome::new();
    let mut paused = false;
    let seen = fake_daemon(&home.0, Box::new(move |request| {
        Some(match request["command"]["type"].as_str().unwrap() {
            "hello" => ok(&request, hello(Some(OWNER))),
            "snapshot" => ok(&request, serde_json::to_value(ordinary(if paused { HostingState::Paused } else { HostingState::Running })).unwrap()),
            "pauseHosting" => { paused = true; ok(&request, json!({"state":"accepted","hosting":"draining","revision":8})) },
            "stopHosting" => err(&request, "revision_conflict", false),
            _ => err(&request, "not_supported", false),
        })
    }));
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Discover, || {});
    assert!(connector.submit(Job::Snapshot { period: Some(UsagePeriod::Week), usage_filter: None }));
    let events = wait_events(&connector, 2);
    assert_eq!(events[0], ConnectorEvent::Owner(Some(OWNER.into())));
    let ConnectorEvent::Snapshot { snapshot, period } = &events[1] else { panic!("{events:?}") };
    assert_eq!(snapshot.revision, 42);
    assert_eq!(*period, Some(UsagePeriod::Week));

    let ticket = CommandTicket { request_id: "cmd-1".into(), expected_revision: Some(42) };
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: ticket.clone() });
    let events = wait_events(&connector, 2);
    assert_eq!(events[1], ConnectorEvent::CommandResult { request_id: "cmd-1".into(), result: CommandResult::Accepted });
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::Snapshot { .. })));
    connector.submit(Job::Command { kind: CommandKind::Stop, ticket: CommandTicket { request_id: "cmd-2".into(), expected_revision: Some(1) } });
    let events = wait_events(&connector, 2);
    assert!(matches!(&events[1], ConnectorEvent::CommandResult { result: CommandResult::Rejected { code, retryable: false, .. }, .. } if code == "revision_conflict"));

    let log = seen.lock().unwrap();
    let snapshot = log.iter().find(|r| r["command"]["type"] == "snapshot").unwrap();
    assert_eq!(snapshot["ownerId"], OWNER);
    let clock = crate::format::Clock::system();
    let range = UsagePeriod::Week.resolve(clock.now_ms, clock.utc_offset_minutes).unwrap();
    assert_eq!(snapshot["command"]["period"], json!({"kind":"range", "startMs": range.start_ms, "endMs": range.end_ms}));
    let pause = log.iter().find(|r| r["command"]["type"] == "pauseHosting").unwrap();
    assert_eq!(pause["requestId"], "cmd-1");
    assert_eq!(pause["expectedRevision"], 42);
    assert_eq!(pause["ownerId"], OWNER);
}

#[test]
fn lost_reply_is_uncertain_and_lookup_reuses_the_request_id() {
    let home = TempHome::new();
    let mut drop_first_pause = true;
    let seen = fake_daemon(&home.0, Box::new(move |request| {
        match request["command"]["type"].as_str().unwrap() {
            "hello" => Some(ok(&request, hello(Some(OWNER)))),
            "snapshot" => Some(ok(&request, serde_json::to_value(ordinary(HostingState::Running)).unwrap())),
            "pauseHosting" if drop_first_pause => {
                drop_first_pause = false;
                None
            }
            "pauseHosting" => Some(ok(&request, json!({"state":"completed","hosting":"paused","revision":9}))),
            _ => Some(err(&request, "not_supported", false)),
        }
    }));
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Discover, || {});
    let ticket = CommandTicket { request_id: "cmd-lost".into(), expected_revision: Some(42) };
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: ticket.clone() });
    let events = wait_events(&connector, 2);
    assert!(matches!(events.last(), Some(ConnectorEvent::CommandResult { result: CommandResult::Uncertain { .. }, .. })), "{events:?}");
    drop(connector);
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Discover, || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    let recovered = wait_events(&connector, 3);
    assert!(recovered.iter().any(|event| matches!(event, ConnectorEvent::PendingCommand(saved) if saved.ticket == ticket && saved.kind == CommandKind::Pause && saved.phase == CommandPhase::Uncertain)), "{recovered:?}");
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket });
    let events = wait_events(&connector, 2);
    assert!(events.iter().any(|e| *e == ConnectorEvent::CommandResult { request_id: "cmd-lost".into(), result: CommandResult::Completed }), "{events:?}");
    let log = seen.lock().unwrap();
    let pauses: Vec<_> = log.iter().filter(|r| r["command"]["type"] == "pauseHosting").collect();
    assert_eq!(pauses.len(), 2);
    assert!(pauses.iter().all(|r| r["requestId"] == "cmd-lost" && r["expectedRevision"] == 42));
}

#[test]
fn command_after_the_daemons_connection_cap_uses_a_fresh_socket() {
    let home = TempHome::new();
    let listener = UnixListener::bind(home.0.join(SOCKET)).unwrap();
    std::fs::set_permissions(home.0.join(SOCKET), std::fs::Permissions::from_mode(0o600)).unwrap();
    let daemon = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Match the production cap, including the initial hello.
        for n in 0..32 {
            let request: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
            let result = if n == 0 { hello(Some(OWNER)) }
                else { serde_json::to_value(ordinary(HostingState::Running)).unwrap() };
            write_frame(&mut stream, &serde_json::to_vec(&ok(&request, result)).unwrap()).unwrap();
        }
        drop(stream);
        let (mut fresh, _) = listener.accept().unwrap();
        fresh.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let discovery: Value = serde_json::from_slice(&read_frame(&mut fresh).unwrap()).unwrap();
        assert_eq!(discovery["command"]["type"], "hello");
        write_frame(&mut fresh, &serde_json::to_vec(&ok(&discovery, hello(Some(OWNER)))).unwrap()).unwrap();
        let command: Value = serde_json::from_slice(&read_frame(&mut fresh).unwrap()).unwrap();
        assert_eq!(command["requestId"], "after-cap");
        write_frame(&mut fresh, &serde_json::to_vec(&ok(&command, json!({"state":"completed","hosting":"paused","revision":43}))).unwrap()).unwrap();
    });
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    for n in 0..31 {
        assert!(connector.submit(Job::Snapshot { period: None, usage_filter: None }));
        let events = wait_events(&connector, if n == 0 { 2 } else { 1 });
        assert!(matches!(events.last(), Some(ConnectorEvent::Snapshot { .. })));
    }
    assert!(connector.submit(Job::Command { kind: CommandKind::Pause,
        ticket: CommandTicket { request_id: "after-cap".into(), expected_revision: Some(42) } }));
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::CommandResult { result: CommandResult::Completed, .. })));
    daemon.join().unwrap();
}

#[test]
fn fixed_scope_refuses_another_owners_daemon() {
    let home = TempHome::new();
    let seen = fake_daemon(&home.0, Box::new(|request| Some(ok(&request, hello(Some("@someone:example.test"))))));
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    let events = wait_events(&connector, 1);
    assert_eq!(events, vec![ConnectorEvent::Connection(ConnectionStatus::OwnerMismatch)]);
    // Only discovery was sent; no scoped data was requested.
    assert!(seen.lock().unwrap().iter().all(|r| r["command"]["type"] == "hello"));
}

#[test]
fn unenrolled_daemon_yields_an_unconfigured_snapshot() {
    let home = TempHome::new();
    fake_daemon(&home.0, Box::new(|request| Some(ok(&request, hello(None)))));
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Discover, || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    let events = wait_events(&connector, 2);
    let ConnectorEvent::Snapshot { snapshot, .. } = &events[1] else { panic!("{events:?}") };
    assert_eq!(snapshot.hosting.state, HostingState::Unconfigured);
    assert!(snapshot.owner.is_none());
}

#[test]
fn absent_daemon_reports_unavailable_and_commands_are_not_sent() {
    let home = TempHome::new();
    let notified = Arc::new(Mutex::new(0));
    let n = notified.clone();
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Discover, move || *n.lock().unwrap() += 1);
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    assert_eq!(wait_events(&connector, 1), vec![ConnectorEvent::Connection(ConnectionStatus::DaemonUnavailable)]);
    connector.submit(Job::Command { kind: CommandKind::Resume, ticket: CommandTicket { request_id: "c".into(), expected_revision: None } });
    let events = wait_events(&connector, 2);
    assert!(matches!(&events[0], ConnectorEvent::CommandResult { result: CommandResult::NotSubmitted { code, retryable: true }, .. } if code == "not_connected"));
    assert!(*notified.lock().unwrap() >= 3);
}

#[test]
fn incompatible_daemon_version_is_reported() {
    let home = TempHome::new();
    fake_daemon(&home.0, Box::new(|request| Some(json!({"version": 2, "requestId": request["requestId"], "revision": 0}))));
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Discover, || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    assert_eq!(
        wait_events(&connector, 1),
        vec![ConnectorEvent::Connection(ConnectionStatus::Incompatible { daemon_version: Some(2) })]
    );
}

#[test]
fn replies_allow_large_complete_snapshots_but_remain_bounded() {
    let payload=vec![b'x';MAX_FRAME+1];
    let mut frame=(payload.len() as u32).to_be_bytes().to_vec();frame.extend_from_slice(&payload);
    assert!(matches!(read_frame(&mut frame.as_slice()),Err(FrameError::TooLarge)));
    assert_eq!(read_bounded(&mut frame.as_slice(),MAX_REPLY_FRAME).unwrap(),payload);
    let header=((MAX_REPLY_FRAME+1) as u32).to_be_bytes();
    assert!(matches!(read_bounded(&mut header.as_slice(),MAX_REPLY_FRAME),Err(FrameError::TooLarge)));
}

#[test]
fn retiring_after_connect_prevents_an_advisory_or_mutating_write() {
    let (stream,mut peer)=UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut connection=DaemonConnection{stream};
    let retired=std::sync::atomic::AtomicBool::new(true);
    let request=Request{version:API_VERSION,request_id:"retired".into(),owner_id:Some("@o:s".into()),expected_revision:None,command:Command::StopHosting};
    assert_eq!(connection.call_guarded(&request,Duration::from_secs(1),Some(&retired)),Err(TransportError::Unavailable));
    let mut byte=[0];
    assert_eq!(peer.read(&mut byte).unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
}

#[test]
fn command_is_not_sent_when_its_durable_record_cannot_be_saved() {
    let home = TempHome::new();
    let seen = fake_daemon(&home.0, Box::new(|request| Some(ok(&request, hello(Some(OWNER))))));
    std::fs::write(home.0.join("controller-commands"), "not a directory").unwrap();
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: CommandTicket { request_id: "must-not-send".into(), expected_revision: Some(42) } });
    let events = wait_events(&connector, 2);
    assert!(matches!(events.last(), Some(ConnectorEvent::CommandResult { result: CommandResult::NotSubmitted { code, .. }, .. }) if code == "command_recovery_unavailable"), "{events:?}");
    assert!(seen.lock().unwrap().iter().all(|request| request["command"]["type"] == "hello"));
}

#[test]
fn recovered_definitive_refusal_allows_a_fresh_command() {
    let home = TempHome::new();
    let mut first = true;
    let seen = fake_daemon(&home.0, Box::new(move |request| match request["command"]["type"].as_str().unwrap() {
        "hello" => Some(ok(&request, hello(Some(OWNER)))),
        "snapshot" => Some(ok(&request, serde_json::to_value(ordinary(HostingState::Running)).unwrap())),
        "pauseHosting" if first => { first = false; None },
        "pauseHosting" if request["requestId"] == "old-rejected" => Some(err(&request, "revision_conflict", true)),
        "pauseHosting" => Some(ok(&request, json!({"state":"completed","hosting":"paused","revision":43}))),
        _ => None,
    }));
    let ticket = CommandTicket { request_id: "old-rejected".into(), expected_revision: Some(1) };
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: ticket.clone() });
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::CommandResult { result: CommandResult::Uncertain { .. }, .. })));
    drop(connector);
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    assert!(wait_events(&connector, 3).iter().any(|event| matches!(event, ConnectorEvent::PendingCommand(_))));
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket });
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::CommandResult { result: CommandResult::Rejected { code, .. }, .. }) if code == "revision_conflict"));
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: CommandTicket { request_id: "new-after-refusal".into(), expected_revision: Some(42) } });
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::CommandResult { result: CommandResult::Completed, .. })));
    assert!(seen.lock().unwrap().iter().any(|request| request["requestId"] == "new-after-refusal"));
}

#[test]
fn foreign_snapshot_cannot_remove_another_owners_pending_record() {
    let home = TempHome::new();
    let foreign = "@other:example.test";
    let store = crate::command_store::CommandStore::open(&home.0, foreign).unwrap();
    let saved = PendingCommand { kind: CommandKind::Pause, ticket: CommandTicket { request_id: "foreign-pending".into(), expected_revision: Some(41) }, phase: CommandPhase::Accepted };
    store.save(&saved).unwrap();
    drop(store);
    fake_daemon(&home.0, Box::new(move |request| Some(match request["command"]["type"].as_str().unwrap() {
        "hello" => ok(&request, hello(Some(OWNER))),
        _ => {
            let mut snapshot = ordinary(HostingState::Paused);
            snapshot.owner.as_mut().unwrap().user_id = foreign.into();
            ok(&request, serde_json::to_value(snapshot).unwrap())
        }
    })));
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::Connection(ConnectionStatus::OwnerMismatch))));
    assert_eq!(crate::command_store::CommandStore::open(&home.0, foreign).unwrap().load().unwrap(), Some(saved));
}

#[test]
fn restarted_check_busy_keeps_the_original_ticket_and_disables_new_retry() {
    let home = TempHome::new();
    let mut first = true;
    fake_daemon(&home.0, Box::new(move |request| match request["command"]["type"].as_str().unwrap() {
        "hello" => Some(ok(&request, hello(Some(OWNER)))),
        "snapshot" => Some(ok(&request, serde_json::to_value(ordinary(HostingState::Running)).unwrap())),
        "pauseHosting" if first => { first = false; None },
        "pauseHosting" => Some(err(&request, "busy", true)),
        _ => None,
    }));
    let ticket = CommandTicket { request_id: "lost-accepted".into(), expected_revision: Some(42) };
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: ticket.clone() });
    assert!(matches!(wait_events(&connector, 2).last(), Some(ConnectorEvent::CommandResult { result: CommandResult::Uncertain { .. }, .. })));
    drop(connector);
    let connector = DaemonConnector::spawn(DaemonEndpoint::new(&home.0), OwnerScope::Fixed(OWNER.into()), || {});
    connector.submit(Job::Snapshot { period: None, usage_filter: None });
    let mut state = crate::fixtures::scenario("running").unwrap();
    for event in wait_events(&connector, 3) {
        if let ConnectorEvent::PendingCommand(command) = event { state.recover_command(command); }
    }
    assert!(state.check_command().is_some());
    connector.submit(Job::Command { kind: CommandKind::Pause, ticket: ticket.clone() });
    for event in wait_events(&connector, 2) {
        if let ConnectorEvent::CommandResult { request_id, result } = event {
            assert!(matches!(result, CommandResult::Uncertain { .. }));
            state.command_result(&request_id, result, crate::fixtures::FIXTURE_NOW_MS);
        }
    }
    assert_eq!(state.command.as_ref().unwrap().phase, CommandPhase::Uncertain);
    assert_eq!(state.command.as_ref().unwrap().ticket, ticket);
    assert!(state.retry_rejected(crate::fixtures::FIXTURE_NOW_MS).is_none());
    assert_eq!(crate::command_store::CommandStore::open(&home.0, OWNER).unwrap().load().unwrap().unwrap().ticket, ticket);
}
