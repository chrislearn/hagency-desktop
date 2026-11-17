//! Immutable, encrypted local custody for messages emitted during a run.
//! A Matrix transaction is tied to the run and native message identity. A
//! lost send response can therefore be retried without producing a second
//! room event, and no provider call needs to be repeated.
use crate::agent_run_controls::Card;
use hagency_runtime::live_text::{MessageError, MessageSink};
use matrix_sdk_store_encryption::StoreCipher;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

const MAX_RECORDS: usize = 16_384;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
// JSON escaping and encrypted serialization can exceed the original UTF-8.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
static KEY_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Default::default);

fn hash(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(value.as_ref()))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn report_content(room: &str, thread: &str, text: &str, report_id: &str) -> Result<serde_json::Value, ()> {
    if !valid_digest(report_id) { return Err(()); }
    let mut content = super::e2ee::native_message_content(room, thread, text).map_err(|_| ())?;
    content.as_object_mut().ok_or(())?
        .insert("org.hagency.report_id".into(), report_id.into());
    Ok(content)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    hagency_store::private::open(path, false)
        .map_err(|_| ())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > limit {
        return Err(());
    }
    Ok(bytes)
}
fn has_saved_records(root: &Path) -> Result<bool, ()> {
    for entry in std::fs::read_dir(root).map_err(|_| ())? {
        let entry = entry.map_err(|_| ())?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("run_"))
        {
            let directory = entry.path();
            // The private-state check refuses a symlink or a non-private directory.
            hagency_store::private::directory(&directory).map_err(|_| ())?;
            if std::fs::read_dir(directory)
                .map_err(|_| ())?
                .next()
                .is_some()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn cipher(root: &Path) -> Result<StoreCipher, ()> {
    let _guard = KEY_LOCK.lock().map_err(|_| ())?;
    let key_path = root.join("store-key");
    let cipher_path = root.join("cipher-key");
    let mut key = match hagency_store::private::read_secret(&key_path) {
        Ok(value) => value,
        Err(hagency_store::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            if cipher_path.exists() || has_saved_records(root)? {
                return Err(());
            }
            let mut value = vec![0u8; 32];
            getrandom::fill(&mut value).map_err(|_| ())?;
            hagency_store::private::write_new(&key_path, &value).map_err(|_| ())?;
            value
        }
        Err(_) => return Err(()),
    };
    if key.len() != 32 {
        key.fill(0);
        return Err(());
    }
    let result = if cipher_path.exists() {
        StoreCipher::import_with_key(&key, &read(&cipher_path, 1024)?).map_err(|_| ())
    } else {
        if has_saved_records(root)? {
            key.fill(0);
            return Err(());
        }
        let cipher = StoreCipher::new().map_err(|_| ())?;
        let encrypted = cipher.export_with_key(&key).map_err(|_| ())?;
        hagency_store::private::write_new(&cipher_path, &encrypted).map_err(|_| ())?;
        Ok(cipher)
    };
    key.fill(0);
    result
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    run: String,
    sequence: u64,
    id: String,
    digest: String,
    text: Option<String>,
    event: Option<String>,
    /// Added only to v2. Older binaries reject, rather than resend, v2 rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report_id: Option<String>,
    /// The Matrix account, device and room encryption state observed before
    /// the first attempt. An uncertain send cannot be retried from another
    /// device or after a change in the observed room encryption state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attempt: Option<Attempt>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    device: String,
    encrypted: bool,
}
struct State {
    records: BTreeMap<u64, Record>,
    ids: BTreeMap<String, u64>,
    bytes: usize,
}
/// The room destination and stable run identity do not grant run-control
/// authority. In particular, an Agent requester may receive messages even
/// though it cannot be offered a human-only control card.
#[derive(Clone)]
pub(super) struct Route {
    id: String,
    room_id: String,
    thread_root: String,
}
impl Route {
    pub(super) fn personal(
        dispatch: &hagency_agent_local::inbox::Dispatch,
        execution: &str,
    ) -> Option<Self> {
        fn key(value: &str) -> bool {
            !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
        }
        let scope = dispatch.scope();
        if [
            &scope.agent,
            &scope.binding,
            &scope.requester,
            &dispatch.id,
            &dispatch.event_id,
        ]
        .into_iter()
        .any(|value| !key(value))
            || !key(execution)
            || !dispatch.event_id.starts_with('$')
            || matrix_sdk::ruma::OwnedUserId::try_from(scope.requester.as_str()).is_err()
            || matrix_sdk::ruma::OwnedRoomId::try_from(scope.room.as_str()).is_err()
            || !key(&scope.thread)
            || !(scope.thread.starts_with('$') || scope.thread == scope.room)
        {
            return None;
        }
        let encoded = serde_json::to_vec(&(scope, &dispatch.id, execution)).ok()?;
        Some(Self {
            id: format!("run_{}", hash(encoded)),
            room_id: dispatch.room_id.clone(),
            thread_root: dispatch.thread_root.clone(),
        })
    }
    fn card(card: Card) -> Result<Self, ()> {
        card.validate().map_err(|_| ())?;
        Ok(Self {
            id: card.id,
            room_id: card.room_id,
            thread_root: card.thread_root,
        })
    }
}
pub(in crate::console) struct Outbox {
    route: Route,
    directory: PathBuf,
    cipher: StoreCipher,
    state: Mutex<State>,
    lock_path: PathBuf,
}
pub(in crate::console) struct Pending {
    pub sequence: u64,
    pub txn: String,
    pub text: String,
    report_id: Option<String>,
}
impl Pending {
    pub(super) fn is_report(&self) -> bool { self.report_id.is_some() }
}

/// A short-lived server proof tied to this exact host-owned personal route.
/// It grants delivery only; it cannot authorize any provider work.
pub(in crate::console) struct PersonalReportPermit {
    route_id: String,
    room_id: String,
    puppet: String,
    encrypted: bool,
    valid_until_ms: i64,
    deadline: std::time::Instant,
}
impl PersonalReportPermit {
    pub(super) fn new(
        proof: &crate::console::device_execution::AgentMessageAuthorization,
        lease: &crate::console::device_execution::LeaseRef,
        dispatch: &hagency_agent_local::inbox::Dispatch,
        execution: &str,
        generation: u64,
        puppet: &str,
        now: i64,
    ) -> Result<Self, ()> {
        if dispatch.agent_id != lease.agent_id { return Err(()); }
        proof.validate(lease, &dispatch.binding_id, dispatch.binding_generation,
            &dispatch.room_id, puppet, generation, now).map_err(|_| ())?;
        let route = Route::personal(dispatch, execution).ok_or(())?;
        let remaining = u64::try_from(proof.valid_until_ms.checked_sub(now).ok_or(())?).map_err(|_| ())?;
        let deadline = std::time::Instant::now().checked_add(std::time::Duration::from_millis(remaining)).ok_or(())?;
        Ok(Self { route_id: route.id, room_id: route.room_id, puppet: puppet.into(),
            encrypted: proof.encrypted, valid_until_ms: proof.valid_until_ms, deadline })
    }
    fn allows(&self, route: &Route, puppet: &str, now: i64) -> bool {
        self.route_id == route.id && self.room_id == route.room_id
            && self.puppet == puppet && self.valid_until_ms > now && std::time::Instant::now() < self.deadline
    }
}
impl Outbox {
    pub fn exists(device_root: &Path, card: &Card) -> bool {
        device_root
            .join("outgoing-messages")
            .join(&card.id)
            .exists()
    }
    pub(super) fn exists_for_route(device_root: &Path, route: &Route) -> bool {
        device_root
            .join("outgoing-messages")
            .join(&route.id)
            .exists()
    }
    /// `device_root` is the same private per-Agent directory used for its
    /// Matrix account; the run identity already contains the work and call.
    pub fn open(device_root: &Path, card: Card) -> Result<Self, ()> {
        Self::open_route(device_root, Route::card(card)?)
    }
    pub(super) fn open_route(device_root: &Path, route: Route) -> Result<Self, ()> {
        let root = device_root.join("outgoing-messages");
        hagency_store::private::directory(&root).map_err(|_| ())?;
        let cipher = cipher(&root)?;
        let directory = root.join(&route.id);
        hagency_store::private::directory(&directory).map_err(|_| ())?;
        let lock_directory = root.join("delivery-locks");
        hagency_store::private::directory(&lock_directory).map_err(|_| ())?;
        let lock_path = lock_directory.join(format!("{}.lock", hash(&route.id)));
        let mut state = State {
            records: BTreeMap::new(),
            ids: BTreeMap::new(),
            bytes: 0,
        };
        for entry in std::fs::read_dir(&directory).map_err(|_| ())? {
            if state.records.len() >= MAX_RECORDS {
                return Err(());
            }
            let path = entry.map_err(|_| ())?.path();
            let record: Record = cipher
                .decrypt_value(&read(&path, MAX_FILE_BYTES)?)
                .map_err(|_| ())?;
            if !match record.version {
                1 => record.report_id.is_none() && record.attempt.is_none(),
                2 => record.report_id.as_deref().is_some_and(valid_digest)
                    && record.attempt.as_ref().is_none_or(|attempt| valid_digest(&attempt.device)),
                _ => false,
            }
                || record.run != route.id
                || record.sequence == 0
                || record.id.is_empty()
                || record.id.len() > 2048
                || record.id.chars().any(char::is_control)
                || !valid_digest(&record.digest)
                || record
                    .text
                    .as_ref()
                    .is_some_and(|text| text.len() > 1024 * 1024 || text.is_empty())
                || record
                    .text
                    .as_ref()
                    .is_some_and(|text| hash(text) != record.digest)
                || (record.text.is_some() == record.event.is_some())
                || record
                    .event
                    .as_ref()
                    .is_some_and(|event| !event.starts_with('$') || event.len() > 1024)
                || path.file_name().and_then(|v| v.to_str())
                    != Some(&format!("{:020}.sealed", record.sequence))
                || state
                    .ids
                    .insert(record.id.clone(), record.sequence)
                    .is_some()
                || state.records.contains_key(&record.sequence)
            {
                return Err(());
            }
            state.bytes = state
                .bytes
                .checked_add(record.text.as_ref().map_or(0, String::len))
                .ok_or(())?;
            state.records.insert(record.sequence, record);
        }
        if state.records.len() > MAX_RECORDS || state.bytes > MAX_PENDING_BYTES {
            return Err(());
        }
        Ok(Self {
            route,
            directory,
            cipher,
            state: Mutex::new(state),
            lock_path,
        })
    }
    fn path(&self, sequence: u64) -> PathBuf {
        self.directory.join(format!("{sequence:020}.sealed"))
    }
    fn save(&self, record: &Record, new: bool) -> Result<(), ()> {
        let bytes = self.cipher.encrypt_value(record).map_err(|_| ())?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(());
        }
        if new {
            hagency_store::private::write_new(&self.path(record.sequence), &bytes)
        } else {
            hagency_store::private::replace(&self.path(record.sequence), &bytes)
        }
        .map_err(|_| ())
    }
    pub fn next(&self) -> Result<Option<Pending>, ()> {
        let state = self.state.lock().map_err(|_| ())?;
        Ok(state.records.values().find_map(|record| {
            record.text.as_ref().map(|text| Pending {
                sequence: record.sequence,
                txn: format!(
                    "hagency_message_{}",
                    hash(
                        serde_json::to_vec(&(&record.run, &record.id)).expect("strings serialize")
                    )
                ),
                text: text.clone(),
                report_id: record.report_id.clone(),
            })
        }))
    }
    /// Serialize attempt identity across processes as well as across Outbox
    /// handles. A caller that cannot acquire the lock retries later.
    fn delivery_lock(&self) -> Result<std::fs::File, ()> {
        let file = match hagency_store::private::open(&self.lock_path, true) {
            Ok(file) => file,
            Err(hagency_store::Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists =>
                hagency_store::private::open(&self.lock_path, false).map_err(|_| ())?,
            Err(_) => return Err(()),
        };
        file.try_lock().map_err(|_| ())?;
        Ok(file)
    }
    /// Persist the sender device and observed encryption state before the
    /// request can leave this host. The SDK rechecks encryption during send,
    /// so an unknown result still requires history reconciliation. Only new
    /// report records use this path; v1 rows retain their delivery behavior.
    fn begin_send(&self, sequence: u64, user: &str, device: &str, encrypted: bool) -> Result<Option<String>, ()> {
        if user.is_empty() || device.is_empty() || user.len() > 1024 || device.len() > 1024
            || user.chars().any(char::is_control) || device.chars().any(char::is_control)
        {
            return Err(());
        }
        let _lock = self.delivery_lock()?;
        let mut state = self.state.lock().map_err(|_| ())?;
        let path = self.path(sequence);
        let mut current: Record = self.cipher.decrypt_value(&read(&path, MAX_FILE_BYTES)?).map_err(|_| ())?;
        let previous = state.records.get(&sequence).ok_or(())?;
        if current.run != self.route.id || current.sequence != sequence
            || current.id != previous.id || current.digest != previous.digest
            || current.version != 2 || previous.version != 2
            || current.report_id != previous.report_id
            || current.text.as_ref().is_some_and(|text| hash(text) != current.digest)
            || (current.text.is_some() == current.event.is_some())
        {
            return Err(());
        }
        if current.event.is_some() {
            let old_bytes = previous.text.as_ref().map_or(0, String::len);
            state.bytes = state.bytes.checked_sub(old_bytes).ok_or(())?;
            state.records.insert(sequence, current);
            return Ok(None);
        }
        let report_id = current.report_id.clone().filter(|id| valid_digest(id)).ok_or(())?;
        let attempt = Attempt {
            device: hash(serde_json::to_vec(&(user, device)).map_err(|_| ())?),
            encrypted,
        };
        if let Some(previous) = &current.attempt {
            if previous != &attempt { return Err(()); }
        } else {
            current.attempt = Some(attempt);
            self.save(&current, false)?;
        }
        state.records.insert(sequence, current);
        Ok(Some(report_id))
    }
    /// The caller must recheck the originating work's current send authority
    /// immediately before each call. A timeout leaves the same immutable
    /// transaction pending so the homeserver can reconcile an unknown send.
    pub async fn send_next(
        &self,
        device: &super::e2ee::AgentDevice,
        encrypted: bool,
        permit: Option<&PersonalReportPermit>,
        expected_sequence: Option<u64>,
    ) -> Result<bool, ()> {
        let Some(pending) = self.next()? else {
            return Ok(false);
        };
        // Authorization was obtained for the record observed by the caller;
        // another outbox handle must not advance us to a different record.
        if expected_sequence.is_some_and(|sequence| sequence != pending.sequence) { return Err(()); }
        if device.is_revoked() {
            return Err(());
        }
        // Old run messages predate report IDs and attempt fencing. Preserve
        // their original transaction and send path across the upgrade.
        if pending.report_id.is_none() {
            let content = super::e2ee::native_message_content(
                &self.route.room_id, &self.route.thread_root, &pending.text,
            ).map_err(|_| ())?;
            let event = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                device.send_room_message_checked(&self.route.room_id, &pending.txn, content, encrypted),
            ).await.map_err(|_| ())?.map_err(|_| ())?;
            self.sent(pending.sequence, &event)?;
            return Ok(true);
        }
        let client = device.approval_client();
        let user = client.user_id().ok_or(())?;
        let device_id = client.device_id().ok_or(())?;
        let permit = permit.ok_or(())?;
        if !permit.allows(&self.route, user.as_str(), super::wall_ms()) { return Err(()); }
        let actual_encrypted = device.room_encrypted(&self.route.room_id).await.map_err(|_| ())?;
        if (encrypted || permit.encrypted) && !actual_encrypted { return Err(()); }
        if !permit.allows(&self.route, user.as_str(), super::wall_ms()) { return Err(()); }
        let Some(report_id) = self.begin_send(
            pending.sequence, user.as_str(), device_id.as_str(), actual_encrypted,
        )? else { return Ok(true); };
        if pending.report_id.as_deref() != Some(report_id.as_str()) { return Err(()); }
        let content = report_content(
            &self.route.room_id,
            &self.route.thread_root,
            &pending.text,
            &report_id,
        )
        ?;
        if device.is_revoked() || !permit.allows(&self.route, user.as_str(), super::wall_ms()) {
            return Err(());
        }
        // Do not let a slow room-state check continue into a send after this
        // proof expires. A request already in flight remains an unknown send.
        let wall_remaining = u64::try_from(permit.valid_until_ms.checked_sub(super::wall_ms()).ok_or(())?)
            .map_err(|_| ())?;
        let timeout = permit.deadline.saturating_duration_since(std::time::Instant::now())
            .min(std::time::Duration::from_millis(wall_remaining))
            .min(std::time::Duration::from_secs(30));
        if timeout.is_zero() { return Err(()); }
        let event = tokio::time::timeout(
            timeout,
            device.send_room_message_if_state_matches(&self.route.room_id, &pending.txn, content, actual_encrypted),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
        self.sent(pending.sequence, &event)?;
        Ok(true)
    }
    pub fn sent(&self, sequence: u64, event: &str) -> Result<(), ()> {
        if !event.starts_with('$') || event.len() > 1024 {
            return Err(());
        }
        let _lock = self.delivery_lock()?;
        let mut state = self.state.lock().map_err(|_| ())?;
        let record: Record = self.cipher.decrypt_value(&read(&self.path(sequence), MAX_FILE_BYTES)?)
            .map_err(|_| ())?;
        let previous = state.records.get(&sequence).ok_or(())?;
        if record.run != self.route.id || record.sequence != sequence
            || record.id != previous.id || record.digest != previous.digest
        { return Err(()); }
        if let Some(sent_event) = &record.event {
            if sent_event != event { return Err(()); }
            let old_bytes = previous.text.as_ref().map_or(0, String::len);
            state.bytes = state.bytes.checked_sub(old_bytes).ok_or(())?;
            state.records.insert(sequence, record);
            return Ok(());
        }
        let mut saved = record;
        let bytes = saved.text.take().ok_or(())?.len();
        saved.event = Some(event.into());
        let old_bytes = previous.text.as_ref().map_or(0, String::len);
        if old_bytes != bytes { return Err(()); }
        self.save(&saved, false)?;
        state.bytes = state.bytes.checked_sub(old_bytes).ok_or(())?;
        state.records.insert(sequence, saved);
        Ok(())
    }
}
impl MessageSink for Outbox {
    fn record(&self, id: &str, text: &str) -> Result<(), MessageError> {
        if id.is_empty() || id.len() > 2048 || id.chars().any(char::is_control)
            || text.is_empty() || text.len() > 1024 * 1024
        { return Err(MessageError); }
        let mut state = self.state.lock().map_err(|_| MessageError)?;
        let digest = hash(text);
        if let Some(sequence) = state.ids.get(id) {
            return (state
                .records
                .get(sequence)
                .is_some_and(|r| r.digest == digest))
            .then_some(())
            .ok_or(MessageError);
        }
        if state.records.len() >= MAX_RECORDS
            || state
                .bytes
                .checked_add(text.len())
                .is_none_or(|size| size > MAX_PENDING_BYTES)
        {
            return Err(MessageError);
        }
        let sequence = state
            .records
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(MessageError)?;
        // Ordinary run replies keep their v1 format and recovery behavior.
        // Only host-constructed, explicitly attributed Octos report IDs opt
        // into the new durable-message contract.
        let is_report = id.starts_with("octos:report:");
        let report_id = if is_report {
            let mut random = [0u8; 32];
            getrandom::fill(&mut random).map_err(|_| MessageError)?;
            Some(hash(random))
        } else {
            None
        };
        let record = Record {
            version: if is_report { 2 } else { 1 },
            run: self.route.id.clone(),
            sequence,
            id: id.into(),
            digest,
            text: Some(text.into()),
            event: None,
            report_id,
            attempt: None,
        };
        self.save(&record, true).map_err(|_| MessageError)?;
        state.bytes += text.len();
        state.ids.insert(id.into(), sequence);
        state.records.insert(sequence, record);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_message_route_does_not_depend_on_a_human_control_card() {
        let directory = tempfile::tempdir().unwrap();
        let mut dispatch = hagency_agent_local::inbox::Dispatch {
            id: "dispatch".into(),
            binding_id: "binding".into(),
            agent_id: "agent".into(),
            event_id: "$request:test".into(),
            room_id: "!room:test".into(),
            requester_mxid: "@person:test".into(),
            thread_root: "$thread:test".into(),
            body: "question".into(),
            state: "running".into(),
            binding_generation: 1,
            dispatch_epoch: Some(1),
            dispatch_device_id: Some("device".into()),
            execution_id: Some("execution".into()),
            outcome: None,
        };
        let card = Card::new(&dispatch, "execution", "@owner:test", "codex").unwrap();
        let route = Route::personal(&dispatch, "execution").unwrap();
        assert_eq!(
            route.id, card.id,
            "existing card outboxes must remain recoverable"
        );

        dispatch.requester_mxid = "@_hagency_agt_test:test".into();
        assert!(Card::new(&dispatch, "execution", "@owner:test", "codex").is_err());
        let route = Route::personal(&dispatch, "execution").unwrap();
        let outbox = Outbox::open_route(directory.path(), route.clone()).unwrap();
        outbox.record("message-1", "agent report").unwrap();
        let txn = outbox.next().unwrap().unwrap().txn;
        drop(outbox);
        assert!(Outbox::exists_for_route(directory.path(), &route));
        let recovered = Outbox::open_route(directory.path(), route).unwrap();
        assert_eq!(recovered.next().unwrap().unwrap().txn, txn);
        assert_eq!(recovered.route.room_id, "!room:test");
        assert_eq!(recovered.route.thread_root, "$thread:test");

        dispatch.thread_root = "!another-room:test".into();
        assert!(Route::personal(&dispatch, "execution").is_none());
    }

    #[test]
    fn personal_report_permit_pins_route_device_generation_and_expiry() {
        use crate::console::device_execution::{AgentMessageAuthorization, LeaseRef};
        let mut dispatch = hagency_agent_local::inbox::Dispatch {
            id: "dispatch".into(), binding_id: "binding".into(), agent_id: "agent".into(),
            event_id: "$request:test".into(), room_id: "!room:test".into(),
            requester_mxid: "@person:test".into(), thread_root: "$thread:test".into(),
            body: "question".into(), state: "completed".into(), binding_generation: 4,
            dispatch_epoch: Some(3), dispatch_device_id: Some("device".into()),
            execution_id: Some("execution".into()), outcome: None,
        };
        let lease = LeaseRef { agent_id: "agent".into(), epoch: 3 };
        let proof = AgentMessageAuthorization { agent_id: "agent".into(), binding_id: "binding".into(),
            binding_generation: 4, room_id: "!room:test".into(), puppet_mxid: "@agent:test".into(),
            encrypted: true, lease_epoch: 3, device_generation: 2, valid_until_ms: 40_000 };
        let permit = PersonalReportPermit::new(&proof, &lease, &dispatch, "execution", 2,
            "@agent:test", 10_000).unwrap();
        let route = Route::personal(&dispatch, "execution").unwrap();
        assert!(permit.allows(&route, "@agent:test", 10_000));
        assert!(!permit.allows(&route, "@other:test", 10_000));
        assert!(!permit.allows(&route, "@agent:test", 40_000));
        assert!(!permit.allows(&Route::personal(&dispatch, "other").unwrap(), "@agent:test", 10_000));
        assert!(PersonalReportPermit::new(&proof, &lease, &dispatch, "execution", 3, "@agent:test", 10_000).is_err());
        dispatch.binding_generation += 1;
        assert!(PersonalReportPermit::new(&proof, &lease, &dispatch, "execution", 2, "@agent:test", 10_000).is_err());
    }

    #[test]
    fn message_outbox_keeps_order_and_retries_the_same_encrypted_transaction() {
        let directory = tempfile::tempdir().unwrap();
        let card = crate::agent_run_controls::tests::card();
        let text = "private observation 中文 ".repeat(500);
        let outbox = Outbox::open(directory.path(), card.clone()).unwrap();
        outbox.record("first", &text).unwrap();
        outbox.record("second", "next").unwrap();
        outbox.record("first", &text).unwrap();
        assert!(outbox.record("first", "changed").is_err());
        let first = outbox.next().unwrap().unwrap();
        assert_eq!(first.text, text);
        let original_txn = first.txn.clone();
        let saved = std::fs::read(outbox.path(first.sequence)).unwrap();
        assert!(
            !saved
                .windows(b"private observation".len())
                .any(|v| v == b"private observation")
        );
        drop(outbox);

        let recovered = Outbox::open(directory.path(), card.clone()).unwrap();
        let retry = recovered.next().unwrap().unwrap();
        assert_eq!(retry.txn, original_txn);
        assert_eq!(retry.text, text);
        recovered.sent(retry.sequence, "$event:test").unwrap();
        recovered.sent(retry.sequence, "$event:test").unwrap();
        assert!(recovered.sent(retry.sequence, "$different:test").is_err());
        assert_eq!(recovered.next().unwrap().unwrap().text, "next");
        drop(recovered);

        let recovered = Outbox::open(directory.path(), card).unwrap();
        assert!(recovered.record("first", "changed").is_err());
        assert_eq!(recovered.next().unwrap().unwrap().text, "next");
    }

    #[test]
    fn report_attempt_is_durable_and_fenced_to_the_original_matrix_device() {
        let directory = tempfile::tempdir().unwrap();
        let card = crate::agent_run_controls::tests::card();
        let outbox = Outbox::open(directory.path(), card.clone()).unwrap();
        outbox.record("octos:report:first", "same text").unwrap();
        outbox.record("octos:report:second", "same text").unwrap();
        let first = outbox.next().unwrap().unwrap();
        let marker = outbox.begin_send(first.sequence, "@agent:test", "DEVICE-A", true).unwrap().unwrap();
        let content = report_content("!room:test", "$root:test", &first.text, &marker).unwrap();
        assert_eq!(content["msgtype"], "m.text");
        assert_eq!(content["body"], "same text");
        assert_eq!(content["org.hagency.report_id"], marker);
        assert_eq!(content["m.relates_to"]["event_id"], "$root:test");
        let bytes = std::fs::read(outbox.path(first.sequence)).unwrap();
        assert!(!bytes.windows(b"DEVICE-A".len()).any(|part| part == b"DEVICE-A"));
        drop(outbox);

        let recovered = Outbox::open(directory.path(), card.clone()).unwrap();
        let retry = recovered.next().unwrap().unwrap();
        assert_eq!(retry.txn, first.txn);
        assert_eq!(recovered.begin_send(retry.sequence, "@agent:test", "DEVICE-A", true).unwrap(), Some(marker));
        assert!(recovered.begin_send(retry.sequence, "@agent:test", "DEVICE-B", true).is_err());
        assert!(recovered.begin_send(retry.sequence, "@agent:test", "DEVICE-A", false).is_err());
        assert!(recovered.begin_send(retry.sequence, "@other:test", "DEVICE-A", true).is_err());
        recovered.sent(retry.sequence, "$event:test").unwrap();
        let second = recovered.next().unwrap().unwrap();
        assert_eq!(second.text, first.text);
        assert_ne!(second.report_id, first.report_id);
    }

    #[test]
    fn legacy_pending_messages_preserve_their_original_transaction() {
        let directory = tempfile::tempdir().unwrap();
        let card = crate::agent_run_controls::tests::card();
        let outbox = Outbox::open(directory.path(), card.clone()).unwrap();
        outbox.record("old", "retained").unwrap();
        let pending = outbox.next().unwrap().unwrap();
        assert!(pending.report_id.is_none());
        drop(outbox);
        let recovered = Outbox::open(directory.path(), card).unwrap();
        let retry = recovered.next().unwrap().unwrap();
        assert_eq!(retry.text, "retained");
        assert_eq!(retry.txn, pending.txn);
        assert!(retry.report_id.is_none());
    }

    #[test]
    fn concurrent_outbox_handles_observe_the_same_sent_record() {
        let directory = tempfile::tempdir().unwrap();
        let card = crate::agent_run_controls::tests::card();
        let first = Outbox::open(directory.path(), card.clone()).unwrap();
        first.record("octos:report:one", "one").unwrap();
        let second = Outbox::open(directory.path(), card).unwrap();
        let sequence = first.next().unwrap().unwrap().sequence;
        first.begin_send(sequence, "@agent:test", "DEVICE", true).unwrap();
        second.begin_send(sequence, "@agent:test", "DEVICE", true).unwrap();
        first.sent(sequence, "$event:test").unwrap();
        second.sent(sequence, "$event:test").unwrap();
        assert!(second.next().unwrap().is_none());
    }

    #[test]
    fn native_message_content_keeps_the_entire_body_and_original_thread() {
        let text = "中文".repeat(30_000);
        let content =
            super::super::e2ee::native_message_content("!room:test", "$root:test", &text).unwrap();
        assert_eq!(content["body"], text);
        assert_eq!(content["m.relates_to"]["rel_type"], "m.thread");
        assert_eq!(content["m.relates_to"]["event_id"], "$root:test");
    }

    #[test]
    fn missing_encryption_key_never_replaces_existing_message_custody() {
        let directory = tempfile::tempdir().unwrap();
        let card = crate::agent_run_controls::tests::card();
        let outbox = Outbox::open(directory.path(), card.clone()).unwrap();
        outbox.record("one", "retained").unwrap();
        drop(outbox);
        let root = directory.path().join("outgoing-messages");
        std::fs::remove_file(root.join("cipher-key")).unwrap();
        assert!(Outbox::open(directory.path(), card).is_err());
        assert!(!root.join("cipher-key").exists());
    }
}
