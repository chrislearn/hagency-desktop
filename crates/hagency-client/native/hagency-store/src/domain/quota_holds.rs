//! ADR-186 §B/§C: a used-up allocation pauses the agent; a top-up resumes it.
//!
//! The hold is a row, never an engagement state: the engagement stays
//! `active`, its running turn finishes, queued work stays queued, and the
//! claim simply does not pick a dispatch whose engagement holds one open
//! (`execution.rs` candidate predicate). Nothing is dropped, refused or
//! marked done (ADR-183: the bridge never decides done). The hold opens only
//! on KNOWN spend — host-attributed usage that reached the allocation — and
//! lifts the moment the allocation is above the spend again (a top-up) or the
//! spend stops being known. That makes host-attributed usage the trigger for
//! this one pause (§B5), and for nothing else: it never revokes, ends or
//! refuses an engagement.
use super::{read_engagement, task_intents};
use crate::Error;
use hagency_core::project::EngagementState;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// The notice kind the project room sees when the hold begins (§B3).
pub(super) const QUOTA_PAUSED_KIND: &str = "quota_paused";
/// The notice kind said when a top-up lifts the hold (§C3).
pub(super) const QUOTA_RESUMED_KIND: &str = "quota_resumed";

/// §B3's words, exactly.
pub(super) fn paused_body(spent: u64, allocation: u64) -> String {
    format!(
        "Paused: used {spent} of {allocation} tokens. The owner can add tokens in the Hagency console."
    )
}
/// §C3's words, exactly.
pub(super) fn resumed_body(available: u64) -> String {
    format!("Resumed: {available} tokens available.")
}

/// §B1/§B4: the engagement's spend — input + output + cache writes, summed
/// over every MONTHLY period it was observed in. Every observation credits
/// both its daily and its monthly bucket (`usage.rs` `credit_period`), so the
/// monthly buckets alone partition the engagement's lifetime and nothing is
/// counted twice. Cache reads are not counted, as with ceilings.
///
/// The figure is the ledger's known-growth LOWER BOUND, so a pause read from
/// it is never a false one: the true spend is at least this much. `None` is
/// unknown — no observed period at all, or periods that carry no known count
/// (an observation without counts). An incomplete period that does carry
/// counts still counts: every runtime observation the host records is marked
/// incomplete by construction (`runtime_usage::normalize` sets
/// `stream_incomplete`), so reading "incomplete" as "unknown" would mean no
/// live agent could ever pause.
pub(super) fn spend(db: &Connection, engagement: &str) -> Result<Option<u64>, Error> {
    let mut statement = db.prepare(
        "SELECT known_growth,incomplete FROM usage_periods \
         WHERE engagement_id=?1 AND granularity='monthly'",
    )?;
    let rows = statement.query_map([engagement], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
    })?;
    let mut total = 0u64;
    let mut complete = false;
    for row in rows {
        let (growth, incomplete) = row?;
        let growth: super::KnownTokens = serde_json::from_str(&growth)?;
        total = total
            .checked_add(growth.observed_fresh_lower_bound()?)
            .ok_or(hagency_core::InvalidInput("usage sum overflow"))?;
        complete |= !incomplete;
    }
    Ok((complete || total > 0).then_some(total))
}

/// Whether the engagement holds an open quota hold.
pub(super) fn paused(db: &Connection, engagement: &str) -> Result<bool, Error> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM quota_holds WHERE engagement_id=?1 AND lifted_at IS NULL)",
        [engagement],
        |r| r.get(0),
    )?)
}

/// §B2/§B3, after usage was credited: open the hold when the known spend has
/// reached the allocation, and say so once in the thread of `dispatch` — the
/// turn whose observation crossed the line, which therefore runs in the
/// project room. The notice is best effort in its own savepoint: a notice
/// that cannot be addressed never undoes the hold, the same rule as the
/// claim's skip notices.
pub(super) fn evaluate(
    tx: &Transaction<'_>,
    engagement: &str,
    dispatch: Option<&str>,
    now: u64,
) -> Result<(), Error> {
    let value = read_engagement(tx, engagement)?;
    if !matches!(
        value.state,
        EngagementState::Reserved | EngagementState::Active
    ) || paused(tx, engagement)?
    {
        return Ok(());
    }
    let Some(spent) = spend(tx, engagement)? else {
        return Ok(()); // §B4: unknown usage never pauses.
    };
    let allocation = u64::from(value.allocation());
    if spent < allocation {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO quota_holds(engagement_id,dispatch_id,spend,allocation,began_at) VALUES(?1,?2,?3,?4,?5)",
        params![engagement, dispatch, spent, allocation, now],
    )?;
    let hold = tx.last_insert_rowid();
    if let Some(dispatch) = dispatch {
        say(
            tx,
            dispatch,
            QUOTA_PAUSED_KIND,
            &format!("{QUOTA_PAUSED_KIND}:{hold}"),
            &paused_body(spent, allocation),
            now,
        )?;
    }
    Ok(())
}

/// §C3, after the allocation was raised: lift the open hold when the new
/// allocation is above the spend (or the spend is no longer known, which can
/// never hold an agent), and say "Resumed" once — in the thread of the
/// oldest queued dispatch, which is the work that now runs, or else in the
/// thread the pause was said in. Returns whether a hold was lifted.
pub(super) fn lift(tx: &Transaction<'_>, engagement: &str, now: u64) -> Result<bool, Error> {
    let open: Option<(i64, Option<String>)> = tx
        .query_row(
            "SELECT id,dispatch_id FROM quota_holds WHERE engagement_id=?1 AND lifted_at IS NULL",
            [engagement],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((hold, paused_in)) = open else {
        return Ok(false);
    };
    let allocation = u64::from(read_engagement(tx, engagement)?.allocation());
    let spent = spend(tx, engagement)?;
    if spent.is_some_and(|spent| spent >= allocation) {
        return Ok(false);
    }
    tx.execute(
        "UPDATE quota_holds SET lifted_at=?2,lifted_allocation=?3 WHERE id=?1",
        params![hold, now, allocation],
    )?;
    let queued: Option<String> = tx
        .query_row(
            "SELECT d.id FROM runner_dispatches d JOIN runner_sessions s ON s.id=d.session_id \
             WHERE s.engagement_id=?1 AND d.state='queued' AND d.task_id IS NOT NULL \
             ORDER BY d.rowid LIMIT 1",
            [engagement],
            |r| r.get(0),
        )
        .optional()?;
    // The first thread that can carry it says it: a queued dispatch with no
    // admitted request in a Matrix session has nowhere to say anything.
    let available = allocation.saturating_sub(spent.unwrap_or(0));
    for dispatch in [queued, paused_in].into_iter().flatten() {
        let said = say(
            tx,
            &dispatch,
            QUOTA_RESUMED_KIND,
            &format!("{QUOTA_RESUMED_KIND}:{hold}"),
            &resumed_body(available),
            now,
        )?;
        if said {
            break;
        }
    }
    Ok(true)
}

/// Say one quota notice in `dispatch`'s thread, best effort in its own
/// savepoint. Returns whether a notice row now exists for it.
fn say(
    tx: &Transaction<'_>,
    dispatch: &str,
    kind: &str,
    key: &str,
    body: &str,
    now: u64,
) -> Result<bool, Error> {
    let count = |tx: &Transaction<'_>| -> Result<u64, Error> {
        Ok(tx.query_row("SELECT COUNT(*) FROM task_notices", [], |r| r.get(0))?)
    };
    let before = count(tx)?;
    tx.execute_batch("SAVEPOINT quota_notice")?;
    match task_intents::keyed_dispatch_notice(tx, dispatch, kind, key, body, now) {
        Ok(()) => tx.execute_batch("RELEASE quota_notice")?,
        Err(_) => tx.execute_batch("ROLLBACK TO quota_notice; RELEASE quota_notice")?,
    }
    Ok(count(tx)? > before)
}
