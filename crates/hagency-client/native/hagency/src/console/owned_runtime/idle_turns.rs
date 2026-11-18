//! Turns a retained native CLI starts by itself while its session is parked
//! (ADR 0013): a timer the session set, a message from a peer session.
//!
//! Such a turn has no request and therefore no dispatch. It reports to the
//! room and thread of the run its session answered last: every message it
//! produces, its final answer included, is recorded as a report in that run's
//! encrypted outbox and sent with fresh delivery-only authority. The turn
//! itself is admitted, charged and settled in the local ledger like a
//! requested one, under an identity of its own.
//!
//! Only a Claude session starts a turn by itself; a Codex or Octos session is
//! not looked at. A timer the Agent set with the host (`wakeups`) runs its
//! turn through the same `run`, for every runtime.
//!
//! The turn shows a card of its own on the message of that run, where the
//! run's requester or the owner can stop it, and a question the model asks
//! goes to the same two. Words cannot be added to the turn from the room,
//! and the room's history and file tools answer "unavailable": they act for
//! a request that is running on the server.
use super::provider_custody::{Anchor, Provider, SessionKey, Sessions};
use super::*;

const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

/// A run whose outbox may still hold reports of a self-started turn.
pub(super) struct Undelivered {
    anchor: Anchor,
    due: tokio::time::Instant,
    pause: Duration,
}
fn remember(pending: &mut Vec<Undelivered>, anchor: &Anchor) {
    let now = tokio::time::Instant::now();
    match pending.iter_mut().find(|entry| {
        entry.anchor.dispatch.id == anchor.dispatch.id && entry.anchor.execution == anchor.execution
    }) {
        Some(entry) => {
            entry.due = now;
            entry.pause = RETRY_FIRST;
        }
        None => pending.push(Undelivered { anchor: anchor.clone(), due: now, pause: RETRY_FIRST }),
    }
}

/// After a restart: the runs of this binding that were noted as holding
/// reports of a self-started turn. Their delivery resumes; no turn is rerun.
pub(super) fn recover<T: Transport>(
    transport: &T,
    lease: &LeaseRef,
    ledger: &Ledger,
    owner: &str,
    config: &StartConfig,
) -> Vec<Undelivered> {
    let mut pending = Vec::new();
    if transport.approval_host().is_none() {
        return pending;
    }
    let Ok(root) = e2ee::message_root(transport, lease, config) else {
        return pending;
    };
    for (dispatch, execution, encrypted) in messages::undelivered::list(&root) {
        let Ok(record) = ledger.inbox_record(owner, &dispatch) else {
            continue;
        };
        // The Agent's bindings share one device folder: each loop takes its own.
        if record.dispatch.binding_id == config.binding_id
            && record.dispatch.agent_id == lease.agent_id
            && record.execution_id.as_deref() == Some(execution.as_str())
        {
            remember(&mut pending, &Anchor::restored(record.dispatch, execution, encrypted));
        }
    }
    pending
}

/// True once the run's outbox has nothing pending.
async fn flush<T: Transport>(
    transport: &T,
    lease: &LeaseRef,
    config: &StartConfig,
    cancel: &mut watch::Receiver<bool>,
    anchor: &Anchor,
) -> Result<bool, RuntimeError> {
    let Some(route) = messages::Route::personal(&anchor.dispatch, &anchor.execution) else {
        return Ok(true);
    };
    let held = |stage: &'static str| {
        tracing::warn!(code = "agent_report_held", stage,
            "A report of an Agent's own turn was not sent at this step; it stays in custody");
    };
    let Ok(root) = e2ee::message_root(transport, lease, config) else {
        held("message_root");
        return Ok(false);
    };
    if !messages::Outbox::exists_for_route(&root, &route) {
        return Ok(true);
    }
    let Ok(outbox) = messages::Outbox::open_route(&root, route) else {
        held("outbox");
        return Ok(false);
    };
    if matches!(outbox.next(), Ok(None)) {
        return Ok(true);
    }
    let device = match e2ee::device(transport, lease, config, cancel).await {
        Ok(device) => device,
        Err(RuntimeError::Stopped) => return Err(RuntimeError::Stopped),
        Err(_) => {
            held("device");
            return Ok(false);
        }
    };
    for _ in 0..64 {
        if matches!(outbox.next(), Ok(None)) {
            return Ok(true);
        }
        if send_personal_message(
            transport, lease, &anchor.dispatch, &anchor.execution, &outbox, &device, cancel,
        )
        .await
        .is_err()
        {
            return if *cancel.borrow() { Err(RuntimeError::Stopped) } else { Ok(false) };
        }
    }
    Ok(false)
}

/// Send what the noted runs still hold. A run that cannot be delivered now is
/// asked again with growing pauses; its reports stay in encrypted custody.
pub(super) async fn deliver<T: Transport>(
    transport: &T,
    lease: &LeaseRef,
    config: &StartConfig,
    cancel: &mut watch::Receiver<bool>,
    pending: &mut Vec<Undelivered>,
) -> Result<(), RuntimeError> {
    let mut index = 0;
    while index < pending.len() {
        if pending[index].due > tokio::time::Instant::now() {
            index += 1;
            continue;
        }
        let anchor = pending[index].anchor.clone();
        // A send cut off here stays an attempt on record: the next pass
        // reconciles it against the room before anything is sent again.
        let flushed = tokio::time::timeout(
            Duration::from_secs(30),
            flush(transport, lease, config, cancel, &anchor),
        )
        .await;
        match flushed {
            Ok(Err(error)) => return Err(error),
            Ok(Ok(true)) => {
                if let Ok(root) = e2ee::message_root(transport, lease, config) {
                    messages::undelivered::forget(&root, &anchor.dispatch.id, &anchor.execution);
                }
                pending.remove(index);
            }
            Ok(Ok(false)) | Err(_) => {
                tracing::warn!(code = "agent_self_started_report_pending",
                    retry_seconds = pending[index].pause.as_secs(),
                    "Reports of a self-started turn remain in encrypted local custody");
                let entry = &mut pending[index];
                entry.due = tokio::time::Instant::now() + entry.pause;
                entry.pause = (entry.pause * 3).min(RETRY_MAX);
                index += 1;
            }
        }
    }
    Ok(())
}

/// Look at the parked sessions and run every turn one of them started by
/// itself. Only a stop of the runtime is an error: a turn that is refused or
/// fails ends with its process, and requests continue.
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
    pending: &mut Vec<Undelivered>,
) -> Result<(), RuntimeError> {
    while let Some((key, provider, anchor)) = sessions.take_self_started().await {
        if *cancel.borrow() {
            return Err(RuntimeError::Stopped);
        }
        run(
            transport, lease, ledger, owner, config, cancel, status, sessions, pending, key, provider,
            anchor, None,
        )
        .await?;
    }
    Ok(())
}

async fn refuse(mut provider: Provider, reason: &'static str) -> Result<bool, RuntimeError> {
    // The turn already runs inside the CLI. Without admission its process is
    // stopped; the conversation resumes from disk with the next request.
    tracing::warn!(code = "agent_self_started_turn_refused", reason,
        "A turn the native CLI started by itself was not admitted; its process is stopped");
    let _ = provider.stop().await;
    Ok(false)
}

/// Run a turn nobody requested and report it to the room of `anchor`.
/// `input` is `None` for the turn the native CLI already started in
/// `provider`, and the host's own words for a turn the host starts for a
/// timer (`wakeups`). Only a stop of the runtime is an error. `true` says
/// that someone in the room stopped the turn.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run<T: Transport>(
    transport: &T,
    lease: &LeaseRef,
    ledger: &mut Ledger,
    owner: &str,
    config: &StartConfig,
    cancel: &mut watch::Receiver<bool>,
    status: &watch::Sender<RuntimeStatus>,
    sessions: &mut Sessions,
    pending: &mut Vec<Undelivered>,
    key: SessionKey,
    mut provider: Provider,
    anchor: Anchor,
    input: Option<String>,
) -> Result<bool, RuntimeError> {
    let scope = anchor.dispatch.scope();
    let Some(host) = transport.approval_host() else {
        return refuse(provider, "no_message_host").await;
    };
    if anchor.dispatch.binding_id != config.binding_id || anchor.dispatch.agent_id != lease.agent_id {
        return refuse(provider, "binding_changed").await;
    }
    let Some(route) = messages::Route::personal(&anchor.dispatch, &anchor.execution) else {
        return refuse(provider, "no_room_route").await;
    };
    let mut random = [0u8; 16];
    if getrandom::fill(&mut random).is_err() {
        return refuse(provider, "local_state_unavailable").await;
    }
    // The ledger's call and dispatch identity of this turn. No server
    // dispatch has it, and it grants nothing outside this host's ledger.
    let turn = format!("self-started-{:032x}", u128::from_be_bytes(random));
    // The turn continues a conversation that was already admitted: nobody is
    // asked for it. Only a scope whose requests are denied refuses it here;
    // the budget is checked at admission.
    match ledger.request_disposition(&scope, &turn) {
        Ok(hagency_agent_local::RequestPolicy::Deny) => {
            return refuse(provider, "request_policy").await;
        }
        Ok(_) => {}
        Err(_) => return refuse(provider, "local_state_unavailable").await,
    }
    let Some(mut admission) = host.console.hosting_admission().enter_turn() else {
        return refuse(provider, "hosting_closed").await;
    };
    let Ok(root) = e2ee::message_root(transport, lease, config) else {
        return refuse(provider, "local_state_unavailable").await;
    };
    // Noted before the first message can be recorded: a restart in the
    // middle of this turn still finds what it left to deliver.
    if messages::undelivered::note(&root, &anchor.dispatch.id, &anchor.execution, anchor.encrypted())
        .is_err()
    {
        return refuse(provider, "local_state_unavailable").await;
    }
    let Ok((queue, mut receiver)) = codex::approval_queue(1) else {
        return refuse(provider, "local_state_unavailable").await;
    };
    // A question of the model goes to the requester of the run this turn
    // reports to and to the owner, on that run's message.
    let (question_queue, question_receiver) = hagency_agent_local::questions::question_queue();
    let mut queue = queue.with_questions(question_queue);
    // The turn gets a card of its own on that message: the room sees that
    // the Agent works, and the same two people can stop it. A run whose
    // requester cannot hold a card (another Agent) goes without one.
    let kind = runtime::kind(&config.credential_ref).name();
    let control = match run_controls::optional_card(&anchor.dispatch, &turn, owner, kind) {
        Some(card) => {
            let Ok((sender, inbox)) =
                hagency_agent_local::run_controls::channel(&scope, &turn, &turn)
            else {
                return refuse(provider, "local_state_unavailable").await;
            };
            queue = queue.with_run_controls(inbox);
            Some((sender, card))
        }
        None => None,
    };
    let Ok(controls) = run_controls::Controller::self_started(route, &root, &turn, control) else {
        return refuse(provider, "local_state_unavailable").await;
    };
    let mut queue = queue.with_live_text(controls.live_text());
    // Tool requests of this turn are shown on the message whose run the
    // session answered last, and answered under the same saved tool rules.
    let asking = (anchor.dispatch.event_id.clone(), anchor.dispatch.thread_root.clone());
    let console = host.clone();
    let bridge = tokio::spawn(async move {
        while let Some(pending) = receiver.recv().await {
            if console
                .console
                .0
                .owned_runtime
                .offer_approval(&console.console, &console.pin, pending, &asking.0, &asking.1)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let _approval_bridge = AbortHeartbeat(bridge.abort_handle());
    // The retained process was started with room tools. Their gate asks the
    // server whether the anchor run is still running; it is not, so each call
    // is answered "unavailable" and the turn continues.
    let gate = Arc::new(LiveToolGate {
        console: host.clone(),
        lease: lease.clone(),
        expected: anchor.dispatch.clone(),
        execution: anchor.execution.clone(),
        cancel: cancel.clone(),
    });
    let room_bridge = match (
        room_tools::Host::new(gate, config, &config.profile.cwd).map(|host| host.for_owner(owner)),
        hagency_agent_local::room_tools::room_queue(&scope, &turn),
    ) {
        (Ok(room_host), Ok((channel, room_receiver))) => {
            queue = queue.with_room_tools(channel);
            (room_host, room_receiver)
        }
        _ => return refuse(provider, "local_state_unavailable").await,
    };
    let mut executing =
        RuntimeStatus::new(&config.agent_id, "executing_chat_only", None).with_capabilities(config);
    executing.binding_id = Some(config.binding_id.clone());
    status.send_replace(executing);
    let telemetry = host
        .console
        .0
        .dashboard_telemetry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let binding = hagency_metering::dashboard::ledger::ExecutionBinding {
        execution_id: turn.clone(),
        resource_id: format!("personal-agent:{}", config.agent_id),
        account_id: None,
        allocation_id: None,
        agent_id: Some(config.agent_id.clone()),
        requester_id: Some(scope.requester.clone()),
        room_id: Some(scope.room.clone()),
        runtime: match runtime::kind(&config.credential_ref) {
            runtime::Kind::Codex => hagency_metering::dashboard::ledger::Runtime::Codex,
            runtime::Kind::Claude => hagency_metering::dashboard::ledger::Runtime::Claude,
            runtime::Kind::Octos => hagency_metering::dashboard::ledger::Runtime::Octos,
        },
        runtime_version: None,
        model: Some(config.profile.model.clone()),
        parent_execution_id: None,
        started_at_ms: super::super::native::telemetry::now_ms(),
    };
    let usage_observer = |usage: &hagency_agent_local::Usage| {
        if let Some(telemetry) = &telemetry {
            if telemetry.personal_usage(&binding, usage).is_err() {
                tracing::warn!(code = "personal_usage_unavailable",
                    "Personal runtime measurement could not be recorded");
            }
        }
    };
    if input.is_some() {
        tracing::info!(code = "agent_timer_turn",
            "The host starts the turn of a timer the Agent set; it runs as a turn of its own");
    } else {
        tracing::info!(code = "agent_self_started_turn",
            "A parked native session started a turn by itself; it runs as a turn of its own");
    }
    admission.started();
    // The messages are sent under the anchor run's route while the turn runs.
    let prepared = inbox::Prepared {
        dispatch: anchor.dispatch.clone(),
        execution_id: anchor.execution.clone(),
    };
    // A question carries this turn's own identity; its card is put where
    // the anchor run's request is.
    let asked = inbox::Prepared {
        dispatch: inbox::Dispatch { id: turn.clone(), ..anchor.dispatch.clone() },
        execution_id: turn.clone(),
    };
    let control_cancel = cancel.clone();
    let question_cancel = cancel.clone();
    let (room_host, room_receiver) = room_bridge;
    // A timer's turn starts with the room's standing role, as a request does.
    let input = input.map(|note| format!("{}{note}", room_host.saved_role()));
    let result = tokio::select! { biased;
        _ = canceled(cancel) => Err(RuntimeError::Stopped),
        result = async {
            tokio::select! {
                result = async {
                    match input.as_deref() {
                        None => provider.session.run_self_started(
                            ledger, owner, &scope, &turn, &turn, BudgetMode::Postpaid, &queue,
                            Some(&usage_observer),
                        ).await,
                        Some(input) => provider.session.run_host_started(
                            ledger, owner, &scope, &turn, &turn, input, BudgetMode::Postpaid, &queue,
                            Some(&usage_observer),
                        ).await,
                    }
                } => result,
                _ = room_host.serve(room_receiver) => Err(codex::Error::Protocol("room bridge closed")),
                _ = questions::serve(transport, lease, config, owner, &asked, question_cancel, question_receiver) => Err(codex::Error::Protocol("question bridge closed")),
                _ = async {
                    controls.serve(transport, lease, config, &prepared, control_cancel).await;
                    // Delivery that cannot start now does not end a paid turn.
                    std::future::pending::<()>().await
                } => unreachable!(),
            }
        } => Ok(result),
    };
    drop(admission);
    let completed = match result {
        Err(error) => {
            let _ = provider.stop().await;
            remember(pending, &anchor);
            return Err(error);
        }
        Ok(Err(codex::Error::Ledger(
            hagency_agent_local::Error::Denied | hagency_agent_local::Error::Budget,
        ))) => {
            // Refused before the ledger admitted anything: nothing is charged.
            remember(pending, &anchor);
            return ready(config, status, refuse(provider, "policy_or_budget").await);
        }
        Ok(Err(error)) => {
            provider_failure("agent_self_started_turn", &error);
            None
        }
        Ok(Ok(completed)) => {
            if let Some(error) = &completed.late_failure {
                provider_failure("agent_self_started_turn_after_answer", error);
            }
            Some(completed)
        }
    };
    // The final answer of a requested turn is its reply. Here nobody waits
    // for a reply: the answer is the last report of the turn.
    if let Some(completed) = &completed
        && !completed.text.is_empty()
    {
        if completed.text.len() > 65_536
            || controls.live_text().message(&format!("answer:{turn}"), &completed.text).is_err()
        {
            tracing::warn!(code = "agent_self_started_answer_dropped",
                "The final answer of a self-started turn could not be recorded for delivery");
        }
    }
    let stopped = matches!(&completed,
        Some(completed) if completed.status == codex::TurnStatus::Interrupted);
    {
        use crate::agent_run_controls::State;
        controls.finish(match &completed {
            Some(_) if stopped => State::Interrupted,
            Some(_) => State::Completed,
            None => State::Failed,
        });
    }
    drop(controls);
    let retain = matches!(&completed, Some(completed) if completed.usage.is_reported())
        && provider.session.can_continue();
    tracing::info!(code = "agent_self_started_turn_ended",
        answered = completed.is_some(),
        usage_reported = completed.as_ref().is_some_and(|completed| completed.usage.is_reported()),
        retained = retain,
        stopped_from_room = stopped,
        "A self-started turn ended");
    if retain {
        sessions.retain(key, provider, anchor.clone());
    } else {
        let _ = provider.stop().await;
    }
    remember(pending, &anchor);
    ready(config, status, Ok(stopped))
}

fn ready<T>(
    config: &StartConfig,
    status: &watch::Sender<RuntimeStatus>,
    result: Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let mut ready = RuntimeStatus::new(
        &config.agent_id,
        if config.shared_workspace { "online" } else { "online_chat_only" },
        None,
    )
    .with_capabilities(config);
    ready.binding_id = Some(config.binding_id.clone());
    status.send_replace(ready);
    result
}
