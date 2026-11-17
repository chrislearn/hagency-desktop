use super::continuous::{completed, next_prompt};
use super::*;

/// A parked continuous session: one prompted turn has ended and the stream
/// is idle. This is the state an idle process is retained in.
async fn parked() -> (Driver, Peer) {
    let (mut driver, mut peer) = pair(16 * 1024, limits());
    driver.enable_native_completion().unwrap();
    driver.enable_continuous_session().unwrap();
    initialize(&mut driver, &mut peer).await;
    let first = next_prompt(&mut driver, &mut peer).await;
    completed(&mut driver, &mut peer, &first).await;
    assert!(driver.can_followup());
    (driver, peer)
}
fn lifecycle(command: &str, state: &str) -> Value {
    json!({"type":"command_lifecycle","command_uuid":command,"state":state,"session_id":"session-one"})
}
/// The frames a timer the session set writes when it fires while idle, as the
/// CLI wrote them (2.1.323, offline capture 2026-11-17): no frame names a user
/// message, and the result has no origin.
fn timer_start() -> Vec<Value> {
    let mut started = lifecycle("wake-one", "started");
    started["wakeup_source"] = json!("loop_wakeup");
    started["scheduled_for"] = json!(1_794_939_960_000u64);
    vec![state("running"), started, init()]
}
fn text_result(id: &str, text: &str) -> Value {
    json!({"type":"result","subtype":"success","is_error":false,"result":text,
        "session_id":"session-one","uuid":id,
        "model_usage_since_last_result":{"main":{"inputTokens":17,"outputTokens":9,
            "cacheReadInputTokens":0,"cacheCreationInputTokens":0}},
        "modelUsage":{"main":{"inputTokens":51,"outputTokens":27,
            "cacheReadInputTokens":0,"cacheCreationInputTokens":0}}})
}
async fn write(peer: &mut Peer, frames: impl IntoIterator<Item = Value>) {
    for frame in frames {
        peer.output.write_all(&encoded(frame)).await.unwrap();
    }
}
async fn nothing_written(peer: &mut Peer) {
    let mut line = String::new();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), peer.input.read_line(&mut line))
            .await
            .is_err(),
        "an idle look writes nothing to the CLI"
    );
}

#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_passes_idle_metadata_without_waiting_or_writing() {
    let (mut driver, mut peer) = parked().await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(false));
    // The bounds of the turn that ended do not apply to a look that never waits.
    tokio::time::advance(Duration::from_secs(3600)).await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(false));
    write(
        &mut peer,
        [
            state("idle"),
            lifecycle("old-command", "completed"),
            json!({"type":"rate_limit_event","session_id":"session-one"}),
        ],
    )
    .await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(false));
    assert!(!driver.self_started_turn());
    assert_eq!(driver.phase(), Phase::ResultObserved);
    nothing_written(&mut peer).await;
    // The session is still the one a next message continues.
    let native = async {
        let probe = read(&mut peer).await;
        assert_eq!(probe["request"]["subtype"], "mcp_status");
        peer.output.write_all(&encoded(json!({"type":"control_response","response":{"subtype":"success","request_id":probe["request_id"],"response":{"mcpServers":[]}}}))).await.unwrap();
    };
    let (prepared, ()) = tokio::join!(driver.prepare_followup(limits()), native);
    prepared.unwrap();
    let second = next_prompt(&mut driver, &mut peer).await;
    completed(&mut driver, &mut peer, &second).await;
    assert!(driver.can_followup());
}

#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_reads_a_frame_that_arrives_in_two_parts() {
    let (mut driver, mut peer) = parked().await;
    let bytes = encoded(state("idle"));
    peer.output.write_all(&bytes[..10]).await.unwrap();
    assert_eq!(driver.poll_idle(limits()).await, Ok(false));
    peer.output.write_all(&bytes[10..]).await.unwrap();
    assert_eq!(driver.poll_idle(limits()).await, Ok(false));
    assert!(driver.can_followup());
}

#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_takes_over_a_timer_turn_long_after_the_last_one() {
    let (mut driver, mut peer) = pair(16 * 1024, limits());
    driver.enable_native_completion().unwrap();
    driver.enable_continuous_session().unwrap();
    initialize(&mut driver, &mut peer).await;
    let first = next_prompt(&mut driver, &mut peer).await;
    let parked_source = driver.observation_source().unwrap();
    completed(&mut driver, &mut peer, &first).await;
    // Far beyond the lifetime of the turn that ended.
    tokio::time::advance(Duration::from_secs(7200)).await;
    write(&mut peer, timer_start()).await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(true));
    assert!(driver.self_started_turn());
    assert_eq!(driver.phase(), Phase::Running);
    assert!(!driver.can_followup());
    assert!(parked_source.is_retired());
    // A second look is not a way to read an open turn.
    assert_eq!(driver.poll_idle(limits()).await, Err(Error::State));
    assert_eq!(driver.phase(), Phase::Running);

    let first = driver.next_message().await.unwrap();
    assert!(matches!(
        first,
        Message::Event { kind: EventKind::System, ref payload, .. } if payload["subtype"] == "init"
    ));
    assert_eq!(driver.session_id(), Some("session-one"));
    driver.supervise_turn().unwrap();
    assert!(
        driver.prepare_steer("added").is_err(),
        "no prompt of this host to add a message to"
    );
    assert_eq!(driver.phase(), Phase::Running);
    event(
        &mut driver,
        &mut peer,
        json!({"type":"system","subtype":"status","status":"requesting","session_id":"session-one"}),
    )
    .await
    .unwrap();
    event(&mut driver, &mut peer, text_result("result-timer", "Timer report"))
        .await
        .unwrap();
    assert!(!driver.can_followup(), "a result alone is not native idle");
    event(&mut driver, &mut peer, lifecycle("wake-one", "completed"))
        .await
        .unwrap();
    event(&mut driver, &mut peer, state("idle")).await.unwrap();
    assert_eq!(driver.phase(), Phase::ResultObserved);
    let receipt = driver.completion_receipt().unwrap();
    assert!(receipt.matches_text("Timer report"));
    assert!(!receipt.is_error());
    assert!(driver.can_followup(), "the session is parked again");
    nothing_written(&mut peer).await;

    // Parked again: the next timer is another turn of its own.
    write(&mut peer, timer_start()).await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(true));
    driver.next_message().await.unwrap();
    event(&mut driver, &mut peer, text_result("result-timer-two", "Second report"))
        .await
        .unwrap();
    event(&mut driver, &mut peer, state("idle")).await.unwrap();
    assert!(driver.completion_receipt().unwrap().matches_text("Second report"));
}

#[tokio::test(start_paused = true)]
async fn native_claude_self_started_turn_reads_a_peer_cycle_that_follows_without_idle() {
    let (mut driver, mut peer) = parked().await;
    write(&mut peer, timer_start()).await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(true));
    driver.next_message().await.unwrap();
    event(&mut driver, &mut peer, text_result("result-timer", "Timer report"))
        .await
        .unwrap();
    event(&mut driver, &mut peer, lifecycle("wake-one", "completed"))
        .await
        .unwrap();
    // A peer message that arrived meanwhile runs at once, with no idle between.
    event(&mut driver, &mut peer, lifecycle("peer-one", "started"))
        .await
        .unwrap();
    event(&mut driver, &mut peer, init()).await.unwrap();
    event(&mut driver, &mut peer, state("idle")).await.unwrap();
    assert_eq!(
        driver.phase(),
        Phase::Running,
        "idle does not end a turn whose second cycle has no result yet"
    );
    event(&mut driver, &mut peer, state("running")).await.unwrap();
    let mut peer_result = text_result("result-peer", "Answer to the peer");
    peer_result["origin"] = json!({"kind":"peer","from":"other-session","msg_id":"m1"});
    event(&mut driver, &mut peer, peer_result).await.unwrap();
    event(&mut driver, &mut peer, state("idle")).await.unwrap();
    assert_eq!(driver.phase(), Phase::ResultObserved);
    assert!(driver
        .completion_receipt()
        .unwrap()
        .matches_text("Answer to the peer"));
    assert!(driver.can_followup());
}

/// A message from another session on this computer wakes a parked session
/// the same way a timer does, as the CLI wrote it (2.1.323, offline capture
/// 2026-11-17): no frame names a user message, the lifecycle frame names no
/// wake-up source, and the result names the peer.
#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_takes_over_a_turn_a_peer_message_started() {
    let (mut driver, mut peer) = parked().await;
    write(
        &mut peer,
        [state("running"), lifecycle("peer-one", "started"), init()],
    )
    .await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(true));
    assert!(driver.self_started_turn());
    driver.next_message().await.unwrap();
    driver.supervise_turn().unwrap();
    event(
        &mut driver,
        &mut peer,
        json!({"type":"system","subtype":"status","status":"requesting","session_id":"session-one"}),
    )
    .await
    .unwrap();
    let mut result = text_result("result-peer", "Peer answer");
    result["origin"] = json!({"kind":"peer","from":"uds:/tmp/cc-socks/3165.sock",
        "verifiedPeerPid":3165,"verifiedLocal":true,"msg_id":"7176e5cb-0966-424e-9daf-2f1f8a3ffa2d",
        "name":"work-6a","fromMode":"prompting","body":"Peer says hello"});
    event(&mut driver, &mut peer, result).await.unwrap();
    event(&mut driver, &mut peer, lifecycle("peer-one", "completed"))
        .await
        .unwrap();
    event(&mut driver, &mut peer, state("idle")).await.unwrap();
    assert_eq!(driver.phase(), Phase::ResultObserved);
    assert!(driver.completion_receipt().unwrap().matches_text("Peer answer"));
    assert!(driver.can_followup());
    nothing_written(&mut peer).await;
}

#[tokio::test(start_paused = true)]
async fn native_claude_self_started_turn_refuses_an_origin_it_does_not_know() {
    let (mut driver, mut peer) = parked().await;
    write(&mut peer, timer_start()).await;
    assert_eq!(driver.poll_idle(limits()).await, Ok(true));
    driver.next_message().await.unwrap();
    let mut unknown = text_result("result-unknown", "text");
    unknown["origin"] = json!({"kind":"channel","server":"elsewhere"});
    assert!(matches!(
        event(&mut driver, &mut peer, unknown).await,
        Err(Error::Identity)
    ));
    assert_eq!(driver.phase(), Phase::Closed);
}

/// The change is limited to turns the host took over as self-started: a
/// prompted turn still ends on a peer result and on an init nobody asked for.
#[tokio::test(start_paused = true)]
async fn native_claude_prompted_turn_still_refuses_self_started_cycles() {
    let (mut driver, mut peer) = parked().await;
    driver.begin_followup(limits()).unwrap();
    let second = next_prompt(&mut driver, &mut peer).await;
    let mut peer_result = text_result("result-peer", "Answer to the peer");
    peer_result["user_message_uuid"] = second["uuid"].clone();
    peer_result["origin"] = json!({"kind":"peer","from":"other-session","msg_id":"m1"});
    assert!(matches!(
        event(&mut driver, &mut peer, peer_result).await,
        Err(Error::Identity)
    ));
    assert_eq!(driver.phase(), Phase::Closed);

    let (mut driver, mut peer) = parked().await;
    driver.begin_followup(limits()).unwrap();
    let second = next_prompt(&mut driver, &mut peer).await;
    let mut answer = text_result("result-two", "answer");
    answer["user_message_uuid"] = second["uuid"].clone();
    event(&mut driver, &mut peer, answer).await.unwrap();
    assert!(matches!(
        event(&mut driver, &mut peer, init()).await,
        Err(Error::Identity)
    ));
    assert_eq!(driver.phase(), Phase::Closed);
}

#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_ends_the_session_on_any_other_frame() {
    let mut other_session = state("running");
    other_session["session_id"] = json!("different-session");
    for frame in [
        other_session,
        init(),
        text_result("stray", "text"),
        permission("stray-callback"),
        json!({"type":"assistant","session_id":"session-one","message":{"id":"m","content":[]}}),
        state("requires_action"),
    ] {
        let (mut driver, mut peer) = parked().await;
        write(&mut peer, [frame]).await;
        assert_eq!(driver.poll_idle(limits()).await, Err(Error::Identity));
        assert_eq!(driver.phase(), Phase::Closed);
        assert!(!driver.self_started_turn());
    }
}

#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_needs_a_parked_continuous_session() {
    // A session that is not continuous is never looked at.
    let (mut driver, mut peer) = pair(4096, limits());
    driver.enable_native_completion().unwrap();
    initialize(&mut driver, &mut peer).await;
    prompt(&mut driver, &mut peer).await;
    event(&mut driver, &mut peer, init()).await.unwrap();
    event(&mut driver, &mut peer, result()).await.unwrap();
    event(&mut driver, &mut peer, state("idle")).await.unwrap();
    assert_eq!(driver.poll_idle(limits()).await, Err(Error::State));
    assert_eq!(driver.phase(), Phase::ResultObserved);

    // Nor is one whose turn is still open.
    let (mut driver, mut peer) = pair(4096, limits());
    driver.enable_native_completion().unwrap();
    driver.enable_continuous_session().unwrap();
    initialize(&mut driver, &mut peer).await;
    next_prompt(&mut driver, &mut peer).await;
    assert_eq!(driver.poll_idle(limits()).await, Err(Error::State));
    assert_eq!(driver.phase(), Phase::Running);
}

#[tokio::test(start_paused = true)]
async fn native_claude_idle_look_reports_a_process_that_exited() {
    let (mut driver, peer) = parked().await;
    drop(peer);
    assert_eq!(driver.poll_idle(limits()).await, Err(Error::PeerEof));
    assert_eq!(driver.phase(), Phase::Closed);
}

/// The installed CLI itself. The model sets a one-minute timer and answers;
/// the timer fires while the session is parked, the idle look takes that turn
/// over, and the session then continues with the next prompt.
#[tokio::test]
#[ignore = "requires HAGENCY_TEST_CLAUDE_EXECUTABLE; local Anthropic SSE only, no paid model; waits up to three minutes for the CLI's own timer"]
async fn installed_claude_timer_turn_is_taken_over_by_the_idle_look() {
    use hagency_runtime::claude::session::{
        ApprovalControlPolicy, ObservationKind, PermissionDecision, PreparedUpdate, UsageCoverage,
    };
    let root = tempfile::tempdir().unwrap();
    let fixture = root.path().join("fixture.py");
    std::fs::write(&fixture, include_str!("../claude_timer_fixture.py")).unwrap();
    let mut server = tokio::process::Command::new("python3")
        .arg(&fixture)
        .arg(root.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut address = String::new();
    BufReader::new(server.stdout.take().unwrap())
        .read_line(&mut address)
        .await
        .unwrap();
    let executable =
        std::env::var_os("HAGENCY_TEST_CLAUDE_EXECUTABLE").expect("installed Claude executable");
    let mut child = tokio::process::Command::new(executable)
        .args(hagency_runtime::claude::interactive_arguments("sonnet", "medium").unwrap())
        .env_clear()
        .env("HOME", root.path())
        .env("CLAUDE_CONFIG_DIR", root.path().join("config"))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("ANTHROPIC_API_KEY", "offline-contract")
        .env(
            "ANTHROPIC_BASE_URL",
            format!("http://127.0.0.1:{}", address.trim()),
        )
        .env("CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS", "1")
        .env("CLAUDE_CODE_BG_TASKS_REPORT_RUNNING", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .current_dir(root.path())
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let turn_limits = Limits {
        write_timeout_ms: 2000,
        event_wait_ms: 20_000,
        lifetime_ms: 60_000,
    };
    let policy = ApprovalControlPolicy {
        owner_wait_ms: 20_000,
        response_reserve_ms: 2000,
    };
    let stages = || std::fs::read_to_string(root.path().join("stages.jsonl")).unwrap_or_default();
    let mut driver = SessionDriver::new(
        child.stdout.take().unwrap(),
        child.stdin.take().unwrap(),
        child.stderr.take().unwrap(),
        turn_limits,
    )
    .unwrap();
    driver.enable_native_completion().unwrap();
    driver.enable_continuous_session().unwrap();
    driver.initialize().await.unwrap();

    // The requested turn: the model sets its timer, then answers.
    driver.prompt("Start the timer check").await.unwrap();
    driver.next_message().await.unwrap();
    driver.enable_approval_control(policy).unwrap();
    let mut scheduled = false;
    for _ in 0..512 {
        let message = driver.next_message().await.unwrap();
        if let Message::Permission {
            request_id,
            tool_name,
            ..
        } = message
        {
            assert_eq!(tool_name, "ScheduleWakeup");
            scheduled = true;
            let mut prepared = driver
                .prepare_approval(&request_id, PermissionDecision::Allow)
                .unwrap();
            while !matches!(
                driver.send_prepared_approval(&mut prepared).await.unwrap(),
                PreparedUpdate::WriteAccepted(_)
            ) {}
        }
        if driver.phase() == Phase::ResultObserved {
            break;
        }
    }
    assert!(scheduled, "model stages: {}", stages());
    assert!(driver
        .completion_receipt()
        .unwrap()
        .matches_text("Foreground answer"));
    assert!(driver.can_followup());

    // Parked. The runtime looks once a second; so does this test.
    let mut looks = 0;
    while !driver.poll_idle(turn_limits).await.unwrap() {
        looks += 1;
        assert!(looks < 180, "the timer did not fire; model stages: {}", stages());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(driver.self_started_turn());
    let first = driver.next_message().await.unwrap();
    assert!(matches!(
        first,
        Message::Event { kind: EventKind::System, ref payload, .. } if payload["subtype"] == "init"
    ));
    driver.supervise_turn().unwrap();
    driver.enable_approval_control(policy).unwrap();
    let mut accounted = false;
    for _ in 0..512 {
        let message = driver.next_message().await.unwrap();
        assert!(
            !matches!(message, Message::Permission { .. }),
            "the report turn asks for no tool"
        );
        if let Some(ObservationKind::Result { usage, is_error }) =
            driver.last_observation().map(|observation| observation.kind().clone())
        {
            assert!(!is_error);
            accounted = usage.coverage() == UsageCoverage::ReportedModelsResult
                && usage.counts().output() == Some(9);
        }
        if driver.phase() == Phase::ResultObserved {
            break;
        }
    }
    assert!(
        driver.completion_receipt().unwrap().matches_text("Timer report"),
        "model stages: {}",
        stages()
    );
    assert!(accounted, "the self-started turn reports its own usage");
    assert!(driver.can_followup(), "parked again after its own turn");

    // The same session answers the next request.
    driver.prepare_followup(turn_limits).await.unwrap();
    driver.prompt("One more question").await.unwrap();
    driver.next_message().await.unwrap();
    for _ in 0..512 {
        driver.next_message().await.unwrap();
        if driver.phase() == Phase::ResultObserved {
            break;
        }
    }
    assert!(driver.completion_receipt().unwrap().matches_text("Next answer"));
    let stages = stages();
    for expected in ["\"start\"", "\"foreground\"", "\"later\", \"n\": 1", "\"later\", \"n\": 2"] {
        assert!(stages.contains(expected), "model stages: {stages}");
    }
    driver.close();
    child.kill().await.unwrap();
    server.kill().await.unwrap();
}
