use super::{parse, Id, ManagementCommand, ManagementPermission, Value};
use ::hagency::native_owner::PoolAgentInput;
use serde::Deserialize;
pub(super) const LIST: &str = "hagency.pools.list";
pub(super) const AGENTS: &str = "hagency.pools.agents.list";
pub(super) const GET: &str = "hagency.pools.agents.get";
pub(super) const REQUEST: &str = "hagency.pools.agents.request";
pub(super) const DECIDE: &str = "hagency.pools.agents.decide";
pub(super) const REALLOCATE: &str = "hagency.pools.agents.reallocate";
pub(super) const RENEW: &str = "hagency.pools.agents.renew";
pub(super) const STOP: &str = "hagency.pools.agents.stop";
pub(super) const ROOM: &str = "hagency.pools.agents.room";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct List {
    after: Option<String>,
    capability: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Agents {
    after: Option<String>,
    pool_id: Option<String>,
    #[serde(default)]
    needs_my_action: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    command_id: String,
    pool_id: String,
    request: PoolAgentInput,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decide {
    command_id: String,
    request_id: String,
    expected_revision: i64,
    approve: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stop {
    command_id: String,
    request_id: String,
    expected_revision: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Renew {
    command_id: String,
    request_id: String,
    expected_revision: i64,
    expected_period_key: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reallocate {
    command_id: String,
    request_id: String,
    expected_revision: i64,
    expected_grant_revision: i64,
    renewal_mode: hagency_core::recurring_quota::RenewalMode,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Room {
    pub command_id: String,
    pub name: String,
}
pub(super) fn permissions(service: &str) -> &'static [ManagementPermission] {
    use ManagementPermission::*;
    match service {
        LIST => &[ReadCatalog],
        AGENTS | GET => &[ReadAgentRequests],
        REQUEST | ROOM => &[RequestAgents],
        DECIDE | RENEW | REALLOCATE => &[DecideAgentRequests],
        STOP => &[CancelAgentRequests],
        _ => &[],
    }
}
pub(super) fn command(service: &str, args: Value) -> Result<Option<ManagementCommand>, String> {
    Ok(Some(match service {
        LIST => {
            let a: List = parse(args)?;
            ManagementCommand::Pools {
                after: a.after,
                capability: a.capability,
            }
        }
        AGENTS => {
            let a: Agents = parse(args)?;
            ManagementCommand::PoolAgents {
                after: a.after,
                pool_id: a.pool_id,
                needs_my_action: a.needs_my_action,
            }
        }
        GET => {
            let a: Id = parse(args)?;
            ManagementCommand::PoolAgent { id: a.id }
        }
        REQUEST => {
            let a: Request = parse(args)?;
            a.request.validate().map_err(|e| e.code)?;
            ManagementCommand::PoolAgentRequest {
                command_id: a.command_id,
                pool_id: a.pool_id,
                request: a.request,
            }
        }
        DECIDE => {
            let a: Decide = parse(args)?;
            ManagementCommand::PoolAgentDecide {
                command_id: a.command_id,
                request_id: a.request_id,
                expected_revision: a.expected_revision,
                approve: a.approve,
            }
        }
        REALLOCATE => {
            let a: Reallocate = parse(args)?;
            ManagementCommand::PoolAgentReallocate {command_id:a.command_id, request_id:a.request_id, expected_revision:a.expected_revision, expected_grant_revision:a.expected_grant_revision, renewal_mode:a.renewal_mode}
        }
        RENEW => {
            let a: Renew = parse(args)?;
            ManagementCommand::PoolAgentRenew {command_id:a.command_id, request_id:a.request_id, expected_revision:a.expected_revision, expected_period_key:a.expected_period_key}
        }
        STOP => {
            let a: Stop = parse(args)?;
            ManagementCommand::PoolAgentStop {
                command_id: a.command_id,
                request_id: a.request_id,
                expected_revision: a.expected_revision,
            }
        }
        _ => return Ok(None),
    }))
}

/// Dates are display metadata only. The server supplies the authorization's
/// period; formatting always uses the owner's IANA zone, never this device's.
pub(super) fn display(mut value: Value) -> Value {
    fn visit(value: &mut Value) {
        if let Some(rows) = value.as_array_mut() { for row in rows { visit(row); } return; }
        let Some(object) = value.as_object_mut() else { return };
        for key in ["items", "request", "data"] { if let Some(child) = object.get_mut(key) { visit(child); } }
        let Some(calendar) = object.get("calendar").and_then(|c| serde_json::from_value::<hagency_core::recurring_quota::Calendar>(c.clone()).ok()) else { return };
        let Ok(zone) = calendar.time_zone.parse::<chrono_tz::Tz>() else { return };
        let reset = object.get("currentPeriod").and_then(|p| p.get("nextResetMs")).and_then(Value::as_u64)
            .or_else(|| object.get("currentPeriodEndMs").and_then(Value::as_u64))
            .or_else(|| calendar.period_at(super::grants::now_ms()).ok().map(|p| p.end_ms));
        let Some(at) = reset.and_then(|ms| i64::try_from(ms).ok()).and_then(chrono::DateTime::from_timestamp_millis) else { return };
        object.insert("periodResetLabel".into(), Value::String(format!("{} · {}", at.with_timezone(&zone).format("%Y-%m-%d %H:%M %Z"), calendar.time_zone)));
    }
    visit(&mut value);
    value
}

#[cfg(test)]
mod recurring_display_tests {
    use super::*;
    #[test]
    fn reset_time_uses_owner_zone_and_keeps_unknown_usage_unknown() {
        let value=display(serde_json::json!({"request":{"calendar":{"cadence":"daily","timeZone":"America/Los_Angeles"},"currentPeriod":{"nextResetMs":1793606400000u64,"remainingTokens":null}}}));
        assert_eq!(value["request"]["periodResetLabel"],"2026-11-02 00:00 PST · America/Los_Angeles");
        assert!(value["request"]["currentPeriod"]["remainingTokens"].is_null());
        assert_eq!(display(serde_json::json!({"id":"legacy"})),serde_json::json!({"id":"legacy"}));
    }
}
