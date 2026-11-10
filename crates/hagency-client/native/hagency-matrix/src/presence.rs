//! Agent liveness signals for a human waiting on an agent.
//!
//! Port of the retained product's `setAgentTyping` / `ackAgentReceipt` /
//! `beginAgentWork` / `endAgentWork` / `ensureTypingRefresh`
//! (`bridge-matrix.js:354-369` constants, `:10527-10566` typing,
//! `:10568-10594` 👀 reaction, `:10603-10668` lifecycle). Both signals are
//! sent as the agent itself, with the agent's own credential — the agent is
//! who must appear busy, not the bridge.
//!
//! Neither signal ever fails a delivered message: every call resolves to
//! `false` instead of an error, exactly as the TS `try/catch` arms do.
use crate::collector::Inner;
use crate::{CancellationToken, Collector};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What the homeserver is told. The homeserver expires the notification on its
/// own if nothing refreshes it, so a crash cannot leave the agent typing.
pub const AGENT_TYPING_TIMEOUT_MS: u64 = 45_000;
/// How often the notification is re-asserted while work is outstanding.
pub const AGENT_TYPING_REFRESH_MS: u64 = 30_000;
/// Past this the notification is allowed to lapse. TWO MINUTES, not twenty: a
/// typing indicator is only honest for a short window, and the 👀 reaction is
/// already the durable record that the message was received.
pub const AGENT_TYPING_MAX_MS: u64 = 2 * 60_000;
/// 👀 — "seen, and being worked on". One event, no work product.
pub const AGENT_ACK_REACTION: &str = "\u{1F440}";

/// The exact wire form of `setAgentTyping` (`bridge-matrix.js:10527`): the
/// fixed path segments and the JSON body. Pure, so the TS-visible shape is
/// asserted directly rather than only through a live fixture.
pub fn typing_request(user_mxid: &str, room_id: &str, typing: bool) -> (Vec<String>, String) {
    let segments = vec![
        "_matrix".to_owned(),
        "client".to_owned(),
        "v3".to_owned(),
        "rooms".to_owned(),
        room_id.to_owned(),
        "typing".to_owned(),
        user_mxid.to_owned(),
    ];
    let value = if typing {
        json!({"typing": true, "timeout": AGENT_TYPING_TIMEOUT_MS})
    } else {
        json!({"typing": false})
    };
    (
        segments,
        serde_json::to_string(&value).expect("fixed typing body serializes"),
    )
}

/// The exact wire form of `ackAgentReceipt` (`bridge-matrix.js:10568`): the
/// `m.annotation` reaction whose transaction id is derived from the event, so
/// a retry of the same handoff cannot double-react.
pub fn ack_request(agent: &str, room_id: &str, event_id: &str) -> (Vec<String>, String) {
    let digest = Sha256::digest(format!("ack:{event_id}:{agent}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let txn_id = format!("ack_{}", &hex[..24]);
    let segments = vec![
        "_matrix".to_owned(),
        "client".to_owned(),
        "v3".to_owned(),
        "rooms".to_owned(),
        room_id.to_owned(),
        "send".to_owned(),
        "m.reaction".to_owned(),
        txn_id,
    ];
    let body = json!({
        "m.relates_to": {
            "rel_type": "m.annotation",
            "event_id": event_id,
            "key": AGENT_ACK_REACTION,
        }
    })
    .to_string();
    (segments, body)
}

impl Inner {
    /// `PUT /_matrix/client/v3/rooms/{room}/typing/{user}`.
    ///
    /// Ephemeral by design: `m.typing` is not a room event, so this adds
    /// nothing to history and costs no rate-limit budget against message sends.
    pub(crate) async fn presence_typing(
        &self,
        room_id: &str,
        typing: bool,
        cancel: &CancellationToken,
    ) -> bool {
        if room_id.is_empty() {
            return false;
        }
        let user = self.config.identity.transport.sender_mxid.as_str();
        let (segments, body) = typing_request(user, room_id, typing);
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        matches!(self.http.put(&segments, body, cancel).await, Ok(r) if r.status == 200)
    }

    /// `PUT /_matrix/client/v3/rooms/{room}/send/m.reaction/{txn}`.
    ///
    /// One event, and it answers the question a blank room cannot: was this
    /// received, and by whom. Deliberately separate from typing — a delivery
    /// that failed at the backend must not produce an acknowledgement.
    pub(crate) async fn presence_ack(
        &self,
        room_id: &str,
        event_id: &str,
        cancel: &CancellationToken,
    ) -> bool {
        if room_id.is_empty() || event_id.is_empty() {
            return false;
        }
        // The agent is identified by its Matrix account here, which is the one
        // stable identity a `HostConfig` carries for it (`agentName` in TS).
        let agent = self.config.identity.transport.sender_mxid.as_str();
        let (segments, body) = ack_request(agent, room_id, event_id);
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        matches!(self.http.put(&segments, body, cancel).await, Ok(r) if r.status == 200)
    }
}

impl Collector {
    /// React to the human's own message the moment it has been handed to the
    /// agent. Never awaited by a caller that must not be delayed; resolves to
    /// `false` rather than an error.
    pub async fn ack_agent_receipt(&self, room_id: &str, event_id: &str) -> bool {
        self.inner
            .presence_ack(room_id, event_id, &CancellationToken::new())
            .await
    }

    /// `typing: true` asserts the notification; `typing: false` ends it.
    pub async fn set_agent_typing(&self, room_id: &str, typing: bool) -> bool {
        self.inner
            .presence_typing(room_id, typing, &CancellationToken::new())
            .await
    }

    /// The room and the addressed trigger event a selection is *about to*
    /// accept. Read BEFORE selection: selection binds the input to a dispatch,
    /// after which it is no longer an unprocessed trigger. Never fatal — a
    /// session this host cannot resolve simply has no liveness signals.
    pub async fn agent_work_target(&self, session_id: &str) -> Option<(String, String)> {
        let route = self
            .inner
            .domain
            .matrix_intake_route(session_id.to_owned())
            .await
            .ok()?;
        let items = self
            .inner
            .domain
            .inbox(session_id.to_owned(), 0, 100, None)
            .await
            .ok()?;
        let trigger = items
            .into_iter()
            .filter(|item| item.wake)
            .min_by_key(|item| item.message.sequence)?;
        Some((route.room_id, trigger.message.event_id))
    }

    /// Handed off to the agent: acknowledge, and start appearing busy.
    ///
    /// The returned [`AgentWork`] owns the 30 s refresh and the 2 minute cap;
    /// ending it (or dropping it) stops the indicator for that room.
    pub fn begin_agent_work(&self, room_id: &str, event_id: &str) -> AgentWork {
        let inner = self.inner.clone();
        let stop = CancellationToken::new();

        // Acknowledge and start appearing busy. Neither is awaited: a failed
        // typing notification must not fail a delivered message.
        let ack_inner = inner.clone();
        let ack_stop = stop.child_token();
        let ack_room = room_id.to_owned();
        let ack_event = event_id.to_owned();
        tokio::spawn(async move {
            ack_inner
                .presence_ack(&ack_room, &ack_event, &ack_stop)
                .await;
            ack_inner.presence_typing(&ack_room, true, &ack_stop).await;
        });

        // Re-assert while work is outstanding, and STOP after the cap.
        let refresh_inner = inner.clone();
        let refresh_room = room_id.to_owned();
        let refresh_stop = stop.child_token();
        let task = tokio::spawn(async move {
            let started = Instant::now();
            loop {
                tokio::select! {
                    _ = refresh_stop.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(AGENT_TYPING_REFRESH_MS)) => {}
                }
                if started.elapsed().as_millis() as u64 > AGENT_TYPING_MAX_MS {
                    // Past the cap the notification is allowed to lapse: an
                    // agent silent that long may be stuck, and continuing to
                    // claim it is typing would be a lie the room cannot check.
                    refresh_inner
                        .presence_typing(&refresh_room, false, &refresh_stop)
                        .await;
                    break;
                }
                refresh_inner
                    .presence_typing(&refresh_room, true, &refresh_stop)
                    .await;
            }
        });

        AgentWork {
            inner,
            room_id: room_id.to_owned(),
            stop,
            task: Some(task),
        }
    }
}

/// One outstanding agent wait for one room (`agentWork` entries in TS).
pub struct AgentWork {
    inner: Arc<Inner>,
    room_id: String,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl AgentWork {
    /// The agent spoke, so it is no longer working for this room
    /// (`endAgentWork`, bridge-matrix.js:10634).
    ///
    /// The withdrawal below is the LAST thing this wait ever sends: the `Drop`
    /// that follows this call sends nothing.
    pub async fn end(mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.inner
            .presence_typing(&self.room_id, false, &CancellationToken::new())
            .await;
    }
}

impl Drop for AgentWork {
    fn drop(&mut self) {
        // A wait the host abandoned (a failure, a cancel, a superseded plan)
        // stops refreshing and lets the homeserver expire the notification by
        // itself. That IS the retained product's stated failure mode: the
        // timeout exists so a crash "cannot leave the agent typing forever …
        // which makes the failure mode silence rather than a permanent false
        // 'still working'" (`setAgentTyping`, bridge-matrix.js:10548-10552).
        // Nothing is sent here, because TS clears typing in exactly one place —
        // the agent's own send (`:10884-10885`), which `end()` performs.
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
