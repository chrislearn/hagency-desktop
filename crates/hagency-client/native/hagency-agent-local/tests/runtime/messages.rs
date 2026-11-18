use super::*;
use hagency_runtime::live_text::{LiveText, MessageError, MessageSink};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Messages(Mutex<Vec<(String, String)>>);
impl MessageSink for Messages {
    fn record(&self, id: &str, text: &str) -> Result<(), MessageError> {
        self.0.lock().unwrap().push((id.into(), text.into()));
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires explicit installed Claude; localhost model, no provider account or paid request"]
async fn installed_claude_delivers_progress_before_waiting_for_a_question() {
    use hagency_runtime::questions::Answer;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let executable = PathBuf::from(std::env::var_os("HAGENCY_TEST_CLAUDE_EXECUTABLE")
        .expect("explicit Claude executable")).canonicalize().unwrap();
    let mut f = Fixture::new(Kind::Claude, "questions", ToolPolicy::Deny);
    f.profile.model = "sonnet".into();
    let model_file = f.path("model.py");
    fs::write(&model_file, include_str!("../../../hagency-runtime/tests/claude_question_fixture.py")).unwrap();
    let mut model = tokio::process::Command::new("python3")
        .arg(model_file).arg(f.path("")).arg("progress")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .kill_on_drop(true).spawn().unwrap();
    let mut port = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5),
        BufReader::new(model.stdout.take().unwrap()).read_line(&mut port)).await.unwrap().unwrap();
    let port: u16 = port.trim().parse().unwrap();
    let binary = serde_json::to_string(&executable).unwrap();
    fs::write(&f.profile.executable, format!("#!/usr/bin/env python3\nimport os,sys\nos.environ['ANTHROPIC_API_KEY']='offline-contract'\nos.environ['ANTHROPIC_BASE_URL']='http://127.0.0.1:{port}'\nos.environ['CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC']='1'\nos.execv({binary}, [{binary}]+sys.argv[1:])\n")).unwrap();
    let guardian = std::path::Path::new(env!("CARGO_BIN_EXE_hagency-local-probe"));
    let mut process = spawn_for_scope_with_guardian(
        &f.profile, &f.reference, &mut f.ledger, &f.scope, Some(guardian),
    ).await.unwrap();
    process.session.initialize().await.unwrap();
    process.session.open_context(&mut f.ledger, &f.scope, &f.profile).await.unwrap();
    let messages = Arc::new(Messages::default());
    let (queue, _) = codex::approval_queue(1).unwrap();
    let (questions, mut incoming) = hagency_agent_local::questions::question_queue();
    let queue = queue.with_questions(questions).with_live_text(LiveText::with_messages(messages.clone()));
    let run = process.session.run_with_started(&mut f.ledger, "@owner:test", &f.scope,
        "call", "dispatch", "Ask questions", BudgetMode::Postpaid, &queue, None);
    let host = async {
        let pending = incoming.recv().await.unwrap();
        assert!(messages.0.lock().unwrap().iter().any(|(_, text)| text == "First finding"));
        assert!(pending.reply([
            ("q0".into(), Answer::Options(vec!["One".into(), "Two".into()])),
            ("q1".into(), Answer::Option("One".into())),
            ("q2".into(), Answer::Option("One".into())),
            ("q3".into(), Answer::Option("Two".into())),
        ].into()).accepted().await);
    };
    let (done, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(run, host)
    }).await.unwrap();
    assert_eq!(done.unwrap().text, "Offline native question completed.");
    process.stop().await.unwrap();
    model.kill().await.unwrap();
}

#[tokio::test]
async fn claude_background_followup_preserves_the_original_answer_as_a_separate_message() {
    let mut f = Fixture::new(Kind::Claude, "background_native_followup", ToolPolicy::Deny);
    let mut process = f.process().await;
    process
        .session
        .open_context(&mut f.ledger, &f.scope, &f.profile)
        .await
        .unwrap();
    let (queue, _) = codex::approval_queue(1).unwrap();
    let messages = Arc::new(Messages::default());
    let queue = queue.with_live_text(LiveText::with_messages(messages.clone()));
    let done = process
        .session
        .run_with_started(
            &mut f.ledger,
            "@owner:test",
            &f.scope,
            "call",
            "call",
            "start background work",
            BudgetMode::Postpaid,
            &queue,
            None,
        )
        .await
        .unwrap();
    assert_eq!(done.text, "native background final answer");
    let records = messages.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1, "offline reply");
    assert!(records[0].0.starts_with("claude:result:"));
    drop(records);
    process.stop().await.unwrap();
}

#[tokio::test]
async fn claude_distinct_background_results_with_equal_text_remain_distinct() {
    let mut f = Fixture::new(Kind::Claude, "background_native_equal", ToolPolicy::Deny);
    let mut process = f.process().await;
    process.session.open_context(&mut f.ledger, &f.scope, &f.profile).await.unwrap();
    let (queue, _) = codex::approval_queue(1).unwrap();
    let messages = Arc::new(Messages::default());
    let queue = queue.with_live_text(LiveText::with_messages(messages.clone()));
    let done = process.session.run_with_started(&mut f.ledger, "@owner:test", &f.scope,
        "call", "call", "start background work", BudgetMode::Postpaid, &queue, None).await.unwrap();
    assert_eq!(done.text, "offline reply");
    let records = messages.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1, "offline reply");
    assert!(records[0].0.starts_with("claude:result:"));
    drop(records);
    process.stop().await.unwrap();
}

#[tokio::test]
async fn claude_reused_result_identity_with_changed_text_fails_closed() {
    let mut f = Fixture::new(Kind::Claude, "background_native_reused_id", ToolPolicy::Deny);
    let mut process = f.process().await;
    process.session.open_context(&mut f.ledger, &f.scope, &f.profile).await.unwrap();
    let (queue, _) = codex::approval_queue(1).unwrap();
    let messages = Arc::new(Messages::default());
    let queue = queue.with_live_text(LiveText::with_messages(messages.clone()));
    let result = process.session.run_with_started(&mut f.ledger, "@owner:test", &f.scope,
        "call", "call", "start background work", BudgetMode::Postpaid, &queue, None).await;
    assert!(result.is_err());
    assert!(messages.0.lock().unwrap().is_empty());
    let _ = process.stop().await;
}

#[tokio::test]
async fn octos_can_report_twice_during_one_turn_and_keep_its_final_answer() {
    let mut f = Fixture::new(Kind::Octos, "octos_progress", ToolPolicy::Deny);
    let mut process = f.process().await;
    process
        .session
        .open_context(&mut f.ledger, &f.scope, &f.profile)
        .await
        .unwrap();
    let (queue, _) = codex::approval_queue(1).unwrap();
    let messages = Arc::new(Messages::default());
    let queue = queue.with_live_text(LiveText::with_messages(messages.clone()));
    let done = process
        .session
        .run_with_started(
            &mut f.ledger,
            "@owner:test",
            &f.scope,
            "call",
            "dispatch",
            "work on the task",
            BudgetMode::Postpaid,
            &queue,
            None,
        )
        .await
        .unwrap();
    assert_eq!(done.text, "offline reply");
    assert!(
        fs::read_to_string(f.path("turn-input"))
            .unwrap()
            .contains("hagency_report_progress")
    );
    let records = messages.0.lock().unwrap();
    assert_eq!(
        &*records,
        &[
            ("octos:tool:progress-1".into(), "first finding".into()),
            ("octos:tool:progress-2".into(), "second finding".into())
        ]
    );
    drop(records);
    process.stop().await.unwrap();
}

#[tokio::test]
async fn claude_delivers_each_intermediate_message_before_its_tool_finishes() {
    check_claude_intermediate_messages("spontaneous_messages").await;
}

#[tokio::test]
async fn claude_split_text_and_tool_frames_deliver_before_the_tool_finishes() {
    check_claude_intermediate_messages("spontaneous_split").await;
}

async fn check_claude_intermediate_messages(mode: &str) {
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
    let marker = f.path("progress-observed");
    let run = process.session.run_with_started(
        &mut f.ledger,
        "@owner:test",
        &f.scope,
        "call",
        "dispatch",
        "work on the task",
        BudgetMode::Postpaid,
        &queue,
        None,
    );
    let witness = async {
        loop {
            if messages
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|(_, text)| text == "First finding")
            {
                fs::write(&marker, "observed").unwrap();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    let (done, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(run, witness)
    })
    .await
    .unwrap();
    assert_eq!(done.unwrap().text, "offline reply");
    let rows = messages.0.lock().unwrap();
    assert_eq!(
        &*rows,
        &[
            ("claude:assistant:progress-a".into(), "First finding".into()),
            (
                "claude:assistant:progress-b".into(),
                "Second finding".into()
            )
        ]
    );
    drop(rows);
    process.stop().await.unwrap();
}
