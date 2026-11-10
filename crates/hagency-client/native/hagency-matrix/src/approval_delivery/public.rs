//! The redacted public status notice (ADR-137): a content-free status word
//! in the project room. Its own validator — a sibling of `Frozen`, never a
//! widening of it — and never a channel for request material.
//!
//! TS parity: `buildPublicApprovalNotice` (bridge-matrix.js:2578-2598) is the
//! source of this packet — `msgtype` `com.agentchat.approval.status.v1`, the
//! body `Agent {agent} is waiting for approval from its owner.`, the status
//! detail under the shared `com.agentchat.approval` key (the request card's
//! own key, bridge-matrix.js:215), and an `m.thread` relation to the task
//! root when the approval carries one (`approval.thread_root_event_id`).
use crate::Error;
use hagency_core::{
    approvals::ApprovalRoomAuthority,
    project::identifier,
    replies::{matrix_event, matrix_room, matrix_user},
};
use hagency_store::PrivateApprovalCard;
use serde_json::{Value, json};

pub(crate) const MAX_NOTICE_BODY: usize = 512;
/// The profile display name's own cap (lib/matrix-agent-profile.js:6-8 takes
/// 128 chars) — the name in the notice is that same string.
pub(crate) const MAX_AGENT_NAME: usize = 128;
pub(crate) const NOTICE_MSGTYPE: &str = "com.agentchat.approval.status.v1";
/// The shared Agent Chat approval key. TS sends the status detail under the
/// SAME key as the request card (`APPROVAL_EVENT_KEY`, bridge-matrix.js:215).
pub(crate) const STATUS_KEY: &str = "com.agentchat.approval";
/// The Matrix event type of the send. TS always PUTs `m.room.message`
/// (`sendAsAgentContent`, bridge-matrix.js:10825) whatever the `msgtype` is;
/// the msgtype is a content field, never the PUT path segment.
pub(crate) const EVENT_TYPE: &str = "m.room.message";
pub(crate) const STATE: &str = "waiting_for_owner";

/// The TS builder, 1:1 (bridge-matrix.js:2578-2598). `thread_root` is empty
/// exactly when TS's `thread_root_event_id` is absent, and then the relation is
/// omitted — the spread `...(threadRoot ? {...} : {})` is a plain condition.
pub(crate) fn build_public_approval_notice(
    agent: &str,
    project: &str,
    thread_root: Option<&str>,
) -> Value {
    let agent = agent.trim();
    let project = project.trim();
    let mut value = json!({
        "msgtype": NOTICE_MSGTYPE,
        "body": format!("Agent {agent} is waiting for approval from its owner."),
        STATUS_KEY: {
            "version": 1,
            "kind": "status",
            "agent": agent,
            "project": project,
            "state": STATE,
        }
    });
    if let Some(root) = thread_root.filter(|root| !root.is_empty()) {
        value["m.relates_to"] = json!({
            "rel_type": "m.thread",
            "event_id": root,
            "is_falling_back": true,
            "m.in_reply_to": {"event_id": root},
        });
    }
    value
}

/// The exact status packet: three top-level keys, plus the thread relation
/// when a task root is known. The status object carries only the status word
/// and the agent/project identity pair the destination re-derivation already
/// binds — no request id, no digest, no tool name, no preview, no scope key in
/// any byte.
#[derive(Clone)]
pub(crate) struct PublicFrozen {
    /// The room-visible name (TS `approval.agent`) — never the engagement id.
    agent: String,
    /// The engagement id, kept ONLY as the binding this notice is addressed
    /// from; it is never rendered into a room string.
    engagement: String,
    project: String,
    server_name: String,
    project_room_id: String,
    private_room_id: String,
    bot_mxid: String,
    /// The task thread this approval belongs to, when one is known. `None` is
    /// TS's own no-thread case: the relation is simply absent.
    thread_root: Option<String>,
    body: String,
    digest: String,
}
impl PublicFrozen {
    /// `thread_root` is the task thread root the notice belongs in, or `None`
    /// when the approval is not thread-scoped — TS reads this from
    /// `approval.thread_root_event_id` and omits the relation when it is empty.
    pub fn new(card: &PrivateApprovalCard, thread_root: Option<String>) -> Result<Self, Error> {
        let t = card.target();
        let a = &t.authority;
        // The room string names the AGENT, never its engagement id
        // (bridge-matrix.js:2585; board #99). The room's display name for the
        // agent is the same string, set by `reconcile_agent_profile`.
        let agent = a.agent_name.trim().to_owned();
        let value = Self {
            body: format!("Agent {agent} is waiting for approval from its owner."),
            agent,
            engagement: a.engagement_id.trim().to_owned(),
            project: a.project_id.clone(),
            server_name: a.server_name.clone(),
            project_room_id: a.project_room_id.clone(),
            private_room_id: a.room_id.clone(),
            bot_mxid: a.bot_mxid.clone(),
            thread_root,
            digest: String::new(),
        };
        let mut value = value;
        value.digest = value.hash()?;
        value.validate()?;
        Ok(value)
    }
    fn hash(&self) -> Result<String, Error> {
        crate::approval_delivery::state::hash(&json!([
            "approval-status-v1",
            self.agent,
            self.project,
            self.project_room_id,
            self.thread_root,
            self.body
        ]))
    }
    /// The PUT path event type. Always `m.room.message`, like every other
    /// agent send — never the msgtype.
    pub fn event_type(&self) -> &str {
        EVENT_TYPE
    }
    /// Deterministic transaction id: one identity per notice content; the
    /// send is never re-attempted, so the id is never reused by this host.
    pub fn transaction(&self) -> String {
        format!("approval_status_{}", self.digest)
    }
    /// The exact PUT segments TS builds (bridge-matrix.js:10823-10825):
    /// `/_matrix/client/v3/rooms/{roomId}/send/m.room.message/{txnId}`.
    pub fn segments(&self, project_room_id: &str) -> Vec<String> {
        vec![
            "_matrix".into(),
            "client".into(),
            "v3".into(),
            "rooms".into(),
            project_room_id.into(),
            "send".into(),
            self.event_type().into(),
            self.transaction(),
        ]
    }
    pub fn content(&self) -> Result<String, Error> {
        let value = self.packet();
        self.validate_packet(&value)?;
        serde_json::to_string(&value).map_err(|_| Error::Storage)
    }
    /// The packet this notice builds, through the same TS builder.
    fn packet(&self) -> Value {
        build_public_approval_notice(&self.agent, &self.project, self.thread_root.as_deref())
    }
    /// Destination agreement with the live rows: the re-derived authority
    /// must address this notice exactly (stale or caller-influenced state is
    /// refused here, never repaired).
    pub fn matches(&self, authority: &ApprovalRoomAuthority) -> bool {
        authority.server_name == self.server_name
            && authority.project_room_id == self.project_room_id
            && authority.bot_mxid == self.bot_mxid
            && authority.engagement_id == self.engagement
            && authority.agent_name.trim() == self.agent
            && authority.project_id == self.project
    }
    fn validate(&self) -> Result<(), Error> {
        // The engagement id stays an identifier; the NAME is bounded text
        // (unicode letters are legal in an agent name, so `identifier` would
        // wrongly refuse them).
        identifier(&self.engagement, 512).map_err(|_| Error::Storage)?;
        identifier(&self.project, 512).map_err(|_| Error::Storage)?;
        if self.agent.is_empty() || self.agent.chars().count() > MAX_AGENT_NAME {
            return Err(Error::Storage);
        }
        matrix_room(&self.project_room_id, &self.server_name).map_err(|_| Error::Storage)?;
        matrix_room(&self.private_room_id, &self.server_name).map_err(|_| Error::Storage)?;
        matrix_user(&self.bot_mxid, &self.server_name).map_err(|_| Error::Storage)?;
        if let Some(root) = &self.thread_root {
            matrix_event(root).map_err(|_| Error::Storage)?;
        }
        // Room distinctness: the status word never lands in the private
        // approval room, and the two rooms are never the same room.
        if self.project_room_id == self.private_room_id
            || self.body.is_empty()
            || self.body.len() > MAX_NOTICE_BODY
            || self.hash()? != self.digest
        {
            return Err(Error::Storage);
        }
        self.validate_packet(&self.packet())
    }
    /// The packet contract as an exact shape: the three fixed top-level keys
    /// plus the optional thread relation, the status word's five keys, and the
    /// bound identity pair — absence of request material is structural (no such
    /// key can appear), not a filter.
    fn validate_packet(&self, value: &Value) -> Result<(), Error> {
        let object = value.as_object().ok_or(Error::Storage)?;
        let status = value[STATUS_KEY].as_object().ok_or(Error::Storage)?;
        let keys: Vec<&str> = object.keys().map(String::as_str).collect();
        let status_keys: Vec<&str> = status.keys().map(String::as_str).collect();
        let top_level = keys.len() == 3 || (keys.len() == 4 && keys.contains(&"m.relates_to"));
        if !top_level
            || !keys.contains(&"msgtype")
            || !keys.contains(&"body")
            || !keys.contains(&STATUS_KEY)
            || status_keys.len() != 5
            || value["msgtype"] != NOTICE_MSGTYPE
            || value["body"].as_str() != Some(self.body.as_str())
            || value[STATUS_KEY]["kind"] != "status"
            || value[STATUS_KEY]["version"] != 1
            || value[STATUS_KEY]["state"] != STATE
            || value[STATUS_KEY]["agent"] != self.agent
            || value[STATUS_KEY]["project"] != self.project
        {
            return Err(Error::Storage);
        }
        match (&self.thread_root, value.get("m.relates_to")) {
            (None, None) => {}
            (Some(root), Some(actual)) => {
                let expected = json!({
                    "rel_type": "m.thread",
                    "event_id": root,
                    "is_falling_back": true,
                    "m.in_reply_to": {"event_id": root},
                });
                if actual != &expected {
                    return Err(Error::Storage);
                }
            }
            _ => return Err(Error::Storage),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(thread_root: Option<&str>) -> PublicFrozen {
        let mut value = PublicFrozen {
            agent: "agent-one".into(),
            engagement: "agent-one".into(),
            project: "project-one".into(),
            server_name: "hq.test".into(),
            project_room_id: "!project:hq.test".into(),
            private_room_id: "!owner-dm:hq.test".into(),
            bot_mxid: "@bot:hq.test".into(),
            thread_root: thread_root.map(str::to_owned),
            body: "Agent agent-one is waiting for approval from its owner.".into(),
            digest: String::new(),
        };
        value.digest = value.hash().unwrap();
        value.validate().unwrap();
        value
    }

    /// The exact TS packet (bridge-matrix.js:2583-2596): the TS body text and
    /// msgtype, the status detail under the SHARED `com.agentchat.approval`
    /// key, and the `m.thread` relation — no request material in any byte.
    #[test]
    fn public_notice_is_the_ts_packet() {
        let value = notice(Some("$root:hq.test"));
        let content = value.content().unwrap();
        let sent: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            sent,
            json!({
                "msgtype": "com.agentchat.approval.status.v1",
                "body": "Agent agent-one is waiting for approval from its owner.",
                "com.agentchat.approval": {
                    "version": 1,
                    "kind": "status",
                    "agent": "agent-one",
                    "project": "project-one",
                    "state": "waiting_for_owner",
                },
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": "$root:hq.test",
                    "is_falling_back": true,
                    "m.in_reply_to": {"event_id": "$root:hq.test"},
                },
            }),
            "TS packet: {content}"
        );
        for forbidden in [
            "request_id",
            "requestId",
            "digest",
            "tool",
            "tool_name",
            "preview",
            "scope",
            "scope_key",
            "params",
            "command",
            "echo",
        ] {
            assert!(
                !content.contains(forbidden),
                "notice leaked `{forbidden}`: {content}"
            );
        }
    }

    /// TS's own no-thread case: an approval whose `thread_root_event_id` is
    /// absent sends the same packet with NO relation at all (the spread is a
    /// plain condition, bridge-matrix.js:2593).
    #[test]
    fn public_notice_without_a_thread_root_omits_the_relation() {
        let content = notice(None).content().unwrap();
        let sent: Value = serde_json::from_str(&content).unwrap();
        assert!(sent.get("m.relates_to").is_none(), "{content}");
        assert_eq!(sent.as_object().unwrap().len(), 3, "{content}");
        assert_eq!(
            sent["body"],
            "Agent agent-one is waiting for approval from its owner."
        );
    }

    /// The PUT path TS builds (bridge-matrix.js:10823-10825): `m.room.message`
    /// is the EVENT TYPE, never the msgtype; the transaction id is the
    /// content-derived identity.
    #[test]
    fn public_notice_put_path_is_the_ts_path() {
        let value = notice(Some("$root:hq.test"));
        let segments = value.segments("!project:hq.test");
        assert_eq!(
            segments,
            vec![
                "_matrix",
                "client",
                "v3",
                "rooms",
                "!project:hq.test",
                "send",
                "m.room.message",
                &value.transaction(),
            ]
        );
        assert!(segments[7].starts_with("approval_status_"));
        assert_eq!(segments[7].len(), "approval_status_".len() + 64);
    }
}
