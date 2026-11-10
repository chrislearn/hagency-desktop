//! Idle-agent membership sweep (board #71; parity with the retained
//! `backend-v2.js:14192-14228` `sweepProjectRoomMembership`, scheduled
//! hourly at `:17511` via `PROJECT_ROOM_MEMBERSHIP_SWEEP_INTERVAL_MS`
//! `:9351`).
//!
//! WHY A SWEEP AT ALL (the retained comment, in substance): a send that
//! fails on membership re-invites and rejoins (`outgoing.rs` already ports
//! that half), so an agent that WORKS heals itself. An idle one does not —
//! its membership can be dropped by a kick, a room upgrade or a server-side
//! cleanup, and the next thing that notices is the next message, which may
//! be days away and will be the one that fails.
//!
//! THE PROBE IS THE INVITE, idempotent by reuse rather than by a new code
//! path: a second invite of a member is the 403 the retained product was
//! written to read correctly ("the invite 403 IS the check",
//! tests/api-engagement-room-admission.test.js:472-478). A kicked member
//! needs a fresh invite no agent can mint for itself
//! (bridge-matrix.js:10936); the representative is the one with standing,
//! so the invite is issued on the collector's own credential — the account
//! `verify_request` already requires to be joined to every project room.
//! The agent-side join after the invite is the acceptance path's own act
//! (native provisions per-agent accounts with their own sync); the sweep
//! never touches it, exactly as the retained sweep only invites through
//! the side and lets the join happen where it always did.
//!
//! ONE PASS PER (agent, room), not per engagement: six concurrent
//! engagements between one agent and one room is a real shape in the
//! retained deployment's data, and a sweep that asked per engagement would
//! make five needless calls to a customer's homeserver every hour.
//!
//! NEVER TERMINAL: a per-pair failure is counted and the sweep reaches the
//! next pair ("one agent failing does not stop the sweep reaching the
//! next", the same test file :512); a store read failure aborts the pass
//! and the loop retries with the retained 1 s -> 60 s backoff, reset by the
//! first clean pass. No outcome kills the loop or the process.

use crate::CancellationToken;
use hagency_core::project::{Engagement, EngagementState};
use serde_json::json;
use std::collections::BTreeSet;

/// The retained interval (backend-v2.js:9351): hourly, because the condition
/// is standing and a tighter loop would re-ask a foreign homeserver about
/// rooms nothing has changed.
pub const MEMBERSHIP_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);
/// The bridge-fault backoff (RULES.md operator rule 1): 1 s doubling to 60 s,
/// reset by the first pass that read its engagements.
const SWEEP_BACKOFF_MIN: std::time::Duration = std::time::Duration::from_secs(1);
const SWEEP_BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(60);

/// One sweep pass, in the shape an operator reads: what was considered, what
/// was skipped, and the split between a real re-invite and the expected
/// already-present answer. `read_failed` marks a pass that could not even
/// read the engagements — the loop's backoff signal, never a panic.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    pub pairs: usize,
    pub skipped_no_side: usize,
    pub invited: usize,
    pub present: usize,
    pub failed: usize,
    pub read_failed: bool,
}

/// The live (agent, room) pairs, one per pair however many engagements share
/// it (retained backend-v2.js:14212-14217: the `seen` set on
/// `agentName\u0000roomId`). The store's live-name index cannot hold two
/// engagements on one (fleet, project, name), so the dedup matters across
/// agents in one room and across pages, not within one project.
pub fn active_pairs(engagements: &[Engagement]) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    engagements
        .iter()
        .filter(|e| e.state == EngagementState::Active)
        .filter(|&e| seen.insert((e.agent_name.as_str().to_owned(), e.project_room_id.clone())))
        .map(|e| (e.agent_name.as_str().to_owned(), e.project_room_id.clone()))
        .collect()
}

/// What one invite answered. The 403 whose `error` says "already in the
/// room" is the retained membership check PASSING, not failing — the exact
/// branch `admitAgentToProjectRoom` was written to read correctly.
///
/// A FAILURE CARRIES THE OBSERVED CAUSE, never a bare word (LESSONS.md
/// "Failure reasons must be recorded": every refusal says what was refused
/// and why). The retained sweep logs `${error?.message || error}`
/// (`backend-v2.js:14225`), so a `M_FORBIDDEN` an operator sees here is the
/// homeserver's own text — a bare "invite refused" is what sent a live run
/// hunting the credential when the real cause was a power level.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invite {
    Invited,
    AlreadyPresent,
    Failed(String),
}
/// Bound on the reason carried out of a foreign homeserver's error text —
/// the same 256-byte, char-boundary rule `approval_delivery.rs:137-142`
/// applies to another peer's words.
fn bounded_reason(value: &str) -> String {
    let mut text = value.to_owned();
    let mut cut = text.len().min(256);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text
}
/// The refusal text for a non-2xx invite: the homeserver's OWN words when it
/// sent any (`errcode: error`, the Matrix error document), otherwise the
/// status alone. Never invented, never blank.
fn invite_refusal(status: u16, value: Option<&serde_json::Value>) -> String {
    let code = value
        .and_then(|v| v.get("errcode"))
        .and_then(serde_json::Value::as_str);
    let error = value
        .and_then(|v| v.get("error"))
        .and_then(serde_json::Value::as_str);
    match (code, error) {
        (Some(code), Some(error)) => bounded_reason(&format!("{code}: {error}")),
        (None, Some(error)) => bounded_reason(error),
        (Some(code), None) => format!("invite answered HTTP {status} ({code})"),
        (None, None) => format!("invite answered HTTP {status}"),
    }
}
pub(crate) fn classify_invite(status: u16, value: Option<&serde_json::Value>) -> Invite {
    match status {
        200 => Invite::Invited,
        403 => {
            // The membership check, read from the observed error text: a 403
            // that says "already in the room" is the check PASSING.
            let already = value
                .and_then(|v| v.get("error"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("already in the room"));
            if already {
                Invite::AlreadyPresent
            } else {
                Invite::Failed(invite_refusal(status, value))
            }
        }
        status => Invite::Failed(invite_refusal(status, value)),
    }
}

impl crate::Collector {
    /// The production loop: one sweep per hour, a failed read retried with
    /// the retained backoff instead of waiting out the hour. Ends only on
    /// `cancel` — a bridge-side fault is never terminal (operator rule 1).
    pub async fn membership_sweep_loop(self: std::sync::Arc<Self>, cancel: CancellationToken) {
        run_loop(&cancel, || self.sweep_project_room_membership(&cancel)).await
    }

    /// Re-admit agents whose room membership was lost while they had nothing
    /// to say. Reads the engagements, keeps the active ones' (agent, room)
    /// pairs deduplicated, skips rooms the fleet's registration does not
    /// cover without any call, and asks the membership question exactly the
    /// way the acceptance path asks it: one invite per pair per pass.
    pub async fn sweep_project_room_membership(&self, cancel: &CancellationToken) -> SweepOutcome {
        let inner = &self.inner;
        let Ok(reg) = inner
            .domain
            .provisioning_registration_for_engagement(
                inner.config.identity.transport.engagement_id.clone(),
            )
            .await
        else {
            return SweepOutcome {
                read_failed: true,
                ..SweepOutcome::default()
            };
        };
        sweep(&inner.http, &inner.domain, &reg, cancel).await
    }
}

/// ADR-187 §A.5: an imported fleet's sweep, acting with the representative's
/// credential (the account with invite standing in every project room).
pub struct MembershipSweep {
    pub(crate) http: crate::http::Http,
    pub(crate) domain: hagency_store::DomainStore,
    pub(crate) registration: hagency_core::authority::Registration,
}
impl MembershipSweep {
    pub async fn pass(&self, cancel: &CancellationToken) -> SweepOutcome {
        sweep(&self.http, &self.domain, &self.registration, cancel).await
    }
    pub async fn run(self: std::sync::Arc<Self>, cancel: CancellationToken) {
        run_loop(&cancel, || self.pass(&cancel)).await
    }
}

/// One sweep per hour, the first after one full period; a failed read is
/// retried with the retained backoff. Ends only on `cancel`.
async fn run_loop<F, Fut>(cancel: &CancellationToken, mut pass: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = SweepOutcome>,
{
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + MEMBERSHIP_SWEEP_INTERVAL,
        MEMBERSHIP_SWEEP_INTERVAL,
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut backoff = SWEEP_BACKOFF_MIN;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {}
        }
        let outcome = pass().await;
        if outcome.read_failed {
            eprintln!("[readmit] sweep failed: engagements unreadable; retrying in {backoff:?}");
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(SWEEP_BACKOFF_MAX);
        } else {
            backoff = SWEEP_BACKOFF_MIN;
        }
    }
}

/// One membership pass over the fleet's active (agent, room) pairs.
async fn sweep(
    http: &crate::http::Http,
    domain: &hagency_store::DomainStore,
    reg: &hagency_core::authority::Registration,
    cancel: &CancellationToken,
) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();
    let mut engagements = Vec::new();
    let mut after = String::new();
    loop {
        let Ok(page) = domain.engagements(after.clone(), 100).await else {
            outcome.read_failed = true;
            return outcome;
        };
        let counted = page.len();
        engagements.extend(page);
        if counted < 100 {
            break;
        }
        after = engagements
            .last()
            .expect("a full page is non-empty")
            .id
            .clone();
    }
    for (agent, room) in active_pairs(&engagements) {
        outcome.pairs += 1;
        // The retained early exit (`!sideIdForRoom(roomId)`,
        // backend-v2.js:14219): a room the side table does not cover is
        // skipped here as well as inside the admit call, because
        // re-learning "not ours" per hour is work with no possible
        // outcome. Native's single-registration equivalent is the room's
        // server name against the registration's.
        let Some((_, server)) = room.split_once(':') else {
            outcome.skipped_no_side += 1;
            continue;
        };
        if !server.eq_ignore_ascii_case(&reg.server_name) {
            outcome.skipped_no_side += 1;
            continue;
        }
        // The native provisioned-account composition the enrollment
        // fixtures model (`tests/provision_rooms/mod.rs:29-33`):
        // `@{fleet}_{engagement}:{server}` — the identity the acceptance
        // path itself admits. The engagement backing the pair is the
        // first live one naming it, mirroring the retained
        // `admitAgentToProjectRoom(engagement)` read.
        let Some(engagement_id) = engagement_for(&engagements, &agent, &room) else {
            continue;
        };
        let agent_mxid = format!("@{}_{}:{}", reg.fleet_id, engagement_id, reg.server_name);
        let body = json!({ "user_id": agent_mxid }).to_string();
        let result = http
            .post(
                &["_matrix", "client", "v3", "rooms", &room, "invite"],
                body,
                cancel,
            )
            .await;
        let invite = match result {
            Ok(response) => classify_invite(response.status, response.value.as_ref()),
            // The transport's own Display, never a bare word: a timeout,
            // a redirect refusal and a malformed body are three different
            // operator actions (`native_sweep` logs this verbatim).
            Err(error) => Invite::Failed(bounded_reason(&error.to_string())),
        };
        match invite {
            Invite::Invited => {
                outcome.invited += 1;
                // Only a RE-admission is worth a line (retained
                // backend-v2.js:14221-14223): `alreadyMember` is the
                // expected answer and logging it would bury the one case
                // an operator wants.
                eprintln!("[readmit] {agent} was not in {room} and has been let back in");
            }
            Invite::AlreadyPresent => outcome.present += 1,
            Invite::Failed(reason) => {
                outcome.failed += 1;
                eprintln!("[readmit] {agent} in {room}: {reason}");
            }
        }
    }
    outcome
}

/// The engagement id backing an (agent, room) pair — the first live one
/// naming exactly that pair, which is all the mxid composition needs.
fn engagement_for(engagements: &[Engagement], agent: &str, room: &str) -> Option<String> {
    engagements
        .iter()
        .find(|e| {
            e.state == EngagementState::Active
                && e.agent_name.as_str() == agent
                && e.project_room_id == room
        })
        .map(|e| e.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engagement(agent: &str, room: &str, state: EngagementState, id: &str) -> Engagement {
        serde_json::from_value(json!({
            "id": id,
            "requestId": format!("req_{id}"),
            "projectId": "project_one",
            "projectRoomId": room,
            "projectName": null,
            "agentName": agent,
            "runtimeName": format!("rt_{agent}"),
            "resourceId": "resource_27cac5503836765cd10751d2",
            "role": "coding",
            "requestedTokens": 250,
            "state": state,
            "cleanup": "not_required"
        }))
        .unwrap()
    }

    /// Retained tests/api-engagement-room-admission.test.js:452-469 — the
    /// sweep asks per (agent, room) ONCE, however many engagements share
    /// the pair: six concurrent engagements between one agent and one room
    /// is a real shape in the retained data.
    #[test]
    fn native_sweep_asks_once_per_agent_room_pair() {
        let mut rows: Vec<_> = (0..6)
            .map(|n| {
                engagement(
                    "Provisioned",
                    "!project:example.test",
                    EngagementState::Active,
                    &format!("en_six_{n}"),
                )
            })
            .collect();
        rows.push(engagement(
            "Other",
            "!project:example.test",
            EngagementState::Active,
            "en_other",
        ));
        rows.push(engagement(
            "Provisioned",
            "!project:example.test",
            EngagementState::Revoked,
            "en_ended",
        ));
        let pairs = active_pairs(&rows);
        assert_eq!(
            pairs.len(),
            2,
            "one per live (agent, room), not per engagement: {pairs:?}"
        );
        assert!(pairs.contains(&("Provisioned".to_owned(), "!project:example.test".to_owned())));
        assert!(pairs.contains(&("Other".to_owned(), "!project:example.test".to_owned())));
    }

    /// Retained tests/api-engagement-room-admission.test.js:472-490 — an
    /// agent already in the room is left alone by READING the invite's 403
    /// as the check: the error text that means "already a member" is a
    /// pass, any other 403 is a refusal, and 200 is a real re-invite. The
    /// REFUSAL carries the homeserver's own words (LESSONS.md "Failure
    /// reasons must be recorded"), not a bare word.
    #[test]
    fn native_sweep_reads_the_invite_403_as_the_membership_check() {
        assert_eq!(
            classify_invite(
                403,
                Some(&json!({"errcode":"M_FORBIDDEN","error":"@x is already in the room."}))
            ),
            Invite::AlreadyPresent
        );
        assert_eq!(classify_invite(200, None), Invite::Invited);
        assert_eq!(
            classify_invite(
                403,
                Some(&json!({"errcode":"M_FORBIDDEN","error":"You are not allowed to invite"})),
            ),
            Invite::Failed("M_FORBIDDEN: You are not allowed to invite".to_owned()),
            "the refusal says WHICH homeserver error it was"
        );
        assert_eq!(
            classify_invite(502, None),
            Invite::Failed("invite answered HTTP 502".to_owned())
        );
        assert_eq!(
            classify_invite(500, Some(&json!({"errcode":"M_UNKNOWN"}))),
            Invite::Failed("invite answered HTTP 500 (M_UNKNOWN)".to_owned())
        );
        // A foreign homeserver's words are bounded like every other peer's
        // (`approval_delivery.rs:137-142`), never echoed unbounded.
        let long = classify_invite(
            403,
            Some(&json!({"errcode":"M_FORBIDDEN","error":"é".repeat(400)})),
        );
        match long {
            Invite::Failed(reason) => assert!(
                reason.len() <= 256 && reason.is_char_boundary(reason.len()),
                "bounded at a char boundary: {} bytes",
                reason.len()
            ),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// The TS-VISIBLE outcome, on the wire (retained
    /// tests/api-engagement-room-admission.test.js:449-490): one invite POST
    /// per live (agent, room), addressed to the composed agent mxid, on the
    /// collector's own credential — and a 200 read as a REAL re-admission
    /// rather than the already-present answer. The two tests above pin the
    /// pair dedup and the verdict classification in isolation; this one is
    /// the behaviour an operator's homeserver actually sees.
    #[tokio::test]
    async fn native_sweep_invites_once_per_pair_on_the_wire() {
        use crate::collector::fixtures as common;
        let f = common::Fixture::new();
        let mut fake = common::Fake::start(false).await;
        let c = crate::Collector::new(f.config(&fake.endpoint), f.store.clone()).unwrap();
        let cancel = CancellationToken::new();
        let fleet = common::domain::registration().fleet_id;
        let engagement = f.identity.transport.engagement_id.clone();
        let (outcome, ()) = tokio::join!(c.sweep_project_room_membership(&cancel), async {
            let request = fake.next().await;
            assert_eq!(request.method, "POST");
            // ONE path segment, the same spelling every other ported room
            // call sends (retire.rs's leave asserts the identical shape).
            assert_eq!(
                request.target,
                "/_matrix/client/v3/rooms/!project:example.test/invite"
            );
            assert_eq!(
                request.headers["authorization"],
                format!("Bearer {}", common::TOKEN)
            );
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(
                body["user_id"],
                format!("@{fleet}_{engagement}:example.test"),
                "the composed provisioned-account identity"
            );
            request.json(200, json!({}));
        });
        assert_eq!(
            outcome.invited, 1,
            "a 200 is a real re-admission, not the already-present answer: {outcome:?}"
        );
        assert_eq!(
            outcome.pairs, 1,
            "one live pair, one call — never per engagement"
        );
        assert_eq!(outcome.failed, 0);
        fake.close().await;
    }

    /// ADR-187 §A.5: an imported fleet's sweep invites with the
    /// representative's credential, not a coordinator's.
    #[tokio::test]
    async fn native_fleet_sweep_acts_with_the_representative_credential() {
        use crate::collector::fixtures as common;
        let f = common::Fixture::new();
        let mut fake = common::Fake::start(false).await;
        let endpoint = reqwest::Url::parse(&fake.endpoint).unwrap();
        let authorization =
            reqwest::header::HeaderValue::from_static("Bearer representative-token-0123456789");
        let sweep = MembershipSweep {
            http: crate::http::Http::for_host(
                &endpoint,
                Some(&authorization),
                &crate::Limits::default(),
                &[
                    reqwest::Certificate::from_pem(include_bytes!("../tests/fixtures/ca.pem"))
                        .unwrap(),
                ],
            )
            .unwrap(),
            domain: f.store.clone(),
            registration: common::domain::registration(),
        };
        let cancel = CancellationToken::new();
        let (outcome, ()) = tokio::join!(sweep.pass(&cancel), async {
            let request = fake.next().await;
            assert_eq!(request.method, "POST");
            assert_eq!(
                request.headers["authorization"],
                "Bearer representative-token-0123456789"
            );
            request.json(200, json!({}));
        });
        assert_eq!(outcome.invited, 1, "{outcome:?}");
        fake.close().await;
    }
}
