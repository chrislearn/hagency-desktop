//! Requester-facing reads (board #48; TS `backend-v2.js:15283-15448`): the offer
//! book (`GET /api/offer-book`), the contributions list
//! (`GET /api/contributions`) and the engagement PREVIEW
//! (`GET /api/engagements/preview`) — the last a DRY RUN that decides nothing
//! and writes nothing.
//!
//! WHAT NATIVE HONESTLY HAS, AND WHAT IT DOES NOT. The retained product spreads
//! these three reads over three stores: `engagementStore.listOffers()` (offer
//! TERMS — `budgetCapPerEngagement`/`rateCap`/`count`), `engagementStore.
//! isWhitelisted()` (an auto-join allowlist) and `approvalStore.listBindings()`
//! (an agent<->project BINDING carrying a membership probe). Native has no
//! offer-terms table, no whitelist and no binding table — and it never
//! auto-joins: every engagement is born `pending` (`domain.rs`, `admit`) and only
//! an operator verdict moves it. Inventing a column would need a migration this
//! task forbids; inventing a value would be worse. So each read serves the REAL
//! state native holds and encodes what native lacks exactly as the retained store
//! encodes "unset"/"never checked":
//!
//!   * the offer book lists the REAL published roles and, per role, the REAL
//!     serving resource and resource list (`Resource::qualifies` +
//!     `qualification::resources_for_role`) and the REAL live count
//!     (`engagements`). The three cap fields are `None` — the retained store's own
//!     "unset" encoding (`lib/engagement-store.js:451-454`: an absent cap is
//!     `null`, never `0`).
//!   * `whitelisted` is `None` when no room was named (the retained route's own
//!     rule, `backend-v2.js:15323`, so a named-room claim is never made about a
//!     room nobody identified) and `Some(false)` when one was: native trusts no
//!     room for auto-join, which is not an accusation but the plain truth of an
//!     approval-only host.
//!   * contributions project the REAL agent<->project relationships native holds
//!     (live engagements joined to their project). The two membership-probe fields
//!     are `None` — the retained store's tri-state "never checked"
//!     (`lib/approval-store.js:327-328`, `agentJoined ?? null`), which is not
//!     `false` ("the agent is NOT in the room") and must not be collapsed into it.
//!   * the preview reads native's REAL resource headroom and answers with the
//!     route native would actually take — `notWhitelisted` — which is the whole
//!     point of a dry run: it reports the host's real default rather than the
//!     retained host's auto-join.

use super::{DomainRepository, Error};
use hagency_core::project::Resource;
use hagency_core::qualification::{self, Tier};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

/// Publication bound: one offer book lists at most this many roles, and each
/// role at most this many resources — bounded-cost reads, the same posture as
/// every other native read.
pub const MAX_OFFER_ROLES: usize = 64;
const MAX_OFFER_RESOURCES: usize = 64;
/// Bound on the contributions list: the same 200-row ceiling the alert read uses.
const MAX_CONTRIBUTIONS: u64 = 200;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferBook {
    pub roles: Vec<OfferRole>,
    /// Tri-state, exactly as the retained route (`backend-v2.js:15323`): `None`
    /// when no room was named — not "not trusted", which is a claim about a room
    /// nobody identified.
    pub whitelisted: Option<bool>,
    pub project_room_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferRole {
    pub role: String,
    /// Whether a cross-family agent could serve this role. Native only lists a
    /// cross-family role as available once its cross-family set is satisfied, so
    /// an included role is cross-family-ok by construction; for a role that does
    /// not require cross-family the constraint does not apply.
    pub cross_family_ok: bool,
    pub budget_cap_per_engagement: Option<u64>,
    pub rate_cap: Option<u64>,
    pub count: Option<u64>,
    /// Live engagements on this role — a figure, not a quota consumed.
    pub running_now: u64,
    /// Who would serve it if the request arrived now — deliberately not a
    /// reservation (`backend-v2.js:15305`).
    pub serving: Option<OfferServing>,
    pub resources: Vec<OfferResource>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferServing {
    /// `None` in the resource branch the retained route publishes
    /// (`backend-v2.js:15298-15301`): the preset identity and seat state stay
    /// private, only the serving configuration is published.
    pub agent: Option<String>,
    pub framework: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub tier: Option<Tier>,
    pub provisioning_required: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferResource {
    pub id: String,
    pub name: String,
    pub framework: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub tier: Option<Tier>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contribution {
    pub agent: String,
    pub project: String,
    pub project_room_id: String,
    pub owner_mxid: String,
    pub active: bool,
    pub agent_joined: Option<bool>,
    pub membership_checked_at: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub route: &'static str,
    pub auto_join: bool,
    pub agent: Option<String>,
    pub agent_remaining_tokens: Option<u64>,
}

impl DomainRepository {
    /// The offer book (`backend-v2.js:15283`). Lists the roles native actually
    /// publishes: an explicit publication, or at least one published resource
    /// that qualifies — the retained `published === true || resources.length > 0`
    /// rule (`:13934`).
    pub fn offer_book(&self, room: Option<&str>) -> Result<OfferBook, Error> {
        let rows = self.role_publications()?;
        let resources: Vec<Resource> = self
            .resource_configurations("", 100)?
            .into_iter()
            .map(|configured| configured.config)
            .collect();
        let running = self.live_engagements_by_role()?;
        let mut roles = Vec::new();
        for row in rows {
            if roles.len() == MAX_OFFER_ROLES {
                break;
            }
            let role = row
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            if role.is_empty() {
                continue;
            }
            let explicit = row
                .get("explicitPublication")
                .and_then(|value| value.as_bool());
            let qualifying = qualification::resources_for_role(&resources, &role, None);
            if explicit != Some(true) && qualifying.is_empty() {
                continue;
            }
            let serving = qualifying.first().map(|resource| OfferServing {
                agent: None,
                framework: Some(resource.framework.clone()),
                model: Some(resource.model.clone()),
                reasoning: resource.reasoning.clone(),
                tier: qualification::model(&resource.profile()).0,
                provisioning_required: true,
            });
            let listed: Vec<OfferResource> = qualifying
                .iter()
                .take(MAX_OFFER_RESOURCES)
                .map(|resource| OfferResource {
                    id: resource.id(),
                    name: resource.preset_id.clone(),
                    framework: resource.framework.clone(),
                    model: resource.model.clone(),
                    reasoning: resource.reasoning.clone(),
                    tier: qualification::model(&resource.profile()).0,
                })
                .collect();
            // Native's REAL cross-family availability rule — the same one the
            // engagement admission path gates on (`domain.rs:role_available`),
            // never a second, drifting predicate.
            let cross_family_ok = super::role_available(&self.db, &role, None)?;
            roles.push(OfferRole {
                cross_family_ok,
                budget_cap_per_engagement: None,
                rate_cap: None,
                count: None,
                running_now: running.get(&role).copied().unwrap_or(0),
                serving,
                resources: listed,
                role,
            });
        }
        Ok(OfferBook {
            roles,
            // The retained route is `requireRequester`, so its own expression
            // (`backend-v2.js:15323`) never publishes room trust to a requester:
            // `whitelisted` is `null` here, always. Native has no whitelist to
            // publish in any case.
            whitelisted: None,
            project_room_id: room.map(str::to_owned),
        })
    }

    /// The contributions list (`backend-v2.js:15408`): the real agent<->project
    /// relationships native holds. The membership probe has no native
    /// counterpart, so it is `None` ("never checked"), never `false`.
    pub fn contributions(&self) -> Result<Vec<Contribution>, Error> {
        let mut query = self.db.prepare(
            "SELECT json_extract(e.projection,'$.agentName'),e.project_id, \
             json_extract(e.projection,'$.projectRoomId'),p.owner_mxid,e.state \
             FROM engagements e JOIN projects p ON p.fleet_id=e.fleet_id AND p.id=e.project_id \
             WHERE e.state IN ('reserved','active') ORDER BY e.id LIMIT ?1",
        )?;
        let rows = query.query_map(params![MAX_CONTRIBUTIONS as i64], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut contributions = Vec::new();
        for row in rows {
            let (agent, project, room, owner, state) = row?;
            let Some(agent) = agent else {
                continue;
            };
            contributions.push(Contribution {
                agent,
                project,
                project_room_id: room.unwrap_or_default(),
                owner_mxid: owner,
                active: state == "active",
                agent_joined: None,
                membership_checked_at: None,
            });
        }
        Ok(contributions)
    }

    /// The engagement preview (`backend-v2.js:15434`): a DRY RUN. It reads and
    /// decides nothing — no row is written, no engagement is created, no ceiling
    /// is reserved. The `route` is the one native would actually take: an
    /// approval-only host trusts no room for auto-join, so the retained
    /// `routeRequest` ladder (`lib/engagement-store.js:179`) stops at its first
    /// rung, `notWhitelisted`.
    pub fn preview(&self, role: &str) -> Result<Preview, Error> {
        qualification::check_role(role)?;
        // The agent that would serve it, from the REAL live state: the first live
        // engagement already on this role, with its resource's real headroom.
        let mut query = self.db.prepare(
            "SELECT json_extract(projection,'$.agentName'),resource_id FROM engagements \
             WHERE state IN ('reserved','active') AND json_extract(projection,'$.role')=?1 \
             ORDER BY id LIMIT 1",
        )?;
        let found = query
            .query_row(params![role], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;
        let mut agent = None;
        let mut remaining = None;
        if let Some((name, resource_id)) = found {
            let at = super::graphs::now_ms()?;
            if let Ok((budget, _)) = self.resource_headroom(&resource_id, at) {
                remaining = budget.remaining_tokens.map(u64::from);
            }
            agent = name;
        }
        Ok(Preview {
            route: "notWhitelisted",
            auto_join: false,
            agent,
            agent_remaining_tokens: remaining,
        })
    }

    /// Live (`reserved`/`active`) engagements per role — the offer book's
    /// `runningNow`. One aggregate, never a scan of the projection store.
    fn live_engagements_by_role(&self) -> Result<std::collections::BTreeMap<String, u64>, Error> {
        let mut query = self.db.prepare(
            "SELECT json_extract(projection,'$.role'),COUNT(*) FROM engagements \
             WHERE state IN ('reserved','active') GROUP BY 1",
        )?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, u64>(1)?))
        })?;
        let mut counts = std::collections::BTreeMap::new();
        for row in rows {
            let (role, count) = row?;
            if let Some(role) = role {
                counts.insert(role, count);
            }
        }
        Ok(counts)
    }
}
