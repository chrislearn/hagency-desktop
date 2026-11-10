//! Pending agent invitations (task #12): the retained
//! `bridge-matrix.js` `state.pendingInvites` family
//! (`rememberPendingInvite`/`settlePendingInvite`/`listPendingInvites`,
//! `:2381-2455`) behind the backend routes `backend-v2.js:10813-10855`.
//!
//! An untrusted or unreadable inviter makes the invitation a PENDING
//! DECISION, never a join and never a log line (ADR-014's 2026-08-11
//! amendment): lending an agent spends the contributor's tokens, so the
//! decision waits for a human at the console.
//!
//! `inviter` is NULL when the invite state names no sender — surfaced,
//! never guessed, because the inviter IS the owner (ADR-002) and inventing
//! one would forge ownership. `declined` is REMEMBERED rather than deleted
//! so the invite poll cannot resurrect it: the invitation is still in
//! Matrix state and would be seen again next round. `accepted` records
//! that the join has happened (including the stale-record reconcile,
//! attributed to the policy `trusted-inviter`, never to a person).
use crate::Error;
use rusqlite::OptionalExtension;
use serde::Serialize;

/// One pending-invitation row, camelCase on the wire exactly as the TS
/// backend renders it (`lib/pending-invite-store.js:142-149`): exactly
/// `projectRoomId`, `agent`, `inviter` (nullable), `projectServer`,
/// `state`, `seenAt`, `decidedAt`, `decidedBy` — no more keys, none
/// renamed differently.
#[derive(Debug, Clone, Serialize)]
pub struct PendingInvite {
    #[serde(rename = "projectRoomId")]
    pub room_id: String,
    #[serde(rename = "agent")]
    pub agent_name: String,
    pub inviter: Option<String>,
    #[serde(rename = "projectServer")]
    pub project_server: String,
    pub state: &'static str,
    #[serde(rename = "seenAt")]
    pub seen_at: i64,
    #[serde(rename = "decidedAt")]
    pub decided_at: Option<i64>,
    #[serde(rename = "decidedBy")]
    pub decided_by: Option<String>,
}

/// TS `projectServerFromRoomId`: the server part of a room id, or the
/// whole id when it carries none — the same derivation the store applies
/// when recording.
fn project_server_from_room_id(room_id: &str) -> String {
    room_id
        .rsplit_once(':')
        .map(|(_, server)| server.to_owned())
        .unwrap_or_else(|| room_id.to_owned())
}

fn row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingInvite> {
    Ok(PendingInvite {
        room_id: row.get(0)?,
        agent_name: row.get(1)?,
        inviter: row.get(2)?,
        project_server: row.get(3)?,
        seen_at: row.get(4)?,
        decided_at: row.get(5)?,
        decided_by: row.get(6)?,
        state: match row.get::<_, String>(7)?.as_str() {
            "accepted" => "accepted",
            "declined" => "declined",
            _ => "pending",
        },
    })
}

const SELECT_COLUMNS: &str =
    "room_id,agent,inviter,project_server,seen_at,decided_at,decided_by,state";

impl crate::DomainRepository {
    /// TS `rememberPendingInvite` (`bridge-matrix.js:2391-2417`): insert a
    /// pending record; returns FALSE when one already exists in state
    /// `pending` or `declined`, so a poll every few seconds does not
    /// re-notify and cannot resurrect a decline. The mode/since pair is
    /// the direct-room binding the TS DM path records
    /// (`backend-v2.js:16287`: `mode` in {direct, group}, `sinceTs` the
    /// admission boundary).
    pub fn remember_pending_invite(
        &mut self,
        room_id: &str,
        agent: &str,
        inviter: Option<&str>,
        mode: &str,
        since_ts: i64,
        now_ms: i64,
    ) -> Result<bool, Error> {
        validate_invite_key(room_id, agent)?;
        let mode = match mode {
            "direct" => "direct",
            "group" => "group",
            _ => return Err(hagency_core::InvalidInput("invalid room mode").into()),
        };
        if since_ts < 0 {
            return Err(hagency_core::InvalidInput("invalid history boundary").into());
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // The settle guard in one statement: a row in pending or declined
        // answers "already known", exactly the TS early return.
        let known: Option<String> = tx
            .query_row(
                "SELECT state FROM pending_invites WHERE room_id=?1 AND agent=?2",
                [room_id, agent],
                |r| r.get(0),
            )
            .optional()?;
        if matches!(known.as_deref(), Some("pending" | "declined")) {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO pending_invites \
             (room_id,agent,inviter,mode,since_ts,state,seen_at,project_server) \
             VALUES(?1,?2,?3,?4,?5,'pending',?6,?7) \
             ON CONFLICT(room_id,agent) DO UPDATE SET \
             inviter=excluded.inviter,mode=excluded.mode,since_ts=excluded.since_ts, \
             state='pending',seen_at=excluded.seen_at,decided_at=NULL,decided_by=NULL",
            rusqlite::params![
                room_id,
                agent,
                inviter,
                mode,
                since_ts,
                now_ms,
                project_server_from_room_id(room_id)
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// TS `backfillPendingInviteInviter` (`:2381-2389`): fills a NULL
    /// inviter on a still-pending record only — the poll that could not
    /// read the invite state records `inviter: null`, and the next poll
    /// resolves the sender; it never overwrites a known inviter and never
    /// touches a settled record.
    pub fn backfill_pending_invite_inviter(
        &mut self,
        room_id: &str,
        agent: &str,
        inviter: &str,
    ) -> Result<bool, Error> {
        validate_invite_key(room_id, agent)?;
        if inviter.is_empty() || inviter.len() > 255 {
            return Err(hagency_core::InvalidInput("invalid inviter").into());
        }
        let changed = self.db.execute(
            "UPDATE pending_invites SET inviter=?3 \
             WHERE room_id=?1 AND agent=?2 AND state='pending' AND inviter IS NULL",
            rusqlite::params![room_id, agent, inviter],
        )?;
        Ok(changed == 1)
    }

    /// TS `listPendingInvites` (`:2419-2428`): every invitation still
    /// awaiting a decision, newest `seenAt` first.
    pub fn pending_invites(&self) -> Result<Vec<PendingInvite>, Error> {
        let mut query = self.db.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM pending_invites \
             WHERE state='pending' ORDER BY seen_at DESC, room_id, agent"
        ))?;
        let rows = query.query_map([], row_from)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// TS `getPendingInvite`: the record for one (room, agent), whatever
    /// its state.
    pub fn pending_invite(
        &self,
        room_id: &str,
        agent: &str,
    ) -> Result<Option<PendingInvite>, Error> {
        validate_invite_key(room_id, agent)?;
        self.db
            .query_row(
                &format!(
                    "SELECT {SELECT_COLUMNS} FROM pending_invites \
                     WHERE room_id=?1 AND agent=?2"
                ),
                [room_id, agent],
                row_from,
            )
            .optional()
            .map_err(Error::from)
    }

    /// TS `settlePendingInvite` (`:2438-2447`): mark a decision. Returns
    /// None when no record exists (the TS null). `by` names the decider —
    /// `trusted-inviter` for the auto-join and the stale-record reconcile
    /// (the agent is in the room, so the invitation WAS answered by
    /// policy, with no human at a screen), the operator name for console
    /// decisions. `joined` records whether the join already happened:
    /// an accept that has not joined yet raises `join_pending` so the
    /// invite poll performs the join and retries on refusal (ADR-183);
    /// the auto-join and reconcile paths pass true.
    pub fn settle_pending_invite(
        &mut self,
        room_id: &str,
        agent: &str,
        accepted: bool,
        joined: bool,
        by: &str,
        now_ms: i64,
    ) -> Result<Option<PendingInvite>, Error> {
        validate_invite_key(room_id, agent)?;
        if by.is_empty() || by.len() > 128 {
            return Err(hagency_core::InvalidInput("invalid decider").into());
        }
        let changed = self.db.execute(
            "UPDATE pending_invites SET state=?3,decided_at=?4,decided_by=?5,join_pending=?6,leave_pending=?7 \
             WHERE room_id=?1 AND agent=?2",
            rusqlite::params![
                room_id,
                agent,
                if accepted { "accepted" } else { "declined" },
                now_ms,
                by,
                if accepted && !joined { 1 } else { 0 },
                if accepted { 0 } else { 1 }
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.pending_invite(room_id, agent)
    }

    /// Every accepted invitation whose join is still owed: the invite
    /// poll's worklist. A join refused by the homeserver stays here and
    /// is retried next round — never terminal (ADR-183).
    pub fn join_pending_invites(&self, agent: &str) -> Result<Vec<(String, String)>, Error> {
        if agent.is_empty() || agent.len() > 255 {
            return Err(hagency_core::InvalidInput("invalid agent").into());
        }
        let mut query = self.db.prepare(
            "SELECT room_id,agent FROM pending_invites \
             WHERE agent=?1 AND state='accepted' AND join_pending=1 \
             ORDER BY seen_at, room_id",
        )?;
        let rows = query.query_map([agent], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// The join happened: clear the owed flag. Called only after the
    /// homeserver answered the join with a room id.
    pub fn mark_invite_joined(&mut self, room_id: &str, agent: &str) -> Result<bool, Error> {
        validate_invite_key(room_id, agent)?;
        let changed = self.db.execute(
            "UPDATE pending_invites SET join_pending=0 WHERE room_id=?1 AND agent=?2",
            [room_id, agent],
        )?;
        Ok(changed == 1)
    }

    /// The leave happened (best-effort, `bridge-matrix.js:9135-9147`):
    /// clear the owed flag so the poll stops retrying it.
    pub fn mark_invite_left(&mut self, room_id: &str, agent: &str) -> Result<bool, Error> {
        validate_invite_key(room_id, agent)?;
        let changed = self.db.execute(
            "UPDATE pending_invites SET leave_pending=0 WHERE room_id=?1 AND agent=?2",
            [room_id, agent],
        )?;
        Ok(changed == 1)
    }

    /// Every declined invitation whose leave is still owed, oldest first.
    pub fn leave_pending_invites(&self, agent: &str) -> Result<Vec<(String, String)>, Error> {
        if agent.is_empty() || agent.len() > 255 {
            return Err(hagency_core::InvalidInput("invalid agent").into());
        }
        let mut query = self.db.prepare(
            "SELECT room_id,agent FROM pending_invites \
             WHERE agent=?1 AND state='declined' AND leave_pending=1 \
             ORDER BY seen_at, room_id",
        )?;
        let rows = query.query_map([agent], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// The project owner recorded for a room, when the room is one: the
    /// store-held equivalent of TS's trusted-inviter set
    /// (`MATRIX_TRUSTED_INVITER_MXIDS`, `bridge-matrix.js:2841-2851`) —
    /// provisioning wrote exactly this owner, so an invitation from them
    /// is the trusted-inviter arm; anyone else is a pending decision.
    pub fn room_owner(&self, room_id: &str) -> Result<Option<String>, Error> {
        if room_id.is_empty() || room_id.len() > 256 {
            return Err(hagency_core::InvalidInput("invalid room id").into());
        }
        self.db
            .query_row(
                "SELECT owner_mxid FROM projects WHERE room_id=?1 ORDER BY generation DESC LIMIT 1",
                [room_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    /// The agent name an engagement's collector syncs under — the invite
    /// record's `agent` key (TS keys records by agent NAME,
    /// `bridge-matrix.js:2391`).
    pub fn engagement_agent(&self, engagement_id: &str) -> Result<Option<String>, Error> {
        if engagement_id.is_empty() || engagement_id.len() > 128 {
            return Err(hagency_core::InvalidInput("invalid engagement").into());
        }
        self.db
            .query_row(
                "SELECT name FROM engagements WHERE id=?1",
                [engagement_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    /// The settled mode/since pair a join consumes: the DM binding facts
    /// the accept path needs (`mode` decides the reply-scope guard, TS
    /// `matrix-direct-chat.js:279-292`).
    pub fn pending_invite_binding(
        &self,
        room_id: &str,
        agent: &str,
    ) -> Result<Option<(String, i64)>, Error> {
        validate_invite_key(room_id, agent)?;
        self.db
            .query_row(
                "SELECT mode,since_ts FROM pending_invites WHERE room_id=?1 AND agent=?2",
                [room_id, agent],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(Error::from)
    }
}

fn validate_invite_key(room_id: &str, agent: &str) -> Result<(), Error> {
    if room_id.is_empty() || room_id.len() > 256 {
        return Err(hagency_core::InvalidInput("invalid room id").into());
    }
    if agent.is_empty() || agent.len() > 255 {
        return Err(hagency_core::InvalidInput("invalid agent").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, crate::DomainRepository) {
        let root = tempfile::tempdir().unwrap();
        let db = crate::DomainRepository::open(&root.path().join("domain")).unwrap();
        (root, db)
    }

    /// TS `rememberPendingInvite`: the first sight inserts pending; a
    /// second sight while pending does not re-notify (false); a decline is
    /// remembered and cannot be resurrected by another poll.
    #[test]
    fn remember_does_not_renotify_or_resurrect_a_decline() {
        let (_root, mut db) = open();
        assert!(
            db.remember_pending_invite(
                "!room:a.test",
                "Worker",
                Some("@owner:a.test"),
                "group",
                7,
                100
            )
            .unwrap()
        );
        assert!(
            !db.remember_pending_invite(
                "!room:a.test",
                "Worker",
                Some("@other:a.test"),
                "group",
                8,
                200
            )
            .unwrap()
        );
        let list = db.pending_invites().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].room_id, "!room:a.test");
        assert_eq!(list[0].agent_name, "Worker");
        assert_eq!(list[0].inviter.as_deref(), Some("@owner:a.test"));
        assert_eq!(list[0].project_server, "a.test");
        assert_eq!(list[0].state, "pending");
        assert_eq!(list[0].seen_at, 100);
        db.settle_pending_invite("!room:a.test", "Worker", false, true, "operator", 300)
            .unwrap();
        // The poll sees the invitation again — it must not resurrect it.
        assert!(
            !db.remember_pending_invite("!room:a.test", "Worker", None, "group", 9, 400)
                .unwrap()
        );
        let record = db
            .pending_invite("!room:a.test", "Worker")
            .unwrap()
            .expect("decline is remembered, not deleted");
        assert_eq!(record.state, "declined");
        assert_eq!(record.decided_by.as_deref(), Some("operator"));
    }

    /// A NULL inviter is recorded as NULL (surfaced, never guessed) and a
    /// later poll that can read the sender fills it — pending rows only.
    #[test]
    fn inviter_backfill_fills_only_a_pending_null() {
        let (_root, mut db) = open();
        assert!(
            db.remember_pending_invite("!room:a.test", "Worker", None, "direct", 5, 100)
                .unwrap()
        );
        assert!(
            db.backfill_pending_invite_inviter("!room:a.test", "Worker", "@owner:a.test")
                .unwrap()
        );
        assert!(
            db.pending_invite("!room:a.test", "Worker")
                .unwrap()
                .unwrap()
                .inviter
                .is_some()
        );
        // A second backfill changes nothing; a settled record is untouched.
        assert!(
            !db.backfill_pending_invite_inviter("!room:a.test", "Worker", "@other:a.test")
                .unwrap()
        );
        db.settle_pending_invite("!room:a.test", "Worker", true, true, "trusted-inviter", 200)
            .unwrap();
        assert!(
            !db.backfill_pending_invite_inviter("!room:a.test", "Worker", "@third:a.test")
                .unwrap()
        );
    }

    /// `listPendingInvites` is pending-only, newest first — the TS render
    /// order the /projects page shows.
    #[test]
    fn list_is_pending_only_newest_first() {
        let (_root, mut db) = open();
        db.remember_pending_invite("!old:a.test", "A", None, "group", 1, 100)
            .unwrap();
        db.remember_pending_invite("!new:a.test", "B", None, "group", 2, 300)
            .unwrap();
        db.remember_pending_invite("!mid:a.test", "C", None, "group", 3, 200)
            .unwrap();
        db.settle_pending_invite("!mid:a.test", "C", true, true, "trusted-inviter", 250)
            .unwrap();
        let list = db.pending_invites().unwrap();
        let order: Vec<&str> = list.iter().map(|r| r.room_id.as_str()).collect();
        assert_eq!(order, ["!new:a.test", "!old:a.test"]);
    }

    /// The settle the stale-reconcile performs: `accepted` attributed to
    /// the policy, and the binding facts survive for the join.
    #[test]
    fn settle_records_the_decider_and_keeps_the_binding() {
        let (_root, mut db) = open();
        db.remember_pending_invite(
            "!room:a.test",
            "Worker",
            Some("@owner:a.test"),
            "direct",
            42,
            100,
        )
        .unwrap();
        let settled = db
            .settle_pending_invite("!room:a.test", "Worker", true, true, "trusted-inviter", 500)
            .unwrap()
            .expect("record exists");
        assert_eq!(settled.state, "accepted");
        assert_eq!(settled.decided_at, Some(500));
        assert_eq!(settled.decided_by.as_deref(), Some("trusted-inviter"));
        let (mode, since) = db
            .pending_invite_binding("!room:a.test", "Worker")
            .unwrap()
            .expect("binding survives the decision");
        assert_eq!(mode, "direct");
        assert_eq!(since, 42);
        // Settling an unknown invitation answers None, the TS null.
        assert!(
            db.settle_pending_invite("!none:a.test", "Worker", true, false, "operator", 600)
                .unwrap()
                .is_none()
        );
    }

    /// The console decide / poll join split: an operator accept that has
    /// not joined yet sits on the worklist; the poll joins and clears it;
    /// a join the homeserver refused stays listed and is retried next
    /// round — never terminal (ADR-183). The TS bridge answers the join
    /// inline in the decide, where a failure overstates the decision; the
    /// worklist is the native shape of "queued", which is what the TS
    /// decide route's response already says (`backend-v2.js:10844-10852`).
    #[test]
    fn console_accept_queues_the_join_and_the_poll_clears_it() {
        let (_root, mut db) = open();
        db.remember_pending_invite(
            "!one:a.test",
            "Worker",
            Some("@owner:a.test"),
            "group",
            1,
            100,
        )
        .unwrap();
        db.remember_pending_invite(
            "!two:a.test",
            "Worker",
            Some("@owner:a.test"),
            "group",
            2,
            200,
        )
        .unwrap();
        db.settle_pending_invite("!one:a.test", "Worker", true, false, "operator", 300)
            .unwrap();
        db.settle_pending_invite("!two:a.test", "Worker", true, false, "operator", 300)
            .unwrap();
        // Both accepted-not-yet-joined joins are owed, oldest seen first.
        let owed: Vec<(String, String)> = db.join_pending_invites("Worker").unwrap();
        assert_eq!(
            owed.iter()
                .map(|(r, a)| (r.as_str(), a.as_str()))
                .collect::<Vec<_>>(),
            [("!one:a.test", "Worker"), ("!two:a.test", "Worker")]
        );
        // The poll joins one; the homeserver refused the other (still owed).
        assert!(db.mark_invite_joined("!one:a.test", "Worker").unwrap());
        let owed = db.join_pending_invites("Worker").unwrap();
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].0, "!two:a.test");
        // Next round retries the refused join and clears it.
        assert!(db.mark_invite_joined("!two:a.test", "Worker").unwrap());
        assert!(db.join_pending_invites("Worker").unwrap().is_empty());
        // Another agent's owed join is not on this agent's worklist.
        db.remember_pending_invite("!three:a.test", "Other", None, "group", 3, 400)
            .unwrap();
        db.settle_pending_invite("!three:a.test", "Other", true, false, "operator", 500)
            .unwrap();
        assert!(db.join_pending_invites("Worker").unwrap().is_empty());
        assert_eq!(db.join_pending_invites("Other").unwrap().len(), 1);
    }

    /// The TS 400s: an invalid mode or a negative history boundary.
    #[test]
    fn invalid_mode_or_boundary_is_refused() {
        let (_root, mut db) = open();
        assert!(matches!(
            db.remember_pending_invite("!r:a.test", "W", None, "dm", 1, 100),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            db.remember_pending_invite("!r:a.test", "W", None, "group", -1, 100),
            Err(Error::Invalid(_))
        ));
    }
}
