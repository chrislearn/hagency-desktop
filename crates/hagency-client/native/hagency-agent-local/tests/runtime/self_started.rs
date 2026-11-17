//! Turns a retained Claude process starts by itself while it is parked: a
//! timer the session set, a message from another session on this computer.
use super::*;
use hagency_runtime::live_text::{LiveText, MessageError, MessageSink};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const OWNER: &str = "@owner:test";

#[derive(Default)]
struct Messages(Mutex<Vec<(String, String)>>);
impl MessageSink for Messages {
    fn record(&self, id: &str, text: &str) -> Result<(), MessageError> {
        self.0.lock().unwrap().push((id.into(), text.into()));
        Ok(())
    }
}

/// One requested turn has been answered and settled: the state a process is
/// retained in.
async fn parked(mode: &str) -> (Fixture, Process, codex::ApprovalQueue, Arc<Messages>) {
    let mut f = Fixture::new(Kind::Claude, mode, ToolPolicy::Deny);
    let mut process = f.process().await;
    process
        .session
        .open_context(&mut f.ledger, &f.scope, &f.profile)
        .await
        .unwrap();
    let (queue, _) = codex::approval_queue(1).unwrap();
    let messages = Arc::new(Messages::default());
    let queue = queue.with_live_text(LiveText::with_messages(messages.clone()));
    let first = requested(&mut f, &mut process, "first", &queue).await;
    assert_eq!(first.text, "offline reply");
    assert_eq!(f.spent(), (54, 0));
    assert!(process.session.can_continue());
    (f, process, queue, messages)
}
async fn requested(
    f: &mut Fixture,
    process: &mut Process,
    call: &str,
    queue: &codex::ApprovalQueue,
) -> codex::Completed {
    tokio::time::timeout(
        Duration::from_secs(10),
        process.session.run_with_started(
            &mut f.ledger,
            OWNER,
            &f.scope,
            call,
            call,
            "next message",
            BudgetMode::Postpaid,
            queue,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap()
}
/// The host's idle look, repeated as the run loop repeats it.
async fn look_until_started(process: &mut Process) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !process.session.poll_self_started().await.unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the CLI started its own turn");
}
async fn self_started(
    f: &mut Fixture,
    process: &mut Process,
    turn: &str,
    queue: &codex::ApprovalQueue,
) -> codex::Result<codex::Completed> {
    tokio::time::timeout(
        Duration::from_secs(10),
        process.session.run_self_started(
            &mut f.ledger,
            OWNER,
            &f.scope,
            turn,
            turn,
            BudgetMode::Postpaid,
            queue,
            None,
        ),
    )
    .await
    .unwrap()
}
fn prompts(f: &Fixture) -> usize {
    fs::read_to_string(f.path("frames"))
        .unwrap()
        .lines()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["type"] == "user"
        })
        .count()
}

#[tokio::test]
async fn claude_parked_session_runs_and_settles_the_turn_it_started_by_itself() {
    for mode in ["continuous:self_timer", "continuous:self_peer"] {
        let (mut f, mut process, queue, messages) = parked(mode).await;
        // Nothing happened yet: the look neither waits nor writes, and the
        // session stays as it was.
        assert_eq!(process.session.poll_self_started().await.unwrap(), false);
        assert!(process.session.can_continue());
        assert!(matches!(
            self_started(&mut f, &mut process, "self-early", &queue).await,
            Err(Error::Protocol("no self-started turn is open"))
        ));
        assert!(process.session.can_continue());
        assert_eq!(f.spent(), (54, 0));

        fs::write(f.path("fire"), "").unwrap();
        look_until_started(&mut process).await;
        // The turn is open and nobody has admitted it yet: the session is
        // neither reusable for a request nor looked at again.
        assert!(!process.session.can_continue());
        assert!(process.session.poll_self_started().await.is_err());
        assert!(
            process
                .session
                .continue_context(&mut f.ledger, &f.scope, &f.profile)
                .await
                .is_err()
        );

        let done = self_started(&mut f, &mut process, "self-one", &queue)
            .await
            .unwrap();
        assert_eq!(done.text, "self-started report", "{mode}");
        assert!(done.late_failure.is_none());
        assert!(done.usage.is_reported());
        assert_eq!(done.usage.input + done.usage.output, 54);
        // Charged and settled once, like a requested turn.
        assert_eq!(f.spent(), (108, 0), "{mode}");
        assert!(f.ledger.outstanding_calls(OWNER).unwrap().is_empty());
        // The text before its tool is a message of its own; the final answer
        // is returned, not recorded.
        let records = messages.0.lock().unwrap().clone();
        assert_eq!(
            records.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(),
            ["self-started progress"],
            "{mode}"
        );
        // The CLI was given no prompt for it.
        assert_eq!(prompts(&f), 1, "{mode}");

        // Parked again; the next request continues the same conversation.
        assert!(process.session.can_continue());
        process
            .session
            .continue_context(&mut f.ledger, &f.scope, &f.profile)
            .await
            .unwrap();
        let third = requested(&mut f, &mut process, "third", &queue).await;
        assert_eq!(third.text, "offline reply");
        assert_eq!(f.spent(), (162, 0), "{mode}");
        assert_eq!(prompts(&f), 2);
        process.stop().await.unwrap();
    }
}

fn rule(requests: RequestPolicy, limit: Limit) -> Policy {
    Policy {
        budget: Budget {
            limit,
            period: Period::Lifetime,
        },
        requests,
        high_risk: ToolPolicy::Deny,
    }
}

/// A scope that denies requests, or whose budget is spent, refuses a
/// self-started turn as it refuses a request. A refused turn is charged
/// nothing and leaves a process that cannot continue: the host stops it.
#[tokio::test]
async fn claude_self_started_turn_is_refused_by_a_denying_scope_and_a_spent_budget() {
    for refusal in ["deny", "budget"] {
        let (mut f, mut process, queue, messages) = parked("continuous:self_timer").await;
        let policy = match refusal {
            "deny" => rule(RequestPolicy::Deny, Limit::Unlimited),
            _ => rule(RequestPolicy::Allow, Limit::Tokens(54)),
        };
        f.ledger
            .set_policy(OWNER, &f.scope, Layer::Requester, 1, &policy)
            .unwrap();
        fs::write(f.path("fire"), "").unwrap();
        look_until_started(&mut process).await;
        let refused = self_started(&mut f, &mut process, "self-one", &queue).await;
        match refusal {
            "budget" => assert!(
                matches!(refused, Err(Error::Ledger(hagency_agent_local::Error::Budget))),
                "{refusal}"
            ),
            _ => assert!(
                matches!(refused, Err(Error::Ledger(hagency_agent_local::Error::Denied))),
                "{refusal}"
            ),
        }
        assert_eq!(f.spent(), (54, 0), "{refusal}");
        assert!(f.ledger.outstanding_calls(OWNER).unwrap().is_empty());
        assert!(messages.0.lock().unwrap().is_empty());
        assert!(!process.session.can_continue());
        process.stop().await.unwrap();
    }
}

/// A self-started turn continues a conversation that was already admitted.
/// Where the scope's requests need the owner's confirmation, nobody is asked
/// for it: it runs and settles. The next request is still asked for.
#[tokio::test]
async fn claude_self_started_turn_runs_unasked_where_requests_ask_the_owner() {
    let (mut f, mut process, queue, _) = parked("continuous:self_timer").await;
    f.ledger
        .set_policy(
            OWNER,
            &f.scope,
            Layer::Requester,
            1,
            &rule(RequestPolicy::AskOwner, Limit::Unlimited),
        )
        .unwrap();
    fs::write(f.path("fire"), "").unwrap();
    look_until_started(&mut process).await;
    let done = self_started(&mut f, &mut process, "self-one", &queue)
        .await
        .unwrap();
    assert_eq!(done.text, "self-started report");
    assert_eq!(f.spent(), (108, 0));
    assert!(f.ledger.outstanding_calls(OWNER).unwrap().is_empty());
    assert!(process.session.can_continue());

    // The confirmation covered that turn only.
    process
        .session
        .continue_context(&mut f.ledger, &f.scope, &f.profile)
        .await
        .unwrap();
    let asked = tokio::time::timeout(
        Duration::from_secs(10),
        process.session.run_with_started(
            &mut f.ledger,
            OWNER,
            &f.scope,
            "third",
            "third",
            "next message",
            BudgetMode::Postpaid,
            &queue,
            None,
        ),
    )
    .await
    .unwrap();
    assert!(matches!(
        asked,
        Err(Error::Ledger(hagency_agent_local::Error::Denied))
    ));
    assert_eq!(f.spent(), (108, 0));
    process.stop().await.unwrap();
}

/// Setting, listing and removing the session's own timers are not tools the
/// owner's rules decide: under a rule that asks the owner, and under one that
/// denies, nobody is asked and the CLI is told to go on.
#[tokio::test]
async fn claude_timer_tools_are_allowed_unasked_under_every_tool_rule() {
    for tool in ["ScheduleWakeup", "CronCreate", "CronList", "CronDelete"] {
        for tools in [ToolPolicy::AskOwner, ToolPolicy::Deny] {
            let mut f = Fixture::new(Kind::Claude, &format!("approval:{tool}"), tools);
            let mut process = f.process().await;
            process
                .session
                .open_context(&mut f.ledger, &f.scope, &f.profile)
                .await
                .unwrap();
            let (queue, mut owner) = codex::approval_queue(1).unwrap();
            let done = requested(&mut f, &mut process, "call", &queue).await;
            assert_eq!(done.tool_requests, 1, "{tool}");
            assert!(owner.try_recv().is_err(), "{tool}");
            assert!(f.path("effect").exists(), "{tool}");
            process.stop().await.unwrap();
        }
    }
}

/// Full trust is the owner's standing answer for their own conversation. A
/// timer the session set continues that conversation: its tool runs unasked.
/// A peer session's message is text from outside the room: the same tool is
/// put to the owner and runs only on their yes.
#[tokio::test]
async fn claude_peer_started_turn_asks_the_owner_where_full_trust_would_allow() {
    for (mode, asked) in [
        ("continuous:self_timer_tool", false),
        ("continuous:self_peer_tool", true),
    ] {
        let mut f = Fixture::called_by(Kind::Claude, mode, ToolPolicy::full_trust(), OWNER);
        let mut process = f.process().await;
        process
            .session
            .open_context(&mut f.ledger, &f.scope, &f.profile)
            .await
            .unwrap();
        let (queue, mut owner) = codex::approval_queue(1).unwrap();
        let queue = queue.with_live_text(LiveText::with_messages(Arc::new(Messages::default())));
        requested(&mut f, &mut process, "first", &queue).await;
        fs::write(f.path("fire"), "").unwrap();
        look_until_started(&mut process).await;
        let effect = f.path("self-effect");
        let answer = async {
            if asked {
                let pending = tokio::time::timeout(Duration::from_secs(10), owner.recv())
                    .await
                    .expect("the owner is asked")
                    .unwrap();
                assert_eq!(pending.proposal.tool, "claude.Bash");
                assert!(!effect.exists());
                pending.decide(true);
            }
        };
        let (done, ()) = tokio::join!(self_started(&mut f, &mut process, "self-one", &queue), answer);
        done.unwrap();
        assert!(effect.exists(), "{mode}");
        assert!(owner.try_recv().is_err(), "{mode}");
        process.stop().await.unwrap();
    }
}

/// A local Agent's CLI is started to take messages of the owner's other
/// sessions on this computer, and told to use a timer instead of waiting.
#[tokio::test]
async fn claude_local_launch_accepts_peer_messages_and_names_the_timer() {
    let (f, mut process, _, _) = parked("continuous:self_timer").await;
    process.stop().await.unwrap();
    let launch: Vec<String> = serde_json::from_str(
        fs::read_to_string(f.path("argv")).unwrap().lines().next().unwrap(),
    )
    .unwrap();
    let after = |flag: &str| &launch[launch.iter().position(|arg| arg == flag).unwrap() + 1];
    let settings: serde_json::Value = serde_json::from_str(after("--settings")).unwrap();
    assert_eq!(settings["crossSessionInbound"], "accept");
    assert_eq!(settings["permissions"]["ask"], serde_json::json!(["*"]));
    assert!(after("--append-system-prompt").contains("ScheduleWakeup"));
}

/// A turn started by something this host does not know is not taken over as
/// an answer: it fails, its call stays unsettled for the owner to see, and
/// the process cannot continue.
#[tokio::test]
async fn claude_self_started_turn_of_an_unknown_origin_fails() {
    let (mut f, mut process, queue, _) = parked("continuous:self_unknown").await;
    fs::write(f.path("fire"), "").unwrap();
    look_until_started(&mut process).await;
    assert!(self_started(&mut f, &mut process, "self-one", &queue).await.is_err());
    assert!(!process.session.can_continue());
    assert_eq!(f.ledger.outstanding_calls(OWNER).unwrap().len(), 1);
    process.stop().await.unwrap();
}

/// The installed CLI. The model sets a one-minute timer, unasked although the
/// tool rule asks the owner, and answers; the timer fires while the session
/// is parked; the turn it starts is admitted,
/// charged and settled under an identity of its own, and the session then
/// answers the next request.
#[tokio::test]
#[ignore = "requires explicit installed Claude; localhost model, no provider account or paid request; waits up to three minutes for the CLI's own timer"]
async fn installed_claude_timer_turn_is_admitted_and_settled_as_its_own_turn() {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let executable = PathBuf::from(
        std::env::var_os("HAGENCY_TEST_CLAUDE_EXECUTABLE").expect("explicit Claude executable"),
    )
    .canonicalize()
    .unwrap();
    let mut f = Fixture::new(Kind::Claude, "questions", ToolPolicy::AskOwner);
    f.profile.model = "sonnet".into();
    let model_file = f.path("model.py");
    fs::write(
        &model_file,
        include_str!("../../../hagency-runtime/tests/claude_timer_fixture.py"),
    )
    .unwrap();
    let mut model = tokio::process::Command::new("python3")
        .arg(model_file)
        .arg(f.path(""))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut port = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(model.stdout.take().unwrap()).read_line(&mut port),
    )
    .await
    .unwrap()
    .unwrap();
    let port: u16 = port.trim().parse().unwrap();
    let binary = serde_json::to_string(&executable).unwrap();
    fs::write(&f.profile.executable, format!("#!/usr/bin/env python3\nimport os,sys\nos.environ['ANTHROPIC_API_KEY']='offline-contract'\nos.environ['ANTHROPIC_BASE_URL']='http://127.0.0.1:{port}'\nos.environ['CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC']='1'\nos.execv({binary},[{binary},*sys.argv[1:]])\n")).unwrap();
    let mut process = f.process().await;
    process
        .session
        .open_context(&mut f.ledger, &f.scope, &f.profile)
        .await
        .unwrap();
    let messages = Arc::new(Messages::default());
    let (queue, mut owner) = codex::approval_queue(1).unwrap();
    let queue = queue.with_live_text(LiveText::with_messages(messages.clone()));
    let stages = |f: &Fixture| fs::read_to_string(f.path("stages.jsonl")).unwrap_or_default();

    let run = process.session.run_with_started(
        &mut f.ledger,
        OWNER,
        &f.scope,
        "call",
        "dispatch",
        "Start the timer check",
        BudgetMode::Postpaid,
        &queue,
        None,
    );
    let done = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .unwrap();
    // The tool rule asks the owner, and nobody was asked for the timer.
    assert!(owner.try_recv().is_err());
    let done = done.unwrap_or_else(|error| panic!("turn failed: {error:?}; model stages: {}", stages(&f)));
    assert_eq!(done.text, "Foreground answer", "model stages: {}", stages(&f));
    assert!(done.usage.is_reported());
    assert!(process.session.can_continue());
    let (after_request, _) = f.spent();

    // Parked. The host looks once a second; so does this test.
    let mut looks = 0;
    while !process.session.poll_self_started().await.unwrap() {
        looks += 1;
        assert!(looks < 180, "the timer did not fire; model stages: {}", stages(&f));
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(!process.session.can_continue());
    let report = tokio::time::timeout(
        Duration::from_secs(90),
        process.session.run_self_started(
            &mut f.ledger,
            OWNER,
            &f.scope,
            "self-started-one",
            "self-started-one",
            BudgetMode::Postpaid,
            &queue,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap_or_else(|error| panic!("self-started turn failed: {error:?}; model stages: {}", stages(&f)));
    assert_eq!(report.text, "Timer report", "model stages: {}", stages(&f));
    assert!(report.late_failure.is_none());
    assert!(report.usage.is_reported());
    // The fixture's model reports 17 input and 9 output tokens per request.
    let cost = report.usage.input + report.usage.output;
    assert!(cost >= 26, "{cost}");
    assert_eq!(f.spent(), (after_request + cost, 0));
    assert!(f.ledger.outstanding_calls(OWNER).unwrap().is_empty());

    // The same session answers the next request.
    assert!(process.session.can_continue());
    process
        .session
        .continue_context(&mut f.ledger, &f.scope, &f.profile)
        .await
        .unwrap();
    let next = tokio::time::timeout(
        Duration::from_secs(90),
        process.session.run_with_started(
            &mut f.ledger,
            OWNER,
            &f.scope,
            "call-two",
            "dispatch-two",
            "One more question",
            BudgetMode::Postpaid,
            &queue,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next.text, "Next answer", "model stages: {}", stages(&f));
    process.stop().await.unwrap();
    model.kill().await.unwrap();
}
