//! Delivery-feedback notices: the exact text the ROOM is owed when a message or
//! attachment did not reach everyone. Ported 1:1 from the retained bridge's
//! `handleMessageDeliveryFeedback` (bridge-matrix.js:6492-6572) and
//! `sendAttachmentsForMessage` (:11045-11052); the warnings themselves are the
//! backend's `backend-v2.js:16763-16835`.
//!
//! Same user-visible text as TS, character for character — including the `⚠️ `
//! prefix, the `@name (reason)` form, the `', '` join and the trailing period.
//! `lines()` is the whole port of the TS `lines` array; the wiring decides WHERE
//! it is said, never WHAT it says.
use super::{execution, task_intents};
use crate::Error;
use hagency_core::{messages::Message, replies::ReplyRoute};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

/// The notice `kind` prefix. The full kind carries the message sequence so two
/// failed messages on one task each get their own notice, and a re-admission of
/// the same event stays idempotent on the derived id.
pub(super) const KIND: &str = "delivery_feedback";

/// TS default agent prefix (`bridge-matrix.js:371`, `MATRIX_AGENT_PREFIX`).
const AGENT_PREFIX: &str = "ac_";

/// The agent NAME a mention is rendered as, from the MXID this port stores.
/// TS never renders an MXID here: `parseMentions` resolves each mention to a
/// name (`bridge-matrix.js:3133-3152`) — `agentNameFromUserId` strips the
/// `@`, the server and the agent prefix (`@ac_zoe:example.test` → `zoe`,
/// :3075-3083), and the no-prefix fallback takes the localpart (`@zoe:…` →
/// `zoe`, :3141-3143). The warnings then join those names (`:6492-6572`), so
/// the room reads `@zoe` — never `@zoe:example.test`.
fn agent_name(mxid: &str) -> String {
    let localpart = mxid
        .trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or(mxid);
    localpart
        .strip_prefix(AGENT_PREFIX)
        .unwrap_or(localpart)
        .to_string()
}

/// Resolve a human's group message against what THIS port can actually know.
///
/// TS asks the backend's agent registry for `exists`/`online` (a tmux-pane
/// liveness model); this port has no pane concept, so the two facts it does own
/// are used instead, and they are the two that decide the `unknown` and
/// `not_in_group` cases:
///   * `known`  — the MXID is a current transport identity (`matrix_transports`),
///     i.e. an agent this fleet has actually provisioned.
///   * `member` — the MXID is in the room's joined set (`matrix_room_scopes`).
///
/// `online` is reported true and `router_served` false: without a pane model
/// there is no evidence of offline, and this port never invents a failure TS
/// would not have. So `target_offline` / `mentions_offline` are ported as text
/// and API but are not producible here — say so rather than fake them.
pub(super) fn group_feedback(
    db: &Connection,
    route: &ReplyRoute,
    mentions: &BTreeSet<String>,
) -> Result<DeliveryFeedback, Error> {
    let mut states = Vec::new();
    for mxid in mentions {
        // Our own bot is not an agent to warn about (TS's `name !== msg.from`).
        if mxid == &route.sender_mxid {
            continue;
        }
        let known: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM matrix_transports WHERE sender_mxid=?1)",
            [mxid],
            |r| r.get(0),
        )?;
        let member: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM matrix_room_scopes r, json_each(r.joined) m \
             WHERE r.server_name=?1 AND r.room_id=?2 AND m.value=?3)",
            params![route.server_name, route.room_id, mxid],
            |r| r.get(0),
        )?;
        states.push(MentionState {
            target: agent_name(mxid),
            known,
            online: true,
            member,
            router_served: false,
            reason: String::new(),
        });
    }
    // `default_recipient` is the agent's own name, so the `not-in-group`
    // comparison stays name-to-name (TS compares names, `:16820-16822`).
    Ok(DeliveryFeedback::warnings(
        None,
        &states,
        &agent_name(&route.sender_mxid),
    ))
}

/// Emit the feedback notice for one admitted group message, if TS would say
/// anything. Reuses the notice transport the room already has
/// (`task_intents::add_notice` → the claim/send path), because TS's
/// `sendDeliveryNotice` is likewise `sayInRoom` for the room's own message.
///
/// `sequence` is the admitted message's sequence, so two failed messages on one
/// task each get their own notice and a re-admission of the same event derives
/// the same id and stays idempotent. Best effort and additive: it never changes
/// the admission's own outcome.
pub(super) fn emit(
    tx: &Transaction<'_>,
    task_id: &str,
    root: &Message,
    mentions: &BTreeSet<String>,
    sequence: u64,
    now: u64,
) -> Result<(), Error> {
    let task = execution::task(tx, task_id)?;
    let route = super::matrix_routes::route(tx, &task.session_id)?;
    let feedback = group_feedback(tx, &route, mentions)?;
    let Some(body) = feedback.notice_body() else {
        return Ok(());
    };
    let kind = format!("{KIND}_{sequence}");
    // The same derivation `task_intents` uses, so re-admission is a no-op.
    let id = format!(
        "notice_{}",
        hagency_core::canonical::digest(&serde_json::json!([task_id, kind]))?
    );
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_notices WHERE id=?1)",
        [&id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    task_intents::add_notice(tx, &task, root, &kind, body, now)?;
    Ok(())
}

/// One feedback warning, mirroring the backend's `warnings` entries. The `code`
/// spelling is the TS wire spelling, so a persisted payload stays readable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryWarning {
    /// A direct target that is not reachable (backend-v2.js:16783).
    TargetOffline {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server: Option<String>,
        #[serde(default)]
        reason: String,
        #[serde(default)]
        queued: bool,
    },
    /// Mentions that exist but are offline (backend-v2.js:16810).
    MentionsOffline { targets: Vec<MentionTarget> },
    /// Mentions that are in neither the registry nor the group (:16817).
    MentionsUnknown { targets: Vec<MentionTarget> },
    /// Mentions that exist but are not members of this group (:16824).
    MentionsNotInGroup { targets: Vec<MentionTarget> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MentionTarget {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One mention's resolved state, as the backend computed it before building
/// the warning (`backend-v2.js:16797-16802`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionState {
    pub target: String,
    /// `state.exists` — the name is in the agent registry.
    pub known: bool,
    /// `state.online`.
    pub online: bool,
    /// `isGroupMember`.
    pub member: bool,
    /// `isRouterServedAgent(name)` — a router-served agent has no pane and its
    /// reachability comes from disposable runners, so it is never warned about.
    pub router_served: bool,
    /// `state.offlineReason || 'offline'`.
    pub reason: String,
}

/// The direct target's resolved state (`backend-v2.js:16783`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectTarget<'a> {
    pub target: &'a str,
    pub server: Option<&'a str>,
    pub reason: &'a str,
    /// The backend sets `queued: true` for the offline-direct-target warning.
    pub queued: bool,
    pub online: bool,
    pub router_served: bool,
}

/// The backend's answer to a submit, as the bridge consumes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryFeedback {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub warnings: Vec<DeliveryWarning>,
}

/// TS renders `@${name}` because its mention targets are registry NAMES. This
/// port's mention set holds full MXIDs, so `group_feedback` resolves each one
/// to its name before building the warning (`agent_name`), and every caller
/// here passes names — the pure text tests use the bare form, and the ported
/// group path now does too. The sigil is still added only when absent, so a
/// name whose text already begins with `@` is never doubled.
fn at(target: &str) -> String {
    if target.starts_with('@') {
        target.to_owned()
    } else {
        format!("@{target}")
    }
}

/// The `@name` list TS builds for the three mention warnings: `', '` joined,
/// each `@target` with an optional ` (reason)`. TS drops entries with a falsy
/// `target`, and the whole line when nothing survives.
fn mention_list(targets: &[MentionTarget], with_reason: bool) -> String {
    targets
        .iter()
        .filter(|item| !item.target.is_empty())
        .map(|item| match (with_reason, item.reason.as_deref()) {
            (true, Some(reason)) if !reason.is_empty() => {
                format!("{} ({reason})", at(&item.target))
            }
            _ => at(&item.target),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl DeliveryFeedback {
    /// The TS `lines` array, in TS order: the `error` line first, then the
    /// warnings in the order the backend emitted them.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(error) = &self.error {
            lines.push(format!("⚠️ Message not delivered: {error}"));
        }
        for warning in &self.warnings {
            match warning {
                DeliveryWarning::TargetOffline {
                    target,
                    reason,
                    queued,
                    ..
                } => {
                    let reason = if reason.is_empty() {
                        String::new()
                    } else {
                        format!(" ({reason})")
                    };
                    if *queued {
                        lines.push(format!(
                            "⚠️ @{target} is offline{reason}. Message queued; it will be delivered when the agent is online. It may be time-sensitive."
                        ));
                    } else {
                        lines.push(format!(
                            "⚠️ @{target} is offline{reason}. Message archived only and was not delivered."
                        ));
                    }
                }
                DeliveryWarning::MentionsOffline { targets } => {
                    let targets = mention_list(targets, true);
                    if !targets.is_empty() {
                        lines.push(format!(
                            "⚠️ Offline mentions were archived only: {targets}."
                        ));
                    }
                }
                DeliveryWarning::MentionsUnknown { targets } => {
                    let targets = mention_list(targets, false);
                    if !targets.is_empty() {
                        lines.push(format!(
                            "⚠️ Mention targets not found in agent registry: {targets}."
                        ));
                    }
                }
                DeliveryWarning::MentionsNotInGroup { targets } => {
                    let targets = mention_list(targets, false);
                    if !targets.is_empty() {
                        lines.push(format!(
                            "⚠️ Mentions not delivered because targets are not members of this group: {targets}."
                        ));
                    }
                }
            }
        }
        lines
    }

    /// The single notice body: the lines joined by `'\n'`, exactly the string TS
    /// hands `sendDeliveryNotice` (`:6566`). `None` when TS would say nothing.
    pub fn notice_body(&self) -> Option<String> {
        let lines = self.lines();
        if lines.is_empty() {
            return None;
        }
        Some(lines.join("\n"))
    }

    /// `submitHumanMessage`'s timeout-retry arm (`:6552`): the detail is
    /// `'timeout'` for an abort-timeout, else the error's message.
    pub fn delivery_failed_after_retry(detail: &str) -> String {
        format!("⚠️ Message delivery failed after retry ({detail}).")
    }

    /// `submitHumanMessage`'s plain failure arm (`:6560`).
    pub fn delivery_failed(detail: &str) -> String {
        format!("⚠️ Message delivery failed ({detail}).")
    }

    /// `sendAttachmentsForMessage` (`:11048`): `pathHint` is the attachment's
    /// trimmed path, or the literal `(unknown path)` TS substitutes.
    pub fn attachment_not_delivered(message_id: &str, path_hint: &str, error: &str) -> String {
        let hint = if path_hint.trim().is_empty() {
            "(unknown path)"
        } else {
            path_hint.trim()
        };
        format!("⚠️ Attachment not delivered for {message_id}: {hint} ({error})")
    }

    /// What the backend decided about ONE mention (backend-v2.js:16797-16827).
    /// `member` is `isGroupMember`, `known` is `state.exists`, `online` is
    /// `state.online`.
    pub fn warnings(
        direct: Option<DirectTarget<'_>>,
        mentions: &[MentionState],
        default_recipient: &str,
    ) -> DeliveryFeedback {
        let mut warnings = Vec::new();
        // A direct target that is not reachable, and is not router-served
        // (`isRouterServedAgent`, :16778). Router-served agents are excluded
        // here exactly as TS excludes them.
        if let Some(target) = direct.filter(|target| !target.router_served && !target.online) {
            warnings.push(DeliveryWarning::TargetOffline {
                target: target.target.to_owned(),
                server: target.server.map(str::to_owned),
                reason: target.reason.to_owned(),
                queued: target.queued,
            });
        }
        // Mentions never include the sender (`:16799`).
        let offline: Vec<MentionTarget> = mentions
            .iter()
            .filter(|item| item.known && !item.online && !item.router_served)
            .map(|item| MentionTarget {
                target: item.target.clone(),
                reason: Some(item.reason.clone()),
            })
            .collect();
        if !offline.is_empty() {
            warnings.push(DeliveryWarning::MentionsOffline { targets: offline });
        }
        let unknown: Vec<MentionTarget> = mentions
            .iter()
            .filter(|item| !item.known && !item.member)
            .map(|item| MentionTarget {
                target: item.target.clone(),
                reason: Some("not-found".into()),
            })
            .collect();
        if !unknown.is_empty() {
            warnings.push(DeliveryWarning::MentionsUnknown { targets: unknown });
        }
        let out_of_group: Vec<MentionTarget> = mentions
            .iter()
            .filter(|item| item.known && !item.member && item.target != default_recipient)
            .map(|item| MentionTarget {
                target: item.target.clone(),
                reason: Some("not-in-group".into()),
            })
            .collect();
        if !out_of_group.is_empty() {
            warnings.push(DeliveryWarning::MentionsNotInGroup {
                targets: out_of_group,
            });
        }
        DeliveryFeedback {
            error: None,
            warnings,
        }
    }

    /// The persisted form of a body: `{"body": ..., "codes": [...]}` so a
    /// stored notice carries both the exact text and which cases produced it.
    pub fn notice_record(body: &str, feedback: &DeliveryFeedback) -> serde_json::Value {
        let codes: Vec<&str> = feedback
            .warnings
            .iter()
            .map(|warning| match warning {
                DeliveryWarning::TargetOffline { .. } => "target_offline",
                DeliveryWarning::MentionsOffline { .. } => "mentions_offline",
                DeliveryWarning::MentionsUnknown { .. } => "mentions_unknown",
                DeliveryWarning::MentionsNotInGroup { .. } => "mentions_not_in_group",
            })
            .collect();
        json!({ "body": body, "codes": codes })
    }
}
