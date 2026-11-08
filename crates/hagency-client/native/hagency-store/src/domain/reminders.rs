//! Agent-scheduled self-reminders (board #53, TS `lib/delivery-queue.js:1725-1760`).
//!
//! An agent schedules a reminder against its OWN running session; when it comes
//! due the service wakes that session with the retained `[Self Time Reminder] …`
//! text, exactly as the TS delivery queue enqueued it (`renderReminderPayload`).
//! The wake rides the ordinary message-ingress path — a host-synthetic
//! `InboundMessage` admitted with `wake=1` and a `config` carrying `origin_ts`,
//! exactly the `session_inputs` shape `select_agent` reads — so the continuous
//! agent inbox mints the dispatch that delivers the reminder back to the agent.
//! `fired_at` is set once, in the same transaction as the wake, so a restart
//! never re-fires a delivered reminder and never loses one un-fired.
use super::{DomainRepository, bounded_row, execution, matrix_routes, serialize};
use crate::Error;
use hagency_core::{
    JSON_SAFE_MAX, canonical,
    messages::{InboundMessage, Message},
    tasks::{clock, text},
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

/// One reminder row, as the console and the list route render it. `id` is the
/// TS `reminderIdCounter` integer the DELETE route addresses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reminder {
    pub id: i64,
    pub engagement_id: String,
    pub session_id: String,
    pub msg: String,
    pub created_at: u64,
    pub fire_at: u64,
    pub fired_at: Option<u64>,
}
/// The create route's answer, mirroring the TS `{ok, id, fireAt, remainingMs}`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReminderReceipt {
    pub id: i64,
    pub fire_at: u64,
    pub remaining_ms: u64,
}
/// The fire sweep's one-transaction outcome: how many due reminders were woken.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReminderSweep {
    pub fired: u64,
    pub remaining: u64,
}
/// TS `formatRelativeTime` (delivery-queue.js:1324-1331): `Ns ago`, `NmNs ago`,
/// or `NhNm ago`. The exact text is part of the wake the agent reads.
fn format_relative_time(ms: u64) -> String {
    let sec = ms / 1000;
    if sec < 60 {
        return format!("{sec}s ago");
    }
    let min = sec / 60;
    if min < 60 {
        return format!("{min}m{}s ago", sec % 60);
    }
    let hr = min / 60;
    format!("{hr}h{}m ago", min % 60)
}
fn reminder_body(created_at: u64, fire_at: u64, msg: &str) -> String {
    format!(
        "[Self Time Reminder] From ts:{created_at} ({}), Now ts:{fire_at}, Msg: {msg}",
        format_relative_time(fire_at.saturating_sub(created_at))
    )
}
impl DomainRepository {
    pub fn schedule_reminder(
        &mut self,
        cap: &hagency_core::tasks::RunnerCapability,
        msg: &str,
        delay_ms: u64,
        now: u64,
    ) -> Result<ReminderReceipt, Error> {
        text(msg, 32 * 1024)?;
        clock(now)?;
        if delay_ms == 0 || delay_ms > JSON_SAFE_MAX {
            return Err(hagency_core::InvalidInput(
                "delay must be a positive number of milliseconds",
            )
            .into());
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let d = execution::authorize_work(&tx, cap, now)?;
        let engagement_id: String = tx.query_row(
            "SELECT engagement_id FROM runner_sessions WHERE id=?1",
            [&d.session_id],
            |r| r.get(0),
        )?;
        bounded_row(&tx, "reminders", "id", "", 100_000)?;
        let fire_at = now
            .checked_add(delay_ms)
            .ok_or(hagency_core::InvalidInput("reminder fire time overflow"))?;
        tx.execute(
            "INSERT INTO reminders(engagement_id,session_id,msg,created_at,fire_at) VALUES(?1,?2,?3,?4,?5)",
            params![engagement_id, d.session_id, msg, now, fire_at],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(ReminderReceipt {
            id,
            fire_at,
            remaining_ms: delay_ms,
        })
    }
    pub fn list_reminders(&self) -> Result<Vec<Reminder>, Error> {
        let mut query = self.db.prepare(
            "SELECT id,engagement_id,session_id,msg,created_at,fire_at,fired_at FROM reminders ORDER BY id",
        )?;
        query
            .query_map([], |r| {
                Ok(Reminder {
                    id: r.get(0)?,
                    engagement_id: r.get(1)?,
                    session_id: r.get(2)?,
                    msg: r.get(3)?,
                    created_at: r.get(4)?,
                    fire_at: r.get(5)?,
                    fired_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub fn delete_reminder(&mut self, id: i64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute("DELETE FROM reminders WHERE id=?1", [id])? == 0 {
            return Err(Error::NotFound);
        }
        tx.commit()?;
        Ok(())
    }
    /// Fire every due reminder: build its wake self-message, admit it to its own
    /// session with `wake=1` (with a `config` carrying `origin_ts`, the exact
    /// `session_inputs` shape `select_agent` reads), and set `fired_at` — all in
    /// one transaction. A reminder whose session no longer has a live route is
    /// left un-fired (the sweep reports `remaining`); it is not an error,
    /// matching the retained bridge's "one failing agent is skipped, never fatal".
    pub fn fire_reminders(&mut self, now: u64, limit: u64) -> Result<ReminderSweep, Error> {
        clock(now)?;
        let limit = limit.clamp(1, 512);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let due: Vec<(i64, String, String, u64)> = {
            let mut statement = tx.prepare(
                "SELECT id,session_id,msg,created_at FROM reminders WHERE fire_at<=?1 AND fired_at IS NULL ORDER BY id LIMIT ?2",
            )?;
            statement
                .query_map(params![now, limit], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut fired = 0u64;
        for (id, session_id, msg, created_at) in due {
            let binding = match execution::matrix_admission_session(&tx, &session_id) {
                Ok(binding) => binding,
                Err(_) => continue,
            };
            let route = match matrix_routes::route(&tx, &session_id) {
                Ok(route) => route,
                Err(_) => continue,
            };
            let input = InboundMessage {
                server_name: route.server_name.clone(),
                room_id: binding.room_id.clone(),
                event_id: format!("$reminder_{id}"),
                sender_mxid: route.sender_mxid.clone(),
                thread_root: binding.thread_root.clone(),
                body: reminder_body(created_at, now, &msg),
                kind: "m.text".into(),
                origin_ts: now,
            };
            input.validate()?;
            let sequence = admit_self_message(&tx, &input, &session_id, now)?;
            tx.execute(
                "UPDATE reminders SET fired_at=?1 WHERE id=?2",
                params![now, id],
            )?;
            fired += 1;
            // `sequence` is unused beyond admission; its liveness keeps the
            // admitted row selectable by `select_agent`.
            let _ = sequence;
        }
        let remaining: u64 = tx.query_row(
            "SELECT COUNT(*) FROM reminders WHERE fired_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(ReminderSweep { fired, remaining })
    }
}
/// Admit one host-synthetic self-message into the ordinary corpus and project
/// it onto the session with `wake=1` and a `config` carrying `origin_ts`. This
/// is the `verified_ingress` shape (config IS set), not `ingest_message`'s
/// (which is the non-Matrix adapter path and sets no config, so `select_agent`
/// would never match its rows).
fn admit_self_message(
    tx: &Transaction<'_>,
    input: &InboundMessage,
    session_id: &str,
    now: u64,
) -> Result<u64, Error> {
    let source_key = input.source_key()?;
    let digest = canonical::digest(&serde_json::to_value(input)?)?;
    let previous: Option<(u64, String)> = tx
        .query_row(
            "SELECT sequence,digest FROM admitted_messages WHERE source_key=?1",
            [&source_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let sequence = if let Some((sequence, old)) = previous {
        if old != digest {
            return Err(Error::Conflict);
        }
        sequence
    } else {
        bounded_row(tx, "admitted_messages", "source_key", &source_key, 100_000)?;
        tx.execute(
            "INSERT INTO admitted_messages(source_key,digest,config) VALUES(?1,?2,'{}')",
            params![source_key, digest],
        )?;
        let sequence = u64::try_from(tx.last_insert_rowid()).map_err(|_| Error::Capacity)?;
        clock(sequence)?;
        let value = Message {
            sequence,
            source_key,
            server_name: input.server_name.clone(),
            room_id: input.room_id.clone(),
            event_id: input.event_id.clone(),
            sender_mxid: input.sender_mxid.clone(),
            thread_root: input.thread_root.clone(),
            body: input.body.clone(),
            kind: input.kind.clone(),
            origin_ts: input.origin_ts,
            received_at: now,
        };
        tx.execute(
            "UPDATE admitted_messages SET config=?2 WHERE sequence=?1",
            params![sequence, serialize(&value)?],
        )?;
        sequence
    };
    let encoded: String = tx.query_row(
        "SELECT config FROM admitted_messages WHERE sequence=?1",
        [sequence],
        |r| r.get(0),
    )?;
    let pending: u64 = tx.query_row(
        "SELECT COUNT(*) FROM session_inputs WHERE session_id=?1 AND processed_at IS NULL",
        [session_id],
        |r| r.get(0),
    )?;
    if pending >= 2000 {
        return Err(Error::Capacity);
    }
    tx.execute(
        "INSERT INTO session_inputs(session_id,message_sequence,wake,config) VALUES(?1,?2,1,?3)",
        params![session_id, sequence, encoded],
    )?;
    Ok(sequence)
}
