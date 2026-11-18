//! Timers a local Agent sets for its conversation (ADR 0013).
//!
//! The Agent sets one with the host tool `schedule_wakeup`. The host keeps it
//! in the sealed folder of the Agent's device and, when it is due, starts a
//! turn of the same conversation by itself: the model is told that its timer
//! fired, and what it answers is reported to the room exactly like a turn a
//! native CLI starts by itself (`idle_turns`).
//!
//! Nothing here depends on the native process that set the timer. A restart
//! of the host and the eviction of an idle session leave it in place, and
//! every runtime that has the room tools can set one.
use super::messages::timers::{self, Timer};
use super::provider_custody::{Anchor, Provider, SessionKey, Sessions};
use super::*;
use hagency_runtime::room_tools::{RoomTimerOperation, RoomTools};
use serde_json::json;

const MAX_PER_CHAT: usize = 8;
/// Turns started for one due time before that time is given up. A turn cut
/// off by a restart of the host is started once more.
const MAX_ATTEMPTS: u8 = 2;
/// A repeating timer ends by itself: nobody has to remember to remove it.
const REPEAT_MAX_RUNS: u32 = 200;
const REPEAT_MAX_SECS: i64 = 7 * 24 * 3600;
/// A timer nobody could fire for this long is removed.
const DEAD_SECS: i64 = 30 * 24 * 3600;
const LOOK_EVERY: Duration = Duration::from_secs(2);
const CLOSED_PAUSE: Duration = Duration::from_secs(30);

/// When the binding's loop looks at the timers next.
pub(super) struct Clock(tokio::time::Instant);
impl Clock {
    pub(super) fn new() -> Self {
        Self(tokio::time::Instant::now())
    }
}

fn same_chat(timer: &Timer, dispatch: &inbox::Dispatch) -> bool {
    timer.room == dispatch.room_id && timer.thread == dispatch.thread_root
}
fn shown(timer: &Timer, now: i64) -> serde_json::Value {
    let mut note = timer.note.as_str();
    if note.len() > 400 {
        let mut end = 400;
        while !note.is_char_boundary(end) {
            end -= 1;
        }
        note = &note[..end];
    }
    json!({
        "wakeup_id": timer.id,
        "in_seconds": (timer.due - now).max(0),
        "due_unix_seconds": timer.due,
        "repeat_seconds": timer.repeat,
        "note": note,
    })
}

/// One call of a timer tool, for the conversation of the run `dispatch` and
/// `execution` name. The reply is the tool's JSON result.
pub(super) fn tool(
    root: &std::path::Path,
    dispatch: &inbox::Dispatch,
    execution: &str,
    operation: &RoomTimerOperation,
) -> String {
    let now = now();
    let error = |code: &str| json!({ "error": code }).to_string();
    match operation {
        RoomTimerOperation::Invalid { tool, reason } => {
            // The reason is one of the host's own sentences, never the model's words.
            tracing::info!(code = "agent_timer_tool_invalid", tool, reason,
                "A timer tool was called with arguments that cannot be used");
            json!({ "error": "invalid_arguments", "hint": reason }).to_string()
        }
        RoomTimerOperation::ScheduleWakeup { delay_seconds, note, repeat_seconds } => {
            let set = timers::list(root).iter().filter(|timer| same_chat(timer, dispatch)).count();
            if set >= MAX_PER_CHAT {
                return json!({"error":"too_many_timers","limit":MAX_PER_CHAT,
                    "hint":"list_wakeups shows them; cancel_wakeup removes one"})
                .to_string();
            }
            let mut random = [0u8; 8];
            if getrandom::fill(&mut random).is_err() {
                return error("timer_store_unavailable");
            }
            let Ok(delay) = i64::try_from(*delay_seconds) else {
                return error("invalid_delay");
            };
            let timer = Timer {
                version: 1,
                id: format!("wk_{:016x}", u64::from_be_bytes(random)),
                dispatch: dispatch.id.clone(),
                execution: execution.into(),
                room: dispatch.room_id.clone(),
                thread: dispatch.thread_root.clone(),
                // Read while the run's record still has its body (see `Anchor`).
                encrypted: dispatch.body.is_empty(),
                due: now.saturating_add(delay),
                created: now,
                note: note.clone(),
                repeat: *repeat_seconds,
                attempts: 0,
                fired: 0,
            };
            if timers::save(root, &timer).is_err() {
                return error("timer_store_unavailable");
            }
            tracing::info!(code = "agent_timer_set", delay_seconds, repeats = repeat_seconds.is_some(),
                "A local Agent set a timer for its conversation");
            json!({
                "wakeup_id": timer.id,
                "in_seconds": delay_seconds,
                "due_unix_seconds": timer.due,
                "repeat_seconds": repeat_seconds,
                "status": "set",
            })
            .to_string()
        }
        RoomTimerOperation::ListWakeups {} => {
            let mut set: Vec<Timer> = timers::list(root)
                .into_iter()
                .filter(|timer| same_chat(timer, dispatch))
                .collect();
            set.sort_by_key(|timer| timer.due);
            json!({ "wakeups": set.iter().map(|timer| shown(timer, now)).collect::<Vec<_>>() })
                .to_string()
        }
        RoomTimerOperation::CancelWakeup { wakeup_id } => {
            if !timers::list(root)
                .iter()
                .any(|timer| timer.id == *wakeup_id && same_chat(timer, dispatch))
            {
                return error("no_such_timer");
            }
            timers::remove(root, wakeup_id);
            json!({ "wakeup_id": wakeup_id, "status": "cancelled" }).to_string()
        }
    }
}

fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 120 => format!("{s} seconds ago"),
        s if s < 7200 => format!("{} minutes ago", s / 60),
        s if s < 172_800 => format!("{} hours ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}
/// What the host tells the model when its timer is due.
fn prompt(timer: &Timer, now: i64) -> String {
    let mut text = format!(
        "[Hagency timer] A timer you set in this chat {} is due now. Nobody sent a new message: \
         this turn was started by the timer. Your note to yourself:\n\n{}\n\nDo what the note says \
         now. What you answer is posted to this chat as your own message, so answer with the report \
         or reminder itself.",
        ago(now - timer.created),
        timer.note,
    );
    if let Some(every) = timer.repeat {
        text.push_str(&format!(
            " This timer repeats every {every} seconds; remove it with cancel_wakeup (id {}) once \
             it is no longer needed.",
            timer.id
        ));
    }
    // The room shows nothing of a turn that was cut off, and the model
    // cannot know of it either unless it is told.
    if timer.attempts > 1 {
        text.push_str(
            " An earlier turn for this timer was cut off before it ended, because the host \
             restarted; nothing it may have answered reached the chat. Check what it already did \
             before you repeat a command or a change.",
        );
    }
    text
}

/// After a turn for the timer ended, or its due time was given up: a
/// repeating timer waits for its next time, any other is removed.
fn advance(root: &std::path::Path, mut timer: Timer, now: i64) {
    timer.fired = timer.fired.saturating_add(1);
    match timer.repeat.and_then(|every| i64::try_from(every).ok()) {
        Some(every)
            if timer.fired < REPEAT_MAX_RUNS && now.saturating_sub(timer.created) < REPEAT_MAX_SECS =>
        {
            // From now, not from the time it was due: a host that was off for
            // a day does not then run the missed turns one after another.
            timer.due = now.saturating_add(every);
            timer.attempts = 0;
            if timers::save(root, &timer).is_err() {
                timers::remove(root, &timer.id);
            }
        }
        _ => timers::remove(root, &timer.id),
    }
}

/// The turn of timer `id` ended. The timer is read again first: the Agent
/// may have removed it in that very turn, and then it stays removed.
fn settle(root: &std::path::Path, id: &str, now: i64) {
    if let Some(timer) = timers::get(root, id) {
        advance(root, timer, now);
    }
}

enum Failure {
    Stopped,
    /// No process could be made ready for this turn.
    Unavailable(&'static str),
}

/// The process the timer's turn runs in: the conversation's idle one if it
/// is still retained, else a new one that resumes the conversation from
/// disk, opened exactly as a requested turn opens it.
async fn session<T: Transport>(
    transport: &T,
    lease: &LeaseRef,
    ledger: &mut Ledger,
    owner: &str,
    config: &StartConfig,
    cancel: &mut watch::Receiver<bool>,
    sessions: &mut Sessions,
    anchor: &Anchor,
) -> Result<(SessionKey, Provider), Failure> {
    let scope = anchor.dispatch.scope();
    let host = transport.approval_host().ok_or(Failure::Unavailable("no_message_host"))?;
    if config.shared_workspace
        && config.shared_workspace_pin.as_deref()
            != shared_workspace_pin(&config.profile.cwd).ok().as_deref()
    {
        return Err(Failure::Unavailable("shared_workspace_changed"));
    }
    #[cfg(not(unix))]
    let mut profile = config.profile.clone();
    #[cfg(unix)]
    let space = space_project(transport, owner, config, &scope).await;
    #[cfg(unix)]
    let owner_directory = if config.host_files || config.shared_workspace {
        Some(
            e2ee::owner_directory(transport, lease, config)
                .map_err(|_| Failure::Unavailable("owner_directory"))?,
        )
    } else {
        None
    };
    #[cfg(unix)]
    let (mut profile, workspace) = working_directory::for_turn(
        &config.profile,
        owner_directory.as_deref(),
        owner,
        &scope,
        config.host_files,
        config.shared_workspace,
        config.shared_workspace_pin.as_deref(),
        space.as_ref(),
    )
    .map_err(|_| Failure::Unavailable("working_directory"))?;
    // An update may have replaced the program the conversation started with.
    if !profile.executable.is_file() {
        let _ = current_program(&mut profile, &config.credential_ref);
    }
    let key = SessionKey::new(owner, &anchor.dispatch, &profile, config);
    let cached = match sessions.take(&key) {
        Some(mut provider) => {
            let checked = tokio::select! { biased;
                _ = canceled(cancel) => None,
                result = tokio::time::timeout(Duration::from_secs(10), async {
                    provider.session.verify_host_environment().await?;
                    provider.session.require_local_account().await?;
                    provider.session.continue_context(ledger, &scope, &profile).await
                }) => Some(result.unwrap_or(Err(codex::Error::Timeout))),
            };
            match checked {
                None => {
                    let _ = provider.stop().await;
                    return Err(Failure::Stopped);
                }
                Some(Ok(())) => Some(provider),
                Some(Err(error)) => {
                    let (kind, detail) = error.diagnostic();
                    tracing::warn!(code = "agent_retained_session_replaced", kind, detail,
                        event = error.diagnostic_event(),
                        "A retained native session failed its reuse check before a timer's turn");
                    let _ = provider.stop().await;
                    None
                }
            }
        }
        None => None,
    };
    let retained = cached.is_some();
    let mut provider = match cached {
        Some(provider) => provider,
        None => {
            let tasks = host.console.0.owned_runtime.tasks.clone();
            let spawned = tokio::select! { biased;
                _ = canceled(cancel) => return Err(Failure::Stopped),
                provider = Provider::capture(
                    &tasks,
                    provider_custody::spawn_for_scope(&profile, &config.credential_ref, ledger, &scope),
                ) => provider,
            };
            let mut provider = spawned.map_err(|error| {
                provider_failure("timer_provider_spawn", &error);
                Failure::Unavailable("provider_spawn")
            })?;
            let opened = tokio::select! { biased;
                _ = canceled(cancel) => None,
                result = async {
                    provider.session.initialize().await?;
                    provider.session.verify_host_environment().await?;
                    provider.session.require_local_account().await?;
                    provider
                        .session
                        .enable_room_tools(RoomTools::history_and_files().with_timers())
                        .await?;
                    provider.session.open_context(ledger, &scope, &profile).await
                } => Some(result),
            };
            match opened {
                Some(Ok(())) => provider,
                None => {
                    let _ = provider.stop().await;
                    return Err(Failure::Stopped);
                }
                Some(Err(error)) => {
                    provider_failure("timer_provider_open", &error);
                    let _ = provider.stop().await;
                    return Err(Failure::Unavailable("provider_open"));
                }
            }
        }
    };
    if config.host_files || config.shared_workspace {
        #[cfg(unix)]
        {
            // The gate asks the server whether the run the timer was set in
            // is still running. It is not, so the host's file tools answer
            // "unavailable" in this turn, as the room tools do.
            let gate = Arc::new(LiveToolGate {
                console: host.clone(),
                lease: lease.clone(),
                expected: anchor.dispatch.clone(),
                execution: anchor.execution.clone(),
                cancel: cancel.clone(),
            });
            let bound = match workspace {
                Some(workspace) if retained => provider.session.rebind_host_files(workspace, gate),
                Some(workspace) => provider.session.enable_host_files(workspace, gate),
                None => Err(codex::Error::Profile("workspace")),
            };
            if bound.is_err() {
                let _ = provider.stop().await;
                return Err(Failure::Unavailable("host_files"));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (retained, lease);
            let _ = provider.stop().await;
            return Err(Failure::Unavailable("host_files"));
        }
    }
    Ok((key, provider))
}

/// Start the turn of every timer of this binding that is due. Only a stop of
/// the runtime is an error: a timer that cannot run now is tried again or
/// given up, and requests continue.
#[allow(clippy::too_many_arguments)]
pub(super) async fn pump<T: Transport>(
    transport: &T,
    lease: &LeaseRef,
    ledger: &mut Ledger,
    owner: &str,
    config: &StartConfig,
    cancel: &mut watch::Receiver<bool>,
    status: &watch::Sender<RuntimeStatus>,
    sessions: &mut Sessions,
    pending: &mut Vec<idle_turns::Undelivered>,
    clock: &mut Clock,
) -> Result<(), RuntimeError> {
    let instant = tokio::time::Instant::now();
    if instant < clock.0 {
        return Ok(());
    }
    clock.0 = instant + LOOK_EVERY;
    let Some(host) = transport.approval_host() else {
        return Ok(());
    };
    let Ok(root) = e2ee::message_root(transport, lease, config) else {
        return Ok(());
    };
    if !timers::any(&root) {
        return Ok(());
    }
    let mut due: Vec<Timer> = timers::list(&root)
        .into_iter()
        .filter(|timer| timer.due <= now())
        .collect();
    due.sort_by_key(|timer| timer.due);
    for mut timer in due {
        if *cancel.borrow() {
            return Err(RuntimeError::Stopped);
        }
        let now_secs = now();
        let Ok(record) = ledger.inbox_record(owner, &timer.dispatch) else {
            if now_secs.saturating_sub(timer.due) > DEAD_SECS {
                timers::remove(&root, &timer.id);
            }
            continue;
        };
        // The Agent's bindings share one device folder: each loop takes its own.
        if record.dispatch.binding_id != config.binding_id || record.dispatch.agent_id != lease.agent_id {
            continue;
        }
        if record.execution_id.as_deref() != Some(timer.execution.as_str()) {
            tracing::warn!(code = "agent_timer_dropped", reason = "run_changed",
                "A timer names a run this host no longer has; it is removed");
            timers::remove(&root, &timer.id);
            continue;
        }
        let anchor = Anchor::restored(record.dispatch, timer.execution.clone(), timer.encrypted);
        let scope = anchor.dispatch.scope();
        if timer.attempts >= MAX_ATTEMPTS {
            tracing::warn!(code = "agent_timer_given_up", attempts = timer.attempts,
                repeats = timer.repeat.is_some(),
                "The turn of a timer was cut off each time it started; this due time is given up");
            advance(&root, timer, now_secs);
            continue;
        }
        // Before a process is started: a scope that denies requests gets no
        // turn, and a closed hosting keeps the timer for when it opens.
        match ledger.request_disposition(&scope, &timer.id) {
            Ok(hagency_agent_local::RequestPolicy::Deny) => {
                tracing::warn!(code = "agent_timer_dropped", reason = "request_policy",
                    "The scope of a timer denies requests; it is removed");
                timers::remove(&root, &timer.id);
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }
        if host.console.hosting_admission().enter_turn().is_none() {
            clock.0 = instant + CLOSED_PAUSE;
            return Ok(());
        }
        timer.attempts += 1;
        if timers::save(&root, &timer).is_err() {
            // An attempt that cannot be counted is not started: a restart in
            // its middle would otherwise start it again without end.
            continue;
        }
        tracing::info!(code = "agent_timer_due", attempt = timer.attempts,
            repeats = timer.repeat.is_some(), late_seconds = now_secs.saturating_sub(timer.due),
            "A timer a local Agent set is due; the host starts its turn");
        let (key, provider) =
            match session(transport, lease, ledger, owner, config, cancel, sessions, &anchor).await {
                Ok(ready) => ready,
                Err(Failure::Stopped) => return Err(RuntimeError::Stopped),
                Err(Failure::Unavailable(reason)) => {
                    tracing::warn!(code = "agent_timer_turn_unavailable", reason,
                        "No process could be made ready for a timer's turn");
                    // Counted as an attempt; the next look tries once more.
                    continue;
                }
            };
        let input = prompt(&timer, now_secs);
        match idle_turns::run(
            transport, lease, ledger, owner, config, cancel, status, sessions, pending, key, provider,
            anchor, Some(input),
        )
        .await
        {
            // The attempt stays counted: after a restart the turn starts once more.
            Err(error) => return Err(error),
            // Stopped from the room: a repeating timer ends with its turn.
            // The room has no other way to end it than through the Agent,
            // and it would start the next turn an interval later.
            Ok(true) if timer.repeat.is_some() => {
                timers::remove(&root, &timer.id);
                tracing::info!(code = "agent_timer_stopped",
                    "The room stopped the turn of a repeating timer; the timer is removed");
            }
            Ok(_) => settle(&root, &timer.id, now()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dispatch(room: &str, thread: &str) -> inbox::Dispatch {
        serde_json::from_value(json!({
            "id":"dsp_one","bindingId":"bnd_one","agentId":"agt_one","eventId":"$event:example.test",
            "roomId":room,"requesterMxid":"@owner:example.test","threadRoot":thread,"body":"",
            "state":"running","bindingGeneration":1
        }))
        .unwrap()
    }

    /// Seen live: a timer's report for a chat that is not encrypted stayed in
    /// custody, because the ledger's record of a run has no body once its
    /// answer is sent and an empty body reads as "came as ciphertext".
    #[test]
    fn a_timer_remembers_whether_its_run_was_encrypted() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let sealed = dispatch("!room:example.test", "$thread:example.test");
        let mut plain = dispatch("!open:example.test", "$thread:example.test");
        plain.body = "remind me in a minute".into();
        call(root, &sealed, set(60, "sealed chat", None));
        call(root, &plain, set(60, "open chat", None));
        let saved = timers::list(root);
        let of = |note: &str| saved.iter().find(|timer| timer.note == note).unwrap().encrypted;
        assert!(of("sealed chat"));
        assert!(!of("open chat"));
        // When the timer is due the record has given up its body.
        let mut stored = plain.clone();
        stored.body.clear();
        let anchor = Anchor::restored(stored.clone(), "exe_one".into(), false);
        assert!(!anchor.encrypted());
        assert!(!anchor.dispatch.body.is_empty());
        assert!(Anchor::restored(stored, "exe_one".into(), true).encrypted());
        // What was noted wins over a body that is still there.
        assert!(Anchor::restored(plain, "exe_one".into(), true).dispatch.body.is_empty());
        // A timer saved before the field existed is read as encrypted.
        let old: Timer = serde_json::from_value(json!({
            "version":1,"id":"wk_00000000000000aa","dispatch":"dsp_one","execution":"exe_one",
            "room":"!open:example.test","thread":"$thread:example.test","due":1,"created":1,
            "note":"n","repeat":null,"attempts":0,"fired":0
        }))
        .unwrap();
        assert!(old.encrypted);
    }
    fn call(root: &std::path::Path, dispatch: &inbox::Dispatch, operation: RoomTimerOperation) -> serde_json::Value {
        serde_json::from_str(&tool(root, dispatch, "exe_one", &operation)).unwrap()
    }
    fn set(seconds: u64, note: &str, repeat: Option<u64>) -> RoomTimerOperation {
        RoomTimerOperation::ScheduleWakeup { delay_seconds: seconds, note: note.into(), repeat_seconds: repeat }
    }

    #[test]
    fn a_chat_sets_lists_and_cancels_only_its_own_timers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        assert!(!timers::any(root));
        let here = dispatch("!room:example.test", "$thread:example.test");
        let elsewhere = dispatch("!room:example.test", "$other:example.test");
        // A look sets nothing up.
        assert_eq!(call(root, &here, RoomTimerOperation::ListWakeups {})["wakeups"], json!([]));
        assert!(!timers::any(root));
        let later = call(root, &here, set(600, "report the build", None));
        let sooner = call(root, &here, set(60, "check the queue", Some(300)));
        let other = call(root, &elsewhere, set(60, "another thread", None));
        assert_eq!(later["status"], "set");
        assert_eq!(sooner["repeat_seconds"], 300);
        let listed = call(root, &here, RoomTimerOperation::ListWakeups {});
        let listed = listed["wakeups"].as_array().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["wakeup_id"], sooner["wakeup_id"], "soonest first");
        assert_eq!(listed[0]["note"], "check the queue");
        // Another thread's timer is neither listed nor cancelled from here.
        let id = other["wakeup_id"].as_str().unwrap().to_owned();
        assert_eq!(
            call(root, &here, RoomTimerOperation::CancelWakeup { wakeup_id: id.clone() })["error"],
            "no_such_timer"
        );
        assert_eq!(
            call(root, &elsewhere, RoomTimerOperation::CancelWakeup { wakeup_id: id })["status"],
            "cancelled"
        );
        let id = later["wakeup_id"].as_str().unwrap().to_owned();
        assert_eq!(
            call(root, &here, RoomTimerOperation::CancelWakeup { wakeup_id: id })["status"],
            "cancelled"
        );
        assert_eq!(timers::list(root).len(), 1);
        let refused = RoomTimerOperation::Invalid { tool: "schedule_wakeup", reason: "the reason" };
        assert_eq!(call(root, &here, refused), json!({"error":"invalid_arguments","hint":"the reason"}));
        assert_eq!(timers::list(root).len(), 1);
        // The saved file is sealed: the note is not on disk in the clear.
        let folder = root.join("outgoing-messages").join("timers");
        for entry in std::fs::read_dir(folder).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("check the queue"));
        }
    }

    #[test]
    fn a_chat_holds_a_bounded_number_of_timers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let here = dispatch("!room:example.test", "$thread:example.test");
        for index in 0..MAX_PER_CHAT {
            assert_eq!(call(root, &here, set(60, &format!("note {index}"), None))["status"], "set");
        }
        assert_eq!(call(root, &here, set(60, "one more", None))["error"], "too_many_timers");
        let elsewhere = dispatch("!room:example.test", "$other:example.test");
        assert_eq!(call(root, &elsewhere, set(60, "its own allowance", None))["status"], "set");
    }

    #[test]
    fn a_timer_that_fired_is_removed_and_a_repeating_one_waits_for_its_next_time() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let here = dispatch("!room:example.test", "$thread:example.test");
        call(root, &here, set(60, "once", None));
        call(root, &here, set(60, "again", Some(120)));
        let now = now();
        for mut timer in timers::list(root) {
            timer.attempts = 1;
            advance(root, timer, now + 60);
        }
        let left = timers::list(root);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].note, "again");
        assert_eq!((left[0].due, left[0].attempts, left[0].fired), (now + 180, 0, 1));
        // It ends by itself after its last run, and after a week.
        let mut last = left[0].clone();
        last.fired = REPEAT_MAX_RUNS - 1;
        advance(root, last, now);
        assert!(timers::list(root).is_empty());
        call(root, &here, set(60, "old", Some(120)));
        let mut old = timers::list(root).remove(0);
        old.created = now - REPEAT_MAX_SECS;
        advance(root, old, now);
        assert!(timers::list(root).is_empty());
    }

    /// Seen live: the Agent removed its repeating timer in the timer's own
    /// turn, the host saved it again when the turn ended, and it kept firing.
    #[test]
    fn a_timer_removed_in_its_own_turn_stays_removed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let here = dispatch("!room:example.test", "$thread:example.test");
        let kept = call(root, &here, set(60, "keeps going", Some(60)));
        let removed = call(root, &here, set(60, "twice is enough", Some(60)));
        let (kept, removed) = (kept["wakeup_id"].as_str().unwrap(), removed["wakeup_id"].as_str().unwrap());
        assert_eq!(timers::get(root, removed).unwrap().note, "twice is enough");
        // Its turn runs, and removes it.
        assert_eq!(
            call(root, &here, RoomTimerOperation::CancelWakeup { wakeup_id: removed.into() })["status"],
            "cancelled"
        );
        let now = now();
        settle(root, removed, now);
        settle(root, kept, now);
        let left = timers::list(root);
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].id.as_str(), left[0].due, left[0].fired), (kept, now + 60, 1));
        assert!(timers::get(root, removed).is_none());
        assert!(timers::get(root, "wk_../../x").is_none());
    }

    #[test]
    fn the_model_is_told_that_a_timer_started_the_turn() {
        let timer = Timer {
            version: 1, id: "wk_00000000000000aa".into(), dispatch: "dsp_one".into(),
            execution: "exe_one".into(), room: "!room:example.test".into(),
            thread: "$thread:example.test".into(), encrypted: true, due: 1_000_600, created: 1_000_000,
            note: "Report the build result.".into(), repeat: Some(600), attempts: 1, fired: 0,
        };
        let text = prompt(&timer, 1_000_600);
        assert!(text.starts_with("[Hagency timer] A timer you set in this chat 10 minutes ago is due now."));
        assert!(text.contains("\n\nReport the build result.\n\n"));
        assert!(text.contains("cancel_wakeup (id wk_00000000000000aa)"));
        assert!(!text.contains("cut off"));
        let again = prompt(&Timer { repeat: None, attempts: 2, ..timer }, 1_000_600);
        assert!(!again.contains("cancel_wakeup"));
        assert!(again.contains("An earlier turn for this timer was cut off"));
    }
}
