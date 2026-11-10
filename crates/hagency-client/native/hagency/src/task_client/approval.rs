//! The task-bound approval pair (ADR-064 amendment, PC-C3). Neither call names
//! an approval: the service derives it from the presented capability's task
//! (task → live dispatch → newest approval at the live fence), so the helper
//! cannot reach a sibling task's approval by naming an id.
use super::{Context, Error, transport};
use hagency_core::{approvals::ApprovalSummary, project::identifier};
use serde_json::Value;
use std::time::Duration;

pub(crate) const READ: &str = "get_approval";
pub(crate) const CONSUME: &str = "consume_approval";

/// The assigned task's live approval, or `None` when it has none. Bounded and
/// projection-only: `ApprovalSummary`'s four keys, never the owner card.
pub(crate) async fn read(
    context: &Context,
    deadline: Duration,
) -> Result<Option<ApprovalSummary>, Error> {
    let bytes = transport::request(context, transport::Operation::ApprovalRead, deadline).await?;
    serde_json::from_slice::<Option<ApprovalSummary>>(&bytes).map_err(|_| Error::Response)
}

/// Apply the owner's decision to the assigned task's approval.
///
/// At-most-once is the store's settled-state machine, not this `call_id` (TS
/// `consumeDecision` takes none, backend-v2.js:10968): a replay lands on
/// `already_consumed`. The `call_id` is the MCP mutation receipt the catalogue
/// already declares, so it is validated and carried, never re-derived.
pub(crate) async fn consume(
    context: &Context,
    call_id: &str,
    deadline: Duration,
) -> Result<Value, Error> {
    identifier(call_id, 512).map_err(|_| Error::Invalid)?;
    let bytes = transport::request(
        context,
        transport::Operation::ApprovalConsume { call_id },
        deadline,
    )
    .await?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Response)?;
    if value.get("decision").and_then(Value::as_str).is_none() {
        return Err(Error::Response);
    }
    Ok(value)
}
