use super::*;
use super::earlier_devices::EarlierDevices;
use matrix_sdk::{deserialized_responses::{DeviceLinkProblem, TimelineEvent}, room::MessagesOptions};
use serde_json::{Value, json};

pub(in crate::console) struct HistoryBatch {
    pub events: Vec<Value>,
    pub next: Option<String>,
}

impl AgentDevice {
    pub(in crate::console) async fn history_request(&self, room: &str, event: &str) -> Result<Option<Value>, RuntimeError> {
        let room_id = OwnedRoomId::try_from(room).map_err(|_| RuntimeError::Request)?;
        let event_id = OwnedEventId::try_from(event).map_err(|_| RuntimeError::Request)?;
        let room = self.client.get_room(&room_id).ok_or(RuntimeError::Transport)?;
        let event = room.event(&event_id, None).await.map_err(|_| RuntimeError::Transport)?;
        if event.kind.parse_event_id().as_ref() != Some(&event_id) { return Err(RuntimeError::Transport); }
        Ok(history_event(&event))
    }
    /// The SDK enforces this Agent's membership/history visibility and decrypts
    /// with its own keys. The caller pins the room and anchor from admitted work.
    pub(in crate::console) async fn history_start(
        &self,
        room: &str,
        anchor: &str,
    ) -> Result<Option<String>, RuntimeError> {
        let room_id = OwnedRoomId::try_from(room).map_err(|_| RuntimeError::Request)?;
        let event_id = OwnedEventId::try_from(anchor).map_err(|_| RuntimeError::Request)?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or(RuntimeError::Transport)?;
        let context = room
            .event_with_context(&event_id, false, 0u32.into(), None)
            .await
            .map_err(|_| RuntimeError::Transport)?;
        if context
            .event
            .as_ref()
            .and_then(|e| e.kind.parse_event_id())
            .as_ref()
            != Some(&event_id)
        {
            return Err(RuntimeError::Transport);
        }
        Ok(context.prev_batch_token)
    }

    /// The first messages after `anchor`, oldest first, read under the same
    /// membership, visibility and decryption as the pages before it.
    pub(in crate::console) async fn history_after(
        &self,
        room: &str,
        anchor: &str,
    ) -> Result<HistoryBatch, RuntimeError> {
        let room_id = OwnedRoomId::try_from(room).map_err(|_| RuntimeError::Request)?;
        let event_id = OwnedEventId::try_from(anchor).map_err(|_| RuntimeError::Request)?;
        let room = self.client.get_room(&room_id).ok_or(RuntimeError::Transport)?;
        let context = room
            .event_with_context(&event_id, false, 0u32.into(), None)
            .await
            .map_err(|_| RuntimeError::Transport)?;
        if context.event.as_ref().and_then(|e| e.kind.parse_event_id()).as_ref() != Some(&event_id) {
            return Err(RuntimeError::Transport);
        }
        let Some(from) = context.next_batch_token else {
            return Ok(HistoryBatch { events: Vec::new(), next: None });
        };
        let mut options = MessagesOptions::forward().from(from.as_str());
        options.limit = 50u32.into();
        let page = room.messages(options).await.map_err(|_| RuntimeError::Transport)?;
        // A full page may have more after it; a shorter one reached the end.
        let more = page.chunk.len() >= 50;
        Ok(HistoryBatch {
            events: page.chunk.iter().filter_map(history_event).collect(),
            next: more.then_some(page.end).flatten(),
        })
    }

    pub(in crate::console) async fn history_page(
        &self,
        room: &str,
        from: &str,
    ) -> Result<HistoryBatch, RuntimeError> {
        let room_id = OwnedRoomId::try_from(room).map_err(|_| RuntimeError::Request)?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or(RuntimeError::Transport)?;
        let mut options = MessagesOptions::backward().from(from);
        options.limit = 50u32.into();
        let page = room
            .messages(options)
            .await
            .map_err(|_| RuntimeError::Transport)?;
        if page.end.as_deref() == Some(from) {
            return Err(RuntimeError::Transport);
        }
        Ok(HistoryBatch {
            events: page.chunk.iter().filter_map(history_event).collect(),
            next: page.end,
        })
    }
}

/// One event from the expected sender, as the check for an earlier send of a
/// report sees it.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::console) struct ReportTrace {
    pub event_id: String,
    /// `None` when this device cannot read it as an authentic, unredacted
    /// event (undecryptable, an unverifiable sending device, or redacted): it
    /// may be the report.
    pub readable: Option<ReportContent>,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::console) struct ReportContent {
    /// The Hagency report ID of an `m.room.message`, if it carries one.
    pub report_id: Option<String>,
    /// SHA-256 of its `body`, the digest the outbox keeps for a report's text.
    pub body_digest: Option<String>,
}
#[derive(Debug, Default)]
pub(in crate::console) struct ReportScan {
    /// The sender's events, newest first: messages that carry a report ID,
    /// and events this device cannot read.
    pub events: Vec<ReportTrace>,
    /// The scan reached an event from the sender's homeserver older than the
    /// start time, or the start of the history this Agent can see.
    pub complete: bool,
}

#[derive(Clone, Copy)]
enum TraceKind {
    Undecryptable,
    Decrypted { authentic: bool },
    Plain,
    /// Undecryptable here, read with a session that one of this Agent's
    /// earlier devices on this computer started itself.
    EarlierDevice,
}

fn trace_kind(event: &TimelineEvent, raw: &Value) -> TraceKind {
    match &event.kind {
        TimelineEventKind::UnableToDecrypt { .. } => TraceKind::Undecryptable,
        TimelineEventKind::Decrypted(d) => TraceKind::Decrypted {
            authentic: d.encryption_info.sender.as_str() == raw["sender"].as_str().unwrap_or_default()
                && verification_issue(&d.encryption_info.verification_state).is_none(),
        },
        TimelineEventKind::PlainText { .. } => TraceKind::Plain,
    }
}

fn report_trace(raw: &Value, kind: TraceKind) -> Option<ReportTrace> {
    let event_id = matrix_id(&raw["event_id"], '$')?.to_owned();
    let readable = match kind {
        TraceKind::Undecryptable | TraceKind::Decrypted { authentic: false } => false,
        TraceKind::Decrypted { authentic: true } | TraceKind::Plain | TraceKind::EarlierDevice => {
            raw["unsigned"].get("redacted_because").is_none()
        }
    };
    if !readable {
        return Some(ReportTrace { event_id, readable: None });
    }
    let message = raw["type"] == "m.room.message";
    let field = |key: &str| if message { raw["content"][key].as_str() } else { None };
    Some(ReportTrace {
        event_id,
        readable: Some(ReportContent {
            report_id: field("org.hagency.report_id").map(str::to_owned),
            body_digest: field("body").map(|body| format!("{:x}", Sha256::digest(body.as_bytes()))),
        }),
    })
}

/// One page, newest first: keeps `sender`'s messages that carry a report ID
/// and its events this device cannot read, and says whether the page reached
/// an event from the sender's homeserver older than `since_ms`. That server
/// stamps the report with the same clock, and a report sent after the
/// attempt began comes after every event the server had created before it.
/// Another server's timestamps prove nothing, so they never end a scan.
fn scan_page(
    page: impl IntoIterator<Item = (Value, TraceKind)>,
    sender: &OwnedUserId,
    since_ms: u64,
    events: &mut Vec<ReportTrace>,
) -> bool {
    let mut reached = false;
    for (raw, kind) in page {
        let Some(from) = matrix_id(&raw["sender"], '@').and_then(|s| OwnedUserId::try_from(s).ok()) else { continue };
        if from.server_name() == sender.server_name()
            && raw["origin_server_ts"].as_u64().is_some_and(|ts| ts < since_ms)
        {
            reached = true;
        }
        if from != *sender { continue; }
        let Some(trace) = report_trace(&raw, kind) else { continue };
        if trace.readable.as_ref().is_none_or(|content| content.report_id.is_some()) {
            events.push(trace);
        }
    }
    reached
}

/// At most this many pages of 50 events are read for one check.
const REPORT_SCAN_PAGES: usize = 6;

impl AgentDevice {
    /// Look back through `room` with this device's keys, newest first, for
    /// what became of an earlier send of a report by `sender`. The sender's
    /// events this device cannot decrypt are read with the stores of this
    /// Agent's earlier devices on this computer, when one of them sent it.
    /// The scan ends at that homeserver's events from before `since_ms`.
    pub(in crate::console) async fn report_scan(
        &self,
        room: &str,
        sender: &str,
        since_ms: u64,
    ) -> Result<ReportScan, RuntimeError> {
        let room_id = OwnedRoomId::try_from(room).map_err(|_| RuntimeError::Request)?;
        let sender = OwnedUserId::try_from(sender).map_err(|_| RuntimeError::Request)?;
        let room = self.client.get_room(&room_id).ok_or(RuntimeError::Transport)?;
        let mut scan = ReportScan::default();
        let mut from: Option<String> = None;
        // Opened at the sender's first event this device cannot decrypt.
        let mut earlier: Option<EarlierDevices> = None;
        let mut read_earlier = 0usize;
        for _ in 0..REPORT_SCAN_PAGES {
            let mut options = MessagesOptions::backward().from(from.as_deref());
            options.limit = 50u32.into();
            let page = tokio::time::timeout(Duration::from_secs(10), room.messages(options))
                .await
                .map_err(|_| RuntimeError::Transport)?
                .map_err(|_| RuntimeError::Transport)?;
            let mut events = Vec::with_capacity(page.chunk.len());
            for event in &page.chunk {
                let Ok(raw) = serde_json::from_str::<Value>(event.kind.raw().json().get()) else { continue };
                let kind = trace_kind(event, &raw);
                if matches!(kind, TraceKind::Undecryptable) && raw["sender"] == sender.as_str() {
                    if earlier.is_none() {
                        earlier = Some(self.earlier_devices().await);
                    }
                    let plain = match &earlier {
                        Some(devices) => devices.read(&room_id, &raw).await,
                        None => None,
                    };
                    if let Some(plain) = plain {
                        read_earlier += 1;
                        events.push((plain, TraceKind::EarlierDevice));
                        continue;
                    }
                }
                events.push((raw, kind));
            }
            if scan_page(events, &sender, since_ms, &mut scan.events) {
                scan.complete = true;
                break;
            }
            match page.end {
                // The start of the history this Agent can see.
                None => {
                    scan.complete = true;
                    break;
                }
                // A server that hands back the same token would never finish.
                Some(end) if from.as_deref() == Some(end.as_str()) => break,
                Some(end) => from = Some(end),
            }
        }
        if let Some(devices) = &earlier {
            tracing::info!(stage = "report_scan_earlier_devices", stores = devices.len(), read = read_earlier,
                complete = scan.complete, "Read the Agent's own earlier messages with its earlier devices' keys");
        }
        Ok(scan)
    }

    /// This Agent's earlier devices on this computer (see `earlier_devices.rs`).
    async fn earlier_devices(&self) -> EarlierDevices {
        let (Some(user), Some(device)) = (self.client.user_id(), self.client.device_id()) else {
            return EarlierDevices::none();
        };
        let Ok(passphrase) = store_passphrase(&self.root) else { return EarlierDevices::none() };
        EarlierDevices::open(&self.root, &passphrase, user, device).await
    }
}

pub(super) fn history_event(event: &TimelineEvent) -> Option<Value> {
    let raw: Value = serde_json::from_str(event.kind.raw().json().get()).ok()?;
    let unavailable_reason = match &event.kind {
        TimelineEventKind::UnableToDecrypt { .. } => Some("unable_to_decrypt"),
        TimelineEventKind::Decrypted(d) => {
            if d.encryption_info.sender.as_str() != raw["sender"].as_str().unwrap_or_default() {
                Some("sender_mismatch")
            } else {
                verification_issue(&d.encryption_info.verification_state)
            }
        }
        TimelineEventKind::PlainText { .. } => None,
    };
    let mut record = history_record(&raw, unavailable_reason.is_some())?;
    if let Some(reason) = unavailable_reason {
        record["unavailableReason"] = reason.into();
    }
    Some(record)
}

fn verification_issue(state: &VerificationState) -> Option<&'static str> {
    match state {
        VerificationState::Unverified(level) => match level {
            VerificationLevel::None(DeviceLinkProblem::MissingDevice) => Some("sender_device_missing"),
            VerificationLevel::None(DeviceLinkProblem::InsecureSource) => Some("insecure_key_source"),
            VerificationLevel::MismatchedSender => Some("sender_mismatch"),
            VerificationLevel::VerificationViolation => Some("identity_verification_changed"),
            VerificationLevel::UnverifiedIdentity | VerificationLevel::UnsignedDevice => None,
        },
        VerificationState::Verified => None,
    }
}

/// Spec limits for the identifiers a record may carry; `msgtype` values are
/// short namespaced names. Anything else is room-member-controlled JSON.
const MAX_MATRIX_ID_BYTES: usize = 255;
const MAX_MSGTYPE_BYTES: usize = 64;

fn matrix_id(value: &Value, prefix: char) -> Option<&str> {
    value
        .as_str()
        .filter(|s| {
            s.len() > 1 && s.len() <= MAX_MATRIX_ID_BYTES && !s.chars().any(char::is_control)
                && match prefix {
                    '$' => OwnedEventId::try_from(*s).is_ok(),
                    '@' => OwnedUserId::try_from(*s).is_ok(),
                    _ => false,
                }
        })
}

/// Every field is validated and bounded here, so one crafted event (an
/// oversized `msgtype`, a non-string `body`, a huge relation id) can neither
/// exceed a history page nor stall pagination for everything older than it.
/// Bodies stay unbounded strings: the reader splits them into fragments.
pub(in crate::console) fn history_record(raw: &Value, unavailable: bool) -> Option<Value> {
    let id = matrix_id(&raw["event_id"], '$')?;
    let sender = matrix_id(&raw["sender"], '@')?;
    let mut record = json!({"eventId":id,"sender":sender});
    if let Some(ts) = raw["origin_server_ts"].as_u64() {
        record["timestamp"] = ts.into();
    }
    if unavailable {
        record["status"] = "unavailable_encrypted_message".into();
        return Some(record);
    }
    if raw["unsigned"].get("redacted_because").is_some() {
        record["status"] = "redacted".into();
        return Some(record);
    }
    if raw["type"] == "m.room.redaction" {
        // Room versions before 11 carry `redacts` at the top level.
        let target = matrix_id(&raw["content"]["redacts"], '$')
            .or_else(|| matrix_id(&raw["redacts"], '$'))?;
        record["status"] = "redaction".into();
        record["target"] = target.into();
        return Some(record);
    }
    if raw["type"] != "m.room.message" {
        return None;
    }
    let content = &raw["content"];
    if let Some(kind) = content["msgtype"].as_str().filter(|s| s.len() <= MAX_MSGTYPE_BYTES) {
        record["messageType"] = kind.into();
    }
    let relation = &content["m.relates_to"];
    if relation["rel_type"] == "m.replace" {
        // An edit without a well-formed target or replacement text is not an
        // edit of anything; it is dropped rather than blanking the original.
        let target = matrix_id(&relation["event_id"], '$')?;
        let body = content["m.new_content"]["body"].as_str()?;
        record["status"] = "edit".into();
        record["target"] = target.into();
        record["body"] = body.into();
        return Some(record);
    }
    if let Some(body) = content["body"].as_str() {
        record["body"] = body.into();
    } else {
        record["status"] = "unavailable_invalid_message".into();
        record["unavailableReason"] = "body_is_not_text".into();
    }
    if let Some(metadata) = super::documents::metadata(content)
    {
        // Only a display descriptor reaches the model. URL, key, IV and hash
        // remain in the authenticated event on this Agent's own device.
        record["attachment"] = metadata;
    }
    if let Some(root) = matrix_id(&relation["event_id"], '$') {
        record["relatedEventId"] = root.into();
    }
    Some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, content: Value) -> Value {
        json!({"type":"m.room.message","event_id":id,"sender":"@member:example.test","origin_server_ts":1700000000000u64,"content":content})
    }

    fn report(id: &str, sender: &str, ts: u64, report_id: Option<&str>) -> Value {
        let mut content = json!({"msgtype":"m.text","body":"done"});
        if let Some(report_id) = report_id {
            content["org.hagency.report_id"] = report_id.into();
        }
        json!({"type":"m.room.message","event_id":id,"sender":sender,"origin_server_ts":ts,"content":content})
    }

    #[test]
    fn report_traces_read_report_ids_only_from_authentic_unredacted_messages() {
        let raw = report("$report", "@agent:example.test", 1_700_000_000_000, Some("id"));
        let read = report_trace(&raw, TraceKind::Decrypted { authentic: true }).unwrap();
        assert_eq!(read.event_id, "$report");
        assert_eq!(read.readable, Some(ReportContent {
            report_id: Some("id".into()),
            body_digest: Some(format!("{:x}", Sha256::digest(b"done"))),
        }));
        assert_eq!(report_trace(&raw, TraceKind::Plain).unwrap(), read);
        assert_eq!(report_trace(&raw, TraceKind::EarlierDevice).unwrap(), read);
        // Anything this device cannot read as an authentic event may be the report.
        assert_eq!(report_trace(&raw, TraceKind::Decrypted { authentic: false }).unwrap().readable, None);
        assert_eq!(report_trace(&raw, TraceKind::Undecryptable).unwrap().readable, None);
        let mut redacted = raw.clone();
        redacted["unsigned"] = json!({"redacted_because":{}});
        assert_eq!(report_trace(&redacted, TraceKind::Plain).unwrap().readable, None);
        // Only room messages carry a report ID.
        let mut reaction = raw.clone();
        reaction["type"] = "m.reaction".into();
        assert_eq!(report_trace(&reaction, TraceKind::Plain).unwrap().readable.unwrap().report_id, None);
        let mut nameless = raw;
        nameless["event_id"] = "report".into();
        assert!(report_trace(&nameless, TraceKind::Plain).is_none());
    }

    #[test]
    fn report_scan_pages_keep_only_the_sender_and_end_on_its_own_server_clock() {
        let agent = OwnedUserId::try_from("@agent:example.test").unwrap();
        let since = 1_000_000;
        let page = vec![
            (report("$new", "@agent:example.test", since + 5, Some("a")), TraceKind::Decrypted { authentic: true }),
            // Another sender's copy of a report ID is not the Agent's event.
            (report("$copy", "@member:example.test", since + 4, Some("a")), TraceKind::Plain),
            // The Agent's ordinary messages carry no report ID and cannot be the report.
            (report("$answer", "@agent:example.test", since + 3, None), TraceKind::Plain),
            (report("$sealed", "@agent:example.test", since + 2, None), TraceKind::Undecryptable),
            // A remote server's old timestamp does not end the scan.
            (report("$remote", "@peer:remote.test", 1, None), TraceKind::Plain),
        ];
        let mut events = Vec::new();
        assert!(!scan_page(page, &agent, since, &mut events));
        assert_eq!(events.iter().map(|e| e.event_id.as_str()).collect::<Vec<_>>(), ["$new", "$sealed"]);
        assert_eq!(events[1].readable, None);
        // Any event from the Agent's homeserver from before the attempt does.
        let older = vec![(report("$old", "@member:example.test", since - 1, None), TraceKind::Plain)];
        assert!(scan_page(older, &agent, since, &mut events));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn history_file_records_expose_only_metadata_and_never_unavailable_attachment_keys() {
        let raw = event("$file", json!({"msgtype":"m.file","body":"设计.pdf","file":{"url":"mxc://example.test/secret","key":"private-key","iv":"private-iv"},"info":{"mimetype":"application/pdf","size":100}}));
        let record = history_record(&raw, false).unwrap();
        assert_eq!(record["attachment"]["filename"], "设计.pdf");
        assert_eq!(record["attachment"]["status"], "not_read");
        assert!(!record.to_string().contains("private"));
        assert!(!record.to_string().contains("mxc://"));
        assert!(history_record(&raw, true).unwrap().get("attachment").is_none());
        let mut redacted = raw.clone();
        redacted["unsigned"] = json!({"redacted_because":{}});
        assert!(history_record(&redacted, false).unwrap().get("attachment").is_none());
    }

    #[test]
    fn history_record_distinguishes_unknown_devices_from_unverified_identities() {
        assert_eq!(verification_issue(&VerificationState::Verified), None);
        for level in [VerificationLevel::UnverifiedIdentity, VerificationLevel::UnsignedDevice] {
            assert_eq!(verification_issue(&VerificationState::Unverified(level)), None);
        }
        for (level, reason) in [
            (VerificationLevel::None(DeviceLinkProblem::MissingDevice), "sender_device_missing"),
            (VerificationLevel::None(DeviceLinkProblem::InsecureSource), "insecure_key_source"),
            (VerificationLevel::MismatchedSender, "sender_mismatch"),
            (VerificationLevel::VerificationViolation, "identity_verification_changed"),
        ] {
            assert_eq!(verification_issue(&VerificationState::Unverified(level)), Some(reason));
        }
        for sender in ["@", "@alice", "@alice:"] {
            let mut raw = event("$message", json!({"body":"hello"}));
            raw["sender"] = sender.into();
            assert!(history_record(&raw, false).is_none(), "{sender}");
        }
    }

    #[test]
    fn history_record_drops_oversized_and_mistyped_fields_but_keeps_long_bodies() {
        let record = history_record(
            &event("$big", json!({"msgtype":"x".repeat(20 * 1024),"body":{"not":"text"},"m.relates_to":{"rel_type":"m.thread","event_id":format!("${}", "r".repeat(400))}})),
            false,
        )
        .unwrap();
        assert!(record.get("messageType").is_none());
        assert!(record.get("body").is_none());
        assert_eq!(record["status"], "unavailable_invalid_message");
        assert!(record.get("relatedEventId").is_none());
        assert!(record.to_string().len() < 256);
        let long = "群".repeat(50_000);
        let record = history_record(&event("$long", json!({"msgtype":"m.text","body":long,"m.relates_to":{"rel_type":"m.thread","event_id":"$root"}})), false).unwrap();
        assert_eq!(record["body"].as_str().unwrap().len(), long.len());
        assert_eq!((record["messageType"].as_str(), record["relatedEventId"].as_str()), (Some("m.text"), Some("$root")));
        assert_eq!(record["timestamp"], 1700000000000u64);
    }

    #[test]
    fn history_record_requires_well_formed_edits_and_redaction_targets() {
        assert!(history_record(&event("$edit", json!({"msgtype":"m.text","body":"* x","m.new_content":{"body":{"no":"text"}},"m.relates_to":{"rel_type":"m.replace","event_id":"$orig"}})), false).is_none());
        let edit = history_record(&event("$edit", json!({"msgtype":"m.text","body":"* fixed","m.new_content":{"msgtype":"m.text","body":"fixed"},"m.relates_to":{"rel_type":"m.replace","event_id":"$orig"}})), false).unwrap();
        assert_eq!((edit["status"].as_str(), edit["target"].as_str(), edit["body"].as_str()), (Some("edit"), Some("$orig"), Some("fixed")));
        let mut redaction = event("$red", json!({"redacts":"$gone","reason":"spam"}));
        redaction["type"] = "m.room.redaction".into();
        let record = history_record(&redaction, false).unwrap();
        assert_eq!((record["status"].as_str(), record["target"].as_str()), (Some("redaction"), Some("$gone")));
        redaction["content"] = json!({"redacts":"not-an-event"});
        assert!(history_record(&redaction, false).is_none());
        let mut bad_sender = event("$e", json!({"msgtype":"m.text","body":"hi"}));
        bad_sender["sender"] = "mallory".into();
        assert!(history_record(&bad_sender, false).is_none());
        let utd = history_record(&json!({"type":"m.room.encrypted","event_id":"$utd","sender":"@member:example.test","content":{"msgtype":"x".repeat(9000)}}), true).unwrap();
        assert_eq!(utd["status"], "unavailable_encrypted_message");
        assert!(utd.to_string().len() < 256);
    }
}
