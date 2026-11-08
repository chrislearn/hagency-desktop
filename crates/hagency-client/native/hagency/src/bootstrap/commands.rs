//! Answer the `!` command lines this agent's sessions admitted.
//!
//! A `!` line is never agent input — the store's intake gates `wake` off for it
//! (`is_bot_command`) — so nothing downstream would otherwise answer it. The
//! answer is said by THIS agent: it is the member of the room with an
//! authenticated transport, and the identity the retained bridge used once it
//! stopped sending as the bot (`lib/bot-commands.js` `reply`/`sendInto`). It is
//! an `m.notice` in the same thread, carrying the handler's own html when that
//! handler had one.
//!
//! There is no task and no owner approval behind a bot command, so the answer
//! rides its own custody row and reuses the final-reply transport: one bounded
//! pass per attempt, one send per claim, and an unknown outcome ends the attempt
//! to be settled by inspection rather than re-sent.
use super::{Failure, StatusHandle};
use hagency_core::commands::CommandNoticeRequest;
use hagency_matrix::{CancellationToken, Collector, OutgoingState};
use hagency_store::DomainStore;

/// The rest wait for the next poll; one attempt must not become a send loop.
const MAX_ANSWERS: usize = 4;

/// The reason `!offer` reports when its projection is unavailable. Native has no
/// `/api/offer-book` read (the retained product's `GET /api/offer-book`), and
/// the retained product's OWN words for an unreadable offer book are what stand
/// in — never a fabricated "nothing on offer", which would be a claim about the
/// contributor's state that native did not actually read.
const OFFER_UNAVAILABLE: &str = "offer book is not published by a native host";

/// Say every admitted `!` line that has no answer yet, oldest first, per
/// session. A line native parsed and authorized but has no renderer for is left
/// alone on purpose: its retained words came from a handler that does not exist
/// here, and inventing them would put a lie in the room.
pub(super) async fn deliver(
    domain: &DomainStore,
    collector: &Collector,
    sessions: &[String],
    cancel: &CancellationToken,
    status: &StatusHandle,
) -> Result<(), Failure> {
    if sessions.is_empty() {
        return Ok(());
    }
    // Read per attempt, exactly as TS read the policy per dispatch.
    let acl = crate::bot_commands::Acl::from_env();
    let observation = observation(domain).await?;
    for session in sessions {
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        // Render and queue this session's unanswered lines. Queueing is
        // idempotent: the answer's id is derived from the line, so a line
        // already queued or already said is not queued twice.
        let lines = domain
            .pending_command_lines(session.clone(), 16)
            .await
            .map_err(|_| Failure::OutcomeUnknown)?;
        let mut queued = 0usize;
        for line in lines {
            let answer = match crate::bot_commands::dispatch(
                &line.body,
                &line.sender_mxid,
                &acl,
                false,
                &observation,
            ) {
                crate::bot_commands::Dispatched::Answer(reply) => reply,
                // Board #79: `!request` — render the TS reply from the line's
                // arguments. The synchronous refusals (usage, malformed token)
                // never reach any backend in TS either (:526-536); a
                // well-formed line reports the honest no-engagement state
                // until the submit seam is wired (see report-79).
                crate::bot_commands::Dispatched::Request(args) => {
                    crate::bot_commands::request_reply(&args, None)
                }
                crate::bot_commands::Dispatched::Unrenderable => continue,
            };
            let receipt = domain
                .submit_command_notice(CommandNoticeRequest {
                    session_id: line.session_id.clone(),
                    body: answer.plain,
                    html: answer.html,
                    source_event_id: line.event_id,
                })
                .await
                .map_err(|_| Failure::OutcomeUnknown)?;
            // The receipt names the session that OWNS the answer. Two agents in
            // one room race on the same event id, and the store serializes them:
            // the loser gets the WINNER's receipt back. Only the owner queues a
            // send, so the room hears one answer, as the one bridge process made
            // it (bridge-matrix.js:4117).
            if receipt.session_id == line.session_id && receipt.state != "delivered" {
                queued += 1;
            }
        }
        // `/thread` directives are consumed before routing (backend-v2.js:2254-2310):
        // never chat input. A non-operator directive is answered with a refusal
        // notice and not applied; a malformed one with the usage/refusal text;
        // a valid operator one applies the override and answers with the
        // confirmation. Every answer rides the same command-notice custody and
        // send loop as a `!` command, so delivery never fails because of a
        // directive.
        let directives = domain
            .pending_thread_directives(session.clone(), 16)
            .await
            .map_err(|_| Failure::OutcomeUnknown)?;
        for line in directives {
            let body = match hagency_store::parse(&line.body) {
                None => continue,
                Some(Err(notice)) => notice,
                Some(Ok(directive)) => {
                    if !acl.is_operator(&line.sender_mxid) {
                        hagency_store::THREAD_DIRECTIVE_OPERATOR_REFUSAL.to_owned()
                    } else {
                        let overrides = domain
                            .set_session_overrides(line.session_id.clone(), directive)
                            .await
                            .map_err(|_| Failure::OutcomeUnknown)?;
                        hagency_store::confirmation(overrides.model.as_deref(), overrides.mode)
                    }
                }
            };
            let receipt = domain
                .submit_command_notice(CommandNoticeRequest {
                    session_id: line.session_id.clone(),
                    body,
                    html: None,
                    source_event_id: line.event_id,
                })
                .await
                .map_err(|_| Failure::OutcomeUnknown)?;
            // Same one-event-one-answer rule: a directive confirmation is queued
            // by the session that owns the claim. (TS dedups the notice on the
            // Matrix event id too — `router/src/store.ts:2813`.)
            if receipt.session_id == line.session_id && receipt.state != "delivered" {
                queued += 1;
            }
        }
        // Say them, oldest first, with the same custody as a final reply: one
        // send per claim, and an unknown outcome ends the attempt to be settled
        // by the next poll's resume rather than sent a second time.
        for _ in 0..MAX_ANSWERS.min(queued.max(1)) {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            if queued == 0 {
                break;
            }
            status.phase("answering_command");
            let Some(claimed) = domain
                .claim_command_notice_for_session(session.clone(), 60_000)
                .await
                .map_err(|_| Failure::OutcomeUnknown)?
            else {
                break;
            };
            let sent = collector
                .send_command_notice(claimed, cancel)
                .await
                .map_err(|error| {
                    status.matrix_refusal(&error);
                    tracing::warn!(error = ?error, "command answer refused");
                    if cancel.is_cancelled() {
                        Failure::Cancelled
                    } else {
                        Failure::OutcomeUnknown
                    }
                })?;
            if sent.state != OutgoingState::Delivered {
                return Err(Failure::OutcomeUnknown);
            }
            queued -= 1;
        }
    }
    Ok(())
}

/// What native can truthfully say for the tier-1 reads.
///
/// Native has no tmux and no bridge-side group model, so the tmux reads report
/// the source as ABSENT, which is what the retained product rendered when its
/// own `hasTmuxBinary()` was false — `sessions: None` is TS's `sessionCount ===
/// null` / `sessions = null`, the exact input that made `!agents` print `?`
/// instead of calling every agent offline.
async fn observation(
    domain: &DomainStore,
) -> Result<crate::bot_commands::HostObservation, Failure> {
    let roster = domain
        .agent_roster()
        .await
        .map_err(|_| Failure::OutcomeUnknown)?;
    Ok(crate::bot_commands::HostObservation {
        status: crate::bot_commands::StatusObservation {
            agents: roster.len(),
            // Native has no groups; the count of them is truthfully zero.
            groups: 0,
            sessions: None,
            tmux_note: Some(TMUX_NOTE.to_owned()),
        },
        agents: crate::bot_commands::AgentsObservation {
            agents: roster
                .into_iter()
                .map(|row| crate::bot_commands::AgentItem {
                    name: row.name,
                    // The roster carries no display identity; the suffix is
                    // simply absent, which is one of the retained product's own
                    // two renderings (`a.identity ? … : ''`).
                    identity: None,
                })
                .collect(),
            sessions: None,
            tmux_note: Some(TMUX_NOTE.to_owned()),
        },
        sessions: None,
        tmux_installed: false,
        offer: crate::bot_commands::OfferBook {
            error: Some(OFFER_UNAVAILABLE.to_owned()),
            ..Default::default()
        },
    })
}

/// The retained product's own words for a host with no tmux binary
/// (`lib/bot-commands.js`: `tmuxNote = 'tmux binary not found on bridge host'`).
const TMUX_NOTE: &str = "tmux binary not found on bridge host";
