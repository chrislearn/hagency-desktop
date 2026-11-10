//! The agent-invite poller (task #12): the retained
//! `bridge-matrix.js:7894-8131` `pollAgentInvites`, as one background
//! task beside the sweeps — same `Shared` (collector + domain) the file
//! service and receive service use, same shutdown token.
//!
//! TRUST (`getRoomTrust` with `requireTrustedInviter`,
//! `bridge-matrix.js:2841-2851`): TS trusts an inviter pre-declared in
//! `MATRIX_TRUSTED_INVITER_MXIDS`. The store-held equivalent is the
//! owner the room's PROJECT recorded — provisioning wrote exactly that
//! owner (`projects.owner_mxid`), so an invitation from them takes the
//! trusted-inviter arm: join now, no operator round-trip. Anyone else —
//! including an inviter the invite state could not name (NULL, surfaced
//! never guessed, ADR-002) — becomes a PENDING DECISION at the console,
//! exactly the ADR-014 2026-08-11 amendment: joining spends the
//! contributor's tokens.
//!
//! THE BRIDGE NEVER DECIDES IT IS DONE: a join or leave the homeserver
//! refused stays on its worklist and is retried next round with the
//! poller's own backoff — never terminal, never a process kill
//! (ADR-183). A successful join settles any stale pending record as
//! `accepted` by `trusted-inviter` (the reconcile at
//! `bridge-matrix.js:7926-7945`: the agent is in the room, so the
//! invitation WAS answered — by policy, never credited to a person).
use super::Shared;
use hagency_matrix::{CancellationToken, Collector};
use std::time::Duration;

/// The retained product's invite-poll cadence: a few seconds
/// (`bridge-matrix.js` runs the poll on the services timer). Diagnostic
/// only — the poller is idempotent, so a missed tick costs nothing.
const POLL_PERIOD: Duration = Duration::from_secs(10);
/// Backoff after a refused poll round, doubling to the cap (ADR-183:
/// 1 s → 60 s shape; a poll is a read, so the cap is modest).
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Start the poller as a background task: one `poll_once` per period
/// until `shutdown` cancels, with backoff after a refused round.
pub(super) fn start(shared: Shared, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = BACKOFF_MIN;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(POLL_PERIOD) => {}
            }
            match poll_round(&shared.collector, &shared.domain, &shutdown).await {
                Ok(()) => backoff = BACKOFF_MIN,
                Err(reason) => {
                    tracing::warn!(
                        reason,
                        retry_in_ms = backoff.as_millis() as u64,
                        "agent invite poll refused; retrying"
                    );
                    tokio::select! {
                        _ = shutdown.cancelled() => return,
                        _ = tokio::time::sleep(backoff) => {}
                    }
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                }
            }
        }
    })
}

/// One poll round. Every failure is reported, never acted on terminally:
/// the next round re-reads the same state, so a transient fault costs a
/// tick, not an invitation.
pub async fn poll_round(
    collector: &Collector,
    domain: &hagency_store::DomainStore,
    cancel: &CancellationToken,
) -> Result<(), &'static str> {
    // The record key is the agent NAME (TS `rememberPendingInvite`'s
    // second argument); the engagement owns that name.
    let agent = domain
        .engagement_agent(collector.engagement_id().to_owned())
        .await
        .map_err(|_| "agent_name_unavailable")?
        .ok_or("engagement_has_no_agent")?;

    // A 409 on the lightweight sync is the busy shape — retry next round.
    let invites = collector
        .observe_invites(cancel)
        .await
        .map_err(|_| "invite_sync_refused")?;

    for invite in &invites {
        // Backfill runs BEFORE the trust branch (TS `:8043-8050`): the
        // poll that could not read the inviter recorded NULL; this round
        // may resolve the sender. It only ever fills a null on a pending
        // record.
        if let Some(inviter) = &invite.inviter {
            let _ = domain
                .backfill_pending_invite_inviter(
                    invite.room_id.clone(),
                    agent.clone(),
                    inviter.clone(),
                )
                .await;
        }
        // The trusted-inviter arm: the room's project recorded this
        // inviter as its owner.
        let owner = domain
            .room_owner(invite.room_id.clone())
            .await
            .map_err(|_| "room_owner_unavailable")?;
        // ADR-187: the agent's own owner is a trusted inviter too (the
        // store-held equivalent of TS `MATRIX_TRUSTED_INVITER_MXIDS` for an
        // imported fleet), so an owner's invitation into a room that is no
        // project's is joined, not parked.
        let agent_owner = domain
            .engagement_owner(collector.engagement_id().to_owned())
            .await
            .ok()
            .flatten();
        let trusted = [owner.as_deref(), agent_owner.as_deref()]
            .into_iter()
            .flatten()
            .any(|owner| {
                invite
                    .inviter
                    .as_deref()
                    .is_some_and(|inviter| inviter.eq_ignore_ascii_case(owner))
            });
        if trusted {
            // Join now; a refusal surfaces below through the worklist
            // reconcile rather than aborting the round.
            match collector.join_room(&invite.room_id, cancel).await {
                Ok(joined) => {
                    bind_joined_room(domain, collector.engagement_id(), &joined).await;
                    let _ = domain
                        .settle_pending_invite(
                            invite.room_id.clone(),
                            agent.clone(),
                            true,
                            true,
                            "trusted-inviter".to_owned(),
                        )
                        .await;
                }
                Err(_) => {
                    // Parked visibly: the invitation stays pending for a
                    // human, never silently dropped (ADR-183).
                    let _ = domain
                        .remember_pending_invite(
                            invite.room_id.clone(),
                            agent.clone(),
                            invite.inviter.clone(),
                            invite.mode().to_owned(),
                            invite.origin_server_ts.unwrap_or(0),
                        )
                        .await;
                }
            }
        } else {
            // The pending decision — remembered (a decline is never
            // resurrected), then the console answers it.
            domain
                .remember_pending_invite(
                    invite.room_id.clone(),
                    agent.clone(),
                    invite.inviter.clone(),
                    invite.mode().to_owned(),
                    invite.origin_server_ts.unwrap_or(0),
                )
                .await
                .map_err(|_| "invite_record_refused")?;
        }
    }

    // The console decisions' worklist: joins the operator accepted. Each
    // refusal leaves its row owed; the next round retries.
    for (room, agent_name) in domain
        .join_pending_invites(agent.clone())
        .await
        .map_err(|_| "join_worklist_refused")?
    {
        match collector.join_room(&room, cancel).await {
            Ok(joined) => {
                bind_joined_room(domain, collector.engagement_id(), &joined).await;
                let _ = domain.mark_invite_joined(room, agent_name).await;
            }
            Err(_) => tracing::warn!(room, "console-accepted join refused; retrying next round"),
        }
    }

    // The declines' worklist: leaving is courtesy, the decision is the
    // record (TS `:9135-9147`); a refused leave retries next round.
    for (room, agent_name) in domain
        .leave_pending_invites(agent.clone())
        .await
        .map_err(|_| "leave_worklist_refused")?
    {
        match collector.leave_room(&room, cancel).await {
            Ok(()) => {
                let _ = domain.mark_invite_left(room, agent_name).await;
            }
            Err(_) => tracing::warn!(room, "decline leave refused; retrying next round"),
        }
    }
    Ok(())
}

/// ADR-188 §2: record a joined room for the agent's engagement. The agent's
/// driver reads it on its next pass and, once the room is observed safe,
/// reads and answers there. Re-joining a room makes it working again.
async fn bind_joined_room(domain: &hagency_store::DomainStore, engagement_id: &str, room: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if let Err(error) = domain
        .record_joined_room(engagement_id.to_owned(), room.to_owned(), now)
        .await
    {
        tracing::warn!(?error, room, "joined room could not be recorded");
    }
}
