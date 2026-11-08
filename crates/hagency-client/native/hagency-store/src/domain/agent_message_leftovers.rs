//! Board #49: the operator message board, the delivery-event log, agent
//! tombstones and the avatar request queue (migration 067).
//!
//! These four replace the retained backend's `messages.json` board
//! (`backend-v2.js:1721`), its `message-delivery-events.jsonl` append log
//! (`:1543,1602`), its `deleted_agents.json` tombstone map (`:1718`) and the
//! avatar route's SSE hand-off (`:16370`). None of them is a Matrix ingress
//! fact — `operator_messages` is the OPERATOR board and carries no ingress
//! authority (that is `admitted_messages`, a different model keyed by source
//! event and projected into runner sessions).
//!
//! Two retained sub-behaviours have NO native source and are documented rather
//! than faked, the same choice `delivery_feedback.rs` records:
//!   * `isGroupMember` (`:4429`) — the port has no project-group membership
//!     table, so `messageTargetsAgent` cannot infer membership. A group message
//!     targets the agent only via the recorded default recipient, an explicit
//!     mention or a recorded room recipient.
//!   * the unread index (`getUnreadInboxMessages`, `:4251`) is the retained
//!     process's in-memory index, not a stored fact. The native board keeps no
//!     reader state, so a message is unread for its target while it is not
//!     suppressed for that target — which is what the retained index reduces to
//!     for the one question the route asks.
use super::DomainRepository;
use crate::Error;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::{Value, json};

/// The retained `relativeTime` (`backend-v2.js:4031-4037`): floored whole
/// seconds, minutes, hours or days ago, with the exact suffixes.
pub fn relative_time(now: u64, ts: u64) -> String {
    let d = now.saturating_sub(ts);
    if d < 60_000 {
        format!("{}s ago", d / 1_000)
    } else if d < 3_600_000 {
        format!("{}m ago", d / 60_000)
    } else if d < 86_400_000 {
        format!("{}h ago", d / 3_600_000)
    } else {
        format!("{}d ago", d / 86_400_000)
    }
}

/// One operator-board message as `GET /api/messages/:id` serves it
/// (`backend-v2.js:16900-16914`): the record's own fields, `priority`
/// normalized, `schema` reduced to `{kind,version[,payload]}` (`:4109`), `ts`
/// omitted (`ts: undefined`) and `time` the retained `relativeTime`.
#[derive(Debug, Clone, Serialize)]
pub struct OperatorMessage {
    pub id: String,
    pub from: String,
    #[serde(rename = "to")]
    pub recipient: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub priority: String,
    pub summary: String,
    pub full: String,
    pub mentions: Value,
    pub attachments: Value,
    pub time: String,
    pub reply_to: Option<String>,
    pub group: Option<String>,
    pub source: String,
    #[serde(rename = "sourceRoom")]
    pub source_room: Option<String>,
    #[serde(rename = "sourceEventId")]
    pub source_event_id: Option<String>,
    #[serde(rename = "senderMxid")]
    pub sender_mxid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    #[serde(rename = "suppressedRecipients")]
    pub suppressed_recipients: Vec<String>,
}

/// One delivery event as `GET /api/agents/:name/delivery-events` serves it
/// (`backend-v2.js:16988-17000`): the row's own fields, newest first.
#[derive(Debug, Clone, Serialize)]
pub struct DeliveryEventRow {
    pub id: u64,
    #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
    pub ts: u64,
}

/// One tombstone: the name a force-delete removed (`:1718,4354`).
#[derive(Debug, Clone, Serialize)]
pub struct Tombstone {
    pub name: String,
    #[serde(rename = "deletedAt")]
    pub deleted_at: u64,
    pub reason: String,
}

/// The outcome of `POST /api/messages/:id/suppress` (`:17002-17040`).
#[derive(Debug, Clone, Serialize)]
pub struct Suppression {
    pub was_unread: bool,
    pub is_unread_now: bool,
    pub suppressed_recipients: Vec<String>,
}

/// One board message to persist: the retained record's own fields
/// (`backend-v2.js:16480` destructure), with `ts` as `created_at`.
#[derive(Debug, Clone)]
pub struct NewOperatorMessage {
    pub id: String,
    pub sender: String,
    pub recipient: Option<String>,
    pub kind: String,
    pub priority: String,
    pub summary: String,
    pub full: String,
    pub mentions: Value,
    pub attachments: Value,
    pub created_at: u64,
    pub reply_to: Option<String>,
    pub group: Option<String>,
    pub source: String,
    pub source_room: Option<String>,
    pub source_event_id: Option<String>,
    pub sender_mxid: Option<String>,
    pub room_recipients: Value,
    pub default_recipient: Option<String>,
    pub schema_kind: Option<String>,
    pub schema_version: Option<u32>,
    pub schema_payload: Option<String>,
}

/// The store's answer to a suppress call. `Unknown` is the route's 404;
/// `NotDeliverable` its 400; `Recorded` its 200 body.
#[derive(Debug, Clone)]
pub enum SuppressOutcome {
    Unknown,
    NotDeliverable,
    Recorded(Suppression),
}

fn parse_array(value: &str) -> Vec<String> {
    serde_json::from_str::<Value>(value)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn value_of(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| json!([]))
}

impl DomainRepository {
    /// The board WRITER: the persistence half of the retained `POST /api/messages`
    /// (`backend-v2.js:16480`, its `messages.push(msg)` + `saveMessages()` at
    /// `:4500`). Idempotent on the id, so a replayed delivery stays one row. The
    /// HTTP route itself is a separate task; this is the store seam both it and
    /// the tests write through, never a test-only hook.
    pub fn record_operator_message(&mut self, message: NewOperatorMessage) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO operator_messages(id,sender,recipient,kind,priority,summary,full,mentions,attachments,created_at,reply_to,group_id,source,source_room,source_event_id,sender_mxid,room_recipients,default_recipient,schema_kind,schema_version,schema_payload,suppressed) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,'[]') \
             ON CONFLICT(id) DO NOTHING",
            params![
                message.id,
                message.sender,
                message.recipient,
                message.kind,
                message.priority,
                message.summary,
                message.full,
                serde_json::to_string(&message.mentions)?,
                serde_json::to_string(&message.attachments)?,
                i64::try_from(message.created_at).unwrap_or(i64::MAX),
                message.reply_to,
                message.group,
                message.source,
                message.source_room,
                message.source_event_id,
                message.sender_mxid,
                serde_json::to_string(&message.room_recipients)?,
                message.default_recipient,
                message.schema_kind,
                message.schema_version.map(i64::from),
                message.schema_payload,
            ],
        )?;
        Ok(())
    }

    /// `GET /api/messages/:id` (`backend-v2.js:16900`): the one operator-board
    /// message with this id, or `None` for the route's 404.
    pub fn operator_message(&self, id: &str, now: u64) -> Result<Option<OperatorMessage>, Error> {
        let row = self
            .db
            .query_row(
                "SELECT id,sender,recipient,kind,priority,summary,full,mentions,attachments,created_at,reply_to,group_id,source,source_room,source_event_id,sender_mxid,schema_kind,schema_version,schema_payload,suppressed FROM operator_messages WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?, r.get::<_, String>(7)?, r.get::<_, String>(8)?,
                        r.get::<_, i64>(9)?, r.get::<_, Option<String>>(10)?, r.get::<_, Option<String>>(11)?,
                        r.get::<_, String>(12)?, r.get::<_, Option<String>>(13)?, r.get::<_, Option<String>>(14)?,
                        r.get::<_, Option<String>>(15)?, r.get::<_, Option<String>>(16)?,
                        r.get::<_, Option<i64>>(17)?, r.get::<_, Option<String>>(18)?, r.get::<_, String>(19)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            id,
            from,
            recipient,
            kind,
            priority,
            summary,
            full,
            mentions,
            attachments,
            created_at,
            reply_to,
            group,
            source,
            source_room,
            source_event_id,
            sender_mxid,
            schema_kind,
            schema_version,
            schema_payload,
            suppressed,
        )) = row
        else {
            return Ok(None);
        };
        // `normalizeMessageSchema` (`:4109`): `{kind, version[, payload]}`.
        let schema = schema_kind.map(|kind| {
            let version = schema_version.unwrap_or(1);
            match schema_payload.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) {
                Some(payload) => json!({"kind":kind,"version":version,"payload":payload}),
                None => json!({"kind":kind,"version":version}),
            }
        });
        Ok(Some(OperatorMessage {
            id,
            from,
            recipient,
            kind,
            priority,
            summary,
            full,
            mentions: value_of(&mentions),
            attachments: value_of(&attachments),
            time: relative_time(now, u64::try_from(created_at).unwrap_or_default()),
            reply_to,
            group,
            source,
            source_room,
            source_event_id,
            sender_mxid,
            schema,
            suppressed_recipients: parse_array(&suppressed),
        }))
    }

    /// `POST /api/messages/:id/suppress` (`backend-v2.js:17002`): record the
    /// agent in the message's `suppressedRecipients`, idempotently, and append
    /// the `message.suppressed` delivery event the retained route appends
    /// (`:17022`).
    pub fn suppress_message(
        &mut self,
        id: &str,
        agent: &str,
        reason: &str,
        now: u64,
    ) -> Result<SuppressOutcome, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        #[allow(clippy::type_complexity)]
        let row: Option<(String, Option<String>, String, Option<String>, String)> = tx
            .query_row(
                "SELECT mentions,default_recipient,room_recipients,recipient,suppressed FROM operator_messages WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((mentions, default_recipient, room_recipients, recipient, suppressed)) = row
        else {
            return Ok(SuppressOutcome::Unknown);
        };
        // `messageTargetsAgent` (`:4424`), minus the inferred membership the
        // port cannot know (documented above).
        let room = parse_array(&room_recipients);
        let mentioned = parse_array(&mentions);
        let deliverable = room.iter().any(|name| name == agent)
            || recipient.as_deref() == Some(agent)
            || (default_recipient.as_deref() == Some(agent)
                && mentioned.iter().any(|name| name == agent));
        if !deliverable {
            return Ok(SuppressOutcome::NotDeliverable);
        }
        let mut recipients = parse_array(&suppressed);
        let already = recipients.iter().any(|name| name == agent);
        if !already {
            recipients.push(agent.to_owned());
            let encoded = serde_json::to_string(&recipients)?;
            tx.execute(
                "UPDATE operator_messages SET suppressed=?2 WHERE id=?1",
                params![id, encoded],
            )?;
            tx.execute(
                "INSERT INTO delivery_events(agent,message_id,kind,source,reason,created_at) VALUES(?1,?2,'message.suppressed','backend',?3,?4)",
                params![agent, id, reason, i64::try_from(now).unwrap_or(i64::MAX)],
            )?;
        }
        tx.commit()?;
        // The unread model documented above: unread for the target until
        // suppressed. `was_unread` is true only on the first suppression.
        Ok(SuppressOutcome::Recorded(Suppression {
            was_unread: !already,
            is_unread_now: false,
            suppressed_recipients: recipients,
        }))
    }

    /// `GET /api/agents/:name/delivery-events` (`backend-v2.js:16988`): the
    /// agent's delivery events, newest first, bounded — the read the retained
    /// `readDeliveryEvents({agent, limit})` performed over the jsonl log.
    pub fn delivery_events(&self, agent: &str, limit: u32) -> Result<Vec<DeliveryEventRow>, Error> {
        let bound = i64::from(limit.clamp(1, 1000));
        let mut query = self.db.prepare(
            "SELECT id,message_id,kind,agent,source,reason,context,created_at FROM delivery_events WHERE agent=?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = query.query_map(params![agent, bound], |r| {
            Ok(DeliveryEventRow {
                id: u64::try_from(r.get::<_, i64>(0)?).unwrap_or_default(),
                message_id: r.get(1)?,
                kind: r.get(2)?,
                agent: r.get(3)?,
                source: r.get(4)?,
                reason: r.get(5)?,
                context: r
                    .get::<_, Option<String>>(6)?
                    .and_then(|raw| serde_json::from_str(&raw).ok()),
                ts: u64::try_from(r.get::<_, i64>(7)?).unwrap_or_default(),
            })
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    /// `POST /api/agents/:name/undelete` (`backend-v2.js:12308`): remove the
    /// tombstone, so re-registration is allowed. `false` means there was none —
    /// the route's 404 `no tombstone found`.
    pub fn undelete_agent(&mut self, name: &str) -> Result<bool, Error> {
        let removed = self
            .db
            .execute("DELETE FROM agent_tombstones WHERE name=?1", [name])?;
        Ok(removed > 0)
    }

    /// The tombstone WRITER: the force-delete `undelete` reverses
    /// (`backend-v2.js:4354`). Idempotent on the name.
    pub fn record_agent_tombstone(
        &mut self,
        name: &str,
        reason: &str,
        now: u64,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO agent_tombstones(name,deleted_at,reason) VALUES(?1,?2,?3) ON CONFLICT(name) DO UPDATE SET deleted_at=?2,reason=?3",
            params![name, i64::try_from(now).unwrap_or(i64::MAX), reason],
        )?;
        Ok(())
    }

    /// `POST /api/agents/:name/avatar` (`backend-v2.js:16370`): enqueue the
    /// avatar request the retained route handed to the bridge over SSE. The
    /// request is durable; the base64 payload never was (the retained route
    /// held it only in the SSE frame).
    pub fn record_avatar_request(
        &mut self,
        agent: &str,
        regenerate: bool,
        custom: bool,
        mime: Option<&str>,
        now: u64,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO avatar_requests(agent,regenerate,custom,mime,requested_at) VALUES(?1,?2,?3,?4,?5)",
            params![
                agent,
                i64::from(regenerate),
                i64::from(custom),
                mime,
                i64::try_from(now).unwrap_or(i64::MAX)
            ],
        )?;
        Ok(())
    }
}
