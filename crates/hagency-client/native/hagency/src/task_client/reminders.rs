//! Schedule a self-reminder for the assigned task's own session (board #53).
use super::{Context, Error, transport};
use hagency_core::JSON_SAFE_MAX;
use serde_json::Value;
use std::time::Duration;

pub(crate) const NAME: &str = "schedule_reminder";

/// `msg` is the agent's own free text (the reminder it will be woken with);
/// `delay_ms` is the positive delay. The service returns the TS-shaped
/// `{id, fire_at, remaining_ms}` receipt.
pub(crate) async fn run(
    context: &Context,
    msg: String,
    delay_ms: u64,
    deadline: Duration,
) -> Result<Value, Error> {
    if msg.trim().is_empty() || msg.len() > 32 * 1024 || msg.contains('\0') {
        return Err(Error::Invalid);
    }
    if delay_ms == 0 || delay_ms > JSON_SAFE_MAX {
        return Err(Error::Invalid);
    }
    let bytes = transport::request(
        context,
        transport::Operation::ScheduleReminder {
            msg: &msg,
            delay_ms,
        },
        deadline,
    )
    .await?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Unknown)?;
    if value.get("id").and_then(Value::as_i64).is_none()
        || value.get("fire_at").and_then(Value::as_u64).is_none()
        || value.get("remaining_ms").and_then(Value::as_u64).is_none()
    {
        return Err(Error::Unknown);
    }
    Ok(value)
}
