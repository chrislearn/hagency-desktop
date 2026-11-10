//! Per-side allocation and budget (backend-v2.js:9254-9307, 9541-9572) and
//! the fleet usage totals (backend-v2.js:15700-15720).
//!
//! Parity decisions:
//! - `committed` sums `reserved`+`active` engagements — the native commitment
//!   states (`pool_commitments`/`seat_commitments` indexes, domain.rs:532)
//!   where the retained JavaScript summed `active` only; native mints into
//!   `reserved` before activation and both states hold tokens.
//! - The retained `usesResourcePool` split (`requestContext.agentDefinition`
//!   present ⇒ pool, lib/resource-allocation-budget.js:3) has no native
//!   discriminator: every native engagement carries an agent definition. All
//!   rows therefore land in the legacy `commitments` bucket and
//!   `pool_commitments` is empty, which keeps `total_committed` equal to
//!   `committed` exactly as `committedForProjectSide({legacyOnly:true})`
//!   does on a fleet with no resource pools.
//! - The allocation lives in `side_allocations` (061), never in the
//!   registration config: NULL is UNALLOCATED, which is not unlimited
//!   (lib/project-side-store.js:443-452), and zero is a real allocation.
use super::DomainRepository;
use crate::Error;
use hagency_metering::TokenCounts;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideCommitment {
    pub id: String,
    pub agent: String,
    pub role: String,
    pub project: String,
    pub project_name: Option<String>,
    pub allocated_tokens: u64,
    pub agent_exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideBudget {
    /// NULL is unallocated, not unlimited; zero is a real allocation.
    pub allocated: Option<u64>,
    pub committed: u64,
    /// `allocated === null ? null : Math.max(0, allocated - committed)`.
    pub remaining: Option<u64>,
    pub commitments: Vec<SideCommitment>,
    pub pool_commitments: Vec<SideCommitment>,
    pub pool_committed: u64,
    pub total_committed: u64,
    /// Commitments whose agent no longer appears on the roster
    /// (`agentExists` is COMPUTED, never stored — backend-v2.js:9285-9286).
    pub orphaned_committed: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    /// Agents with an engagement row — the fleet denominator.
    pub agents: u64,
    /// Sum of every engagement's known fresh-token high water
    /// (input+output+cacheWrite — CEILING_KINDS, operator ruling 2026-08-12);
    /// NULL when nothing was measured at all: null is "not known", 0 would
    /// claim the fleet consumed nothing (backend-v2.js:15700-15712).
    pub tokens_drawn: Option<u64>,
    /// Display volume over all four kinds for the same rows; NULL likewise.
    pub tokens_used: Option<u64>,
    /// How many of the agents the figure above covers. The numerator never
    /// travels without its denominator.
    pub tokens_measured_for: u64,
    pub tokens_partial: bool,
}

fn u64_to_i64(value: u64) -> Result<i64, Error> {
    i64::try_from(value)
        .map_err(|_| hagency_core::InvalidInput("allocation exceeds storage").into())
}

fn i64_to_u64(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| hagency_core::InvalidInput("negative figure stored").into())
}

impl DomainRepository {
    /// `PUT /api/project-sides/:id/allocation`: set or clear the side's
    /// allocation. `None` clears (unallocated ≠ unlimited); the store owns
    /// the not-found verdict, as `setAllocation` returning null does. The
    /// route id is the SERVER NAME — `projectSideStore.setAllocation`
    /// normalizes it through `serverName()` (lib/project-side-store.js:455)
    /// and the side list serves that same name as `ProjectSide.id` — so the
    /// write resolves it to the registration row before touching the
    /// allocation table, whose key stays the fleet id.
    pub fn set_side_allocation(&mut self, side: &str, allocated: Option<u64>) -> Result<(), Error> {
        let fleet: Option<String> = self
            .db
            .query_row(
                "SELECT fleet_id FROM registrations \
                 WHERE json_extract(config,'$.serverName')=?1 ORDER BY fleet_id LIMIT 1",
                [side],
                |r| r.get(0),
            )
            .optional()?;
        let Some(fleet) = fleet else {
            return Err(Error::NotFound);
        };
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = super::graphs::now_ms()?;
        match allocated {
            None => {
                tx.execute(
                    "INSERT INTO side_allocations(fleet_id,allocated_tokens,updated_at) VALUES(?1,NULL,?2) \
                     ON CONFLICT(fleet_id) DO UPDATE SET allocated_tokens=NULL,updated_at=excluded.updated_at",
                    params![fleet, i64::try_from(now).map_err(|_| hagency_core::InvalidInput("clock"))?],
                )?;
            }
            Some(value) => {
                tx.execute(
                    "INSERT INTO side_allocations(fleet_id,allocated_tokens,updated_at) VALUES(?1,?2,?3) \
                     ON CONFLICT(fleet_id) DO UPDATE SET allocated_tokens=excluded.allocated_tokens,updated_at=excluded.updated_at",
                    params![fleet, u64_to_i64(value)?, i64::try_from(now).map_err(|_| hagency_core::InvalidInput("clock"))?],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// `GET /api/project-sides/:id/budget`: what the side has, has promised,
    /// and has left, with the commitment breakdown. ONE statement per read
    /// inside one transaction, so allocation and commitments cannot tear.
    pub fn side_budget(&self, side: &str) -> Result<SideBudget, Error> {
        // Single connection, sequential reads: allocation and commitments
        // are observed under the same writer queue, so they cannot tear —
        // the same reasoning as `usage_summary`'s per-engagement reads.
        // The route id is the server name (the side list's own `id`), the
        // tables key on the fleet id — resolve once, use everywhere.
        let fleet: Option<String> = self
            .db
            .query_row(
                "SELECT fleet_id FROM registrations \
                 WHERE json_extract(config,'$.serverName')=?1 ORDER BY fleet_id LIMIT 1",
                [side],
                |r| r.get(0),
            )
            .optional()?;
        let Some(fleet) = fleet else {
            return Err(Error::NotFound);
        };
        let allocated: Option<i64> = self
            .db
            .query_row(
                "SELECT allocated_tokens FROM side_allocations WHERE fleet_id=?1",
                [&fleet],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let allocated = allocated.map(i64_to_u64).transpose()?;
        // `agentExists` is COMPUTED, never stored (backend-v2.js:9285-9286):
        // the retained JS checks the agents registry. Native has no separate
        // registry — the roster itself is engagement-keyed (ADR-126) — so
        // every commitment's agent exists by construction and
        // `orphaned_committed` stays 0, exactly the invariant the retained
        // comment promises for a fleet running this code ("a delete no
        // longer leaves one behind"). The field is still served.
        // ADR-186 §A4: a commitment is the granted amount when one is set.
        let mut query = self.db.prepare(
            "SELECT e.id,e.project_id,COALESCE(e.allocated_tokens,e.tokens),e.projection \
             FROM engagements e WHERE e.fleet_id=?1 AND e.state IN ('reserved','active') ORDER BY e.id",
        )?;
        let mut commitments = Vec::new();
        let mut committed = 0u64;
        for row in query.query_map([&fleet], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (id, project, tokens, projection) = row?;
            let tokens = i64_to_u64(tokens)?;
            let projection: serde_json::Value = serde_json::from_str(&projection)?;
            committed = committed
                .checked_add(tokens)
                .ok_or(hagency_core::InvalidInput("commitment sum overflow"))?;
            commitments.push(SideCommitment {
                id,
                agent: projection["agentName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                role: projection["role"].as_str().unwrap_or_default().to_owned(),
                project: project.clone(),
                project_name: projection["projectName"].as_str().map(str::to_owned),
                allocated_tokens: tokens,
                agent_exists: true,
            });
        }
        Ok(SideBudget {
            allocated,
            committed,
            remaining: allocated.map(|value| value.saturating_sub(committed)),
            total_committed: committed,
            commitments,
            pool_commitments: Vec::new(),
            pool_committed: 0,
            orphaned_committed: 0,
        })
    }

    /// The retained `GET /api/usage` `totals` block for the measured figures
    /// native actually holds: per-engagement known high water, summed, with
    /// its denominator. A sum over ALL sources would double-count a
    /// re-observed engagement whose tokens are already committed, so each
    /// engagement contributes one row — the same shape the ceiling read uses
    /// (usage_sources high_water is the known-growth lower bound, never an
    /// allowance).
    pub fn usage_totals(&self) -> Result<UsageTotals, Error> {
        let mut agents = self.db.prepare(
            "SELECT DISTINCT json_extract(projection,'$.agentName') FROM engagements LIMIT 1024",
        )?;
        let agents = agents
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .len() as u64;
        // SQLite's bare-column rule: with MAX() in the SELECT list,
        // s.high_water comes from the source row holding the newest
        // observed_at — one figure per engagement, never a re-observation
        // double-count.
        let mut query = self.db.prepare(
            "SELECT e.id,s.high_water,MAX(s.observed_at) FROM engagements e \
             JOIN usage_sources s ON s.engagement_id=e.id \
             GROUP BY e.id LIMIT 1024",
        )?;
        let mut drawn = 0u64;
        let mut used = 0u64;
        let mut measured = 0u64;
        for row in query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (_, water) = row?;
            let water: TokenCounts = serde_json::from_str(&water)?;
            // NULL kinds are unknown, never zero: only the measured kinds
            // add, matching KnownTokens::adding. A malformed stored figure
            // is data corruption, not a usage answer.
            let engagement_drawn = water
                .ceiling_volume()
                .map_err(|_| hagency_core::InvalidInput("stored usage figure overflows"))?;
            let engagement_used = water
                .display_volume()
                .map_err(|_| hagency_core::InvalidInput("stored usage figure overflows"))?;
            // A source bound but never observed holds all-null kinds: its
            // engagement is NOT measured. The retained filter is
            // `typeof tokensUsed === 'number'` (backend-v2.js:15702) — an
            // unobserved agent neither adds to the sums nor counts in the
            // denominator, so a fleet of nothing-but-bound sources reports
            // null totals ("not known"), never Some(0) ("consumed
            // nothing"). Where one kind IS known the other may still be
            // null and contributes 0 — the `?? 0` of the retained reduce.
            if engagement_drawn.is_none() && engagement_used.is_none() {
                continue;
            }
            drawn = drawn
                .checked_add(engagement_drawn.unwrap_or(0))
                .ok_or(hagency_core::InvalidInput("usage sum overflow"))?;
            used = used
                .checked_add(engagement_used.unwrap_or(0))
                .ok_or(hagency_core::InvalidInput("usage sum overflow"))?;
            measured = measured
                .checked_add(1)
                .ok_or(hagency_core::InvalidInput("usage sum overflow"))?;
        }
        Ok(UsageTotals {
            agents,
            tokens_drawn: (measured > 0).then_some(drawn),
            tokens_used: (measured > 0).then_some(used),
            tokens_measured_for: measured,
            tokens_partial: measured > 0 && measured < agents,
        })
    }
}
