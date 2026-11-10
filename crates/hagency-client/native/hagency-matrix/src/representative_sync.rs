//! TS `lib/representative-sync.js` parity (task #80): the registration
//! representative's own `/sync` intake — an ordinary access token belonging to
//! the recorded representative, no login and no synthetic `hs_token`.
//!
//! Control flow is a direct transcription of the retained collector:
//!
//! * `startRepresentativeSyncCollector` (`lib/representative-sync.js:5-101`) —
//!   one loop that (a) retries pending gap recoveries, (b) polls `/sync`,
//!   (c) delivers invites + (for a non-initial response) the timeline and
//!   state events, then leaves, (d) records pending gaps BEFORE the cursor,
//!   (e) commits the cursor, and (f) retries recovery. Backoff is
//!   exponential 1 s → 60 s and resets on any successful poll; a REFUSED
//!   delivery is not terminal: it retries on a fixed 1 s lane and only after
//!   8 consecutive delivery failures does it report a circuit break and
//!   stop — holding its cursor so no event is lost.
//! * `reconcileRepresentativeTimeline` (`:104-124`) — bounded gap recovery
//!   that walks backwards until it reaches an event it already observed, and
//!   refuses (retryably) rather than guessing a history boundary.
//!
//! The transport, the durable state and the delivery hooks are injected: the
//! production driver is the bounded `Http` client (`RepresentativeHttp`), the
//! production state is the host's private state directory, and the tests drive
//! scripted fakes. No capability lives in this module that a caller cannot
//! supply, and nothing here decides the work is finished.
use crate::{Error, Limits, http::Http};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
};

/// A boxed future: the injection seam for every effect this collector
/// performs. `hagency-matrix` carries no `async_trait` dependency, and a
/// generic parameter per effect would be unreadable.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// TS `let backoff = 1_000` (lib/representative-sync.js:43).
pub const START_BACKOFF_MS: u64 = 1_000;
/// TS `backoff = Math.min(backoff * 2, 60_000)` (:96).
pub const MAX_BACKOFF_MS: u64 = 60_000;
/// TS `++stats.batchAttempts >= 8` (:88).
pub const MAX_DELIVERY_ATTEMPTS: u64 = 8;
/// TS `sleep(deliveryFailed ? 1_000 : backoff, ...)` (:95).
pub const DELIVERY_RETRY_MS: u64 = 1_000;
/// TS `maxPages = 10` (:104).
pub const MAX_RECONCILE_PAGES: usize = 10;
/// TS the sync filter caps to-device/account-data away; the poll timeout is
/// 30 s (`lib/appservice-sync.js:100`).
pub const SYNC_TIMEOUT_MS: u64 = 30_000;

/// A failed poll or a refused recovery. TS distinguishes exactly one code —
/// `unproven_gap_boundary` (:33) — whose meaning is "the gap remains recorded;
/// retrying cannot clear it by guessing". Everything else is a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SyncError {
    pub message: String,
    /// TS `error?.code === 'unproven_gap_boundary'`.
    pub boundary: bool,
}
impl SyncError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            boundary: false,
        }
    }
    /// TS `Object.assign(new Error(message), { code: 'unproven_gap_boundary' })`.
    pub fn boundary(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            boundary: true,
        }
    }
}

/// TS `writePendingReconcile(roomId, verdict)`: `'pending'` records a gap,
/// `'cleared'` retires it (lib/representative-sync.js:73, :31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingVerdict {
    Pending,
    Cleared,
}

/// TS `stats` (:23) — the whole counter object, including `gaveUp`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStats {
    pub polls: u64,
    pub processed: u64,
    pub failed: u64,
    pub batch_attempts: u64,
    pub last_error: Option<String>,
    pub gave_up: bool,
}

/// TS `onCircuitBreak({ attempts, heldCursor, lastError })` (:90).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircuitBreak {
    pub attempts: u64,
    pub held_cursor: Option<String>,
    pub last_error: String,
}

/// TS `appserviceSyncOnce`'s projection (`lib/appservice-sync.js:139-180`): the
/// five signals a `/sync` body carries, every timeline/state/invite event
/// stamped with its `room_id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncBatch {
    pub next_batch: String,
    pub timeline: Vec<Value>,
    pub invites: Vec<Value>,
    pub state: Vec<Value>,
    pub leaves: Vec<String>,
    pub rooms_needing_reconcile: Vec<String>,
    pub reconcile_tokens: BTreeMap<String, Option<String>>,
    /// TS `initial: !since` — an initial timeline is historical baseline, not a
    /// missed live interval (:59, :71).
    pub initial: bool,
}
impl SyncBatch {
    /// TS `appserviceSyncOnce` body projection. `since` is the cursor the poll
    /// was made with — a first poll (no cursor) is `initial`.
    pub fn from_body(body: &Value, since: Option<&str>) -> Result<Self, SyncError> {
        // TS F08: HTTP 200 with no `next_batch` is NOT a healthy poll.
        let next_batch = body
            .get("next_batch")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                SyncError::new("sync answered HTTP 200 with no next_batch — not a sync response")
            })?
            .to_owned();
        // TS: a present-but-non-object `rooms` is the same class of error.
        if let Some(rooms) = body.get("rooms")
            && !rooms.is_object()
        {
            return Err(SyncError::new(
                "sync answered HTTP 200 with a non-object rooms section",
            ));
        }
        let mut batch = Self {
            next_batch,
            initial: since.is_none(),
            ..Self::default()
        };
        // TS `body?.rooms?.join ?? {}` — an absent section is an empty one.
        let section = |name: &str| -> Option<&serde_json::Map<String, Value>> {
            body.get("rooms")
                .and_then(Value::as_object)
                .and_then(|rooms| rooms.get(name))
                .and_then(Value::as_object)
        };
        for (room_id, room) in section("join").into_iter().flatten() {
            let timeline = || room.get("timeline");
            let events = |section: &str| -> Vec<Value> {
                room.get(section)
                    .and_then(|s| s.get("events"))
                    .and_then(Value::as_array)
                    .map(|events| events.iter().cloned().map(|e| stamp(e, room_id)).collect())
                    .unwrap_or_default()
            };
            // A timeline event is projected; its room is stamped on.
            batch.timeline.extend(events("timeline"));
            // TS F07: state events rode a section nothing projected.
            batch.state.extend(events("state"));
            // TS F07: `timeline.limited` is the homeserver saying "there is a
            // gap you have not seen" — the room needs a reconcile.
            if timeline()
                .and_then(|t| t.get("limited"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                batch.rooms_needing_reconcile.push(room_id.clone());
                batch.reconcile_tokens.insert(
                    room_id.clone(),
                    timeline()
                        .and_then(|t| t.get("prev_batch"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
        for room_id in section("leave").into_iter().flatten().map(|(id, _)| id) {
            batch.leaves.push(room_id.clone());
        }
        for (room_id, room) in section("invite").into_iter().flatten() {
            if let Some(events) = room
                .get("invite_state")
                .and_then(|s| s.get("events"))
                .and_then(Value::as_array)
            {
                batch
                    .invites
                    .extend(events.iter().cloned().map(|e| stamp(e, room_id)));
            }
        }
        Ok(batch)
    }

    /// TS `result.inviteEvents.filter(event => event.type === 'm.room.member' &&
    /// event.content?.membership === 'invite' && event.state_key ===
    /// representativeMxid)` (lib/representative-sync.js:57-58). Stripped invite
    /// state is room metadata, not an admitted timeline: only the recorded
    /// representative's exact invite can bootstrap the join.
    pub fn representative_invites(&self, representative_mxid: &str) -> Vec<Value> {
        self.invites
            .iter()
            .filter(|event| {
                event.get("type").and_then(Value::as_str) == Some("m.room.member")
                    && event
                        .get("content")
                        .and_then(|c| c.get("membership"))
                        .and_then(Value::as_str)
                        == Some("invite")
                    && event.get("state_key").and_then(Value::as_str) == Some(representative_mxid)
            })
            .cloned()
            .collect()
    }

    /// TS `[...invites, ...(result.initial ? [] : [...result.timelineEvents,
    /// ...result.stateEvents])]` (:59).
    pub fn delivered_events(&self, representative_mxid: &str) -> Vec<Value> {
        let mut events = self.representative_invites(representative_mxid);
        if !self.initial {
            events.extend(self.timeline.iter().cloned());
            events.extend(self.state.iter().cloned());
        }
        events
    }

    /// TS `txnId: \`representative-sync-${result.nextBatch}\`` (:61).
    pub fn txn_id(&self) -> String {
        format!("representative-sync-{}", self.next_batch)
    }
}

/// `stamp`: TS `{ ...ev, room_id: roomId }` — the room id is ADDED, never
/// replacing an event's own.
fn stamp(mut event: Value, room_id: &str) -> Value {
    if let Some(object) = event.as_object_mut() {
        object.insert("room_id".into(), json!(room_id));
    }
    event
}

/// The injected `/sync` poll, the abort gate and the sleep lane.
pub trait SyncDriver: Send + Sync {
    /// TS `active()` (:22): not aborted and the caller still wants this side.
    fn active(&self) -> bool;
    /// TS `appserviceSyncOnce({ baseUrl, accessToken, since, fetchImpl, signal })`.
    fn sync_once(&self, since: Option<String>) -> BoxFuture<'_, Result<SyncBatch, SyncError>>;
    /// TS the injected `sleep(ms, signal)` (:13).
    fn sleep(&self, ms: u64) -> BoxFuture<'_, ()>;
}

/// The injected durable state: the cursor, the pending-gap map and the
/// observed-event ids per room (TS `state.representativeSync[sideId]`,
/// bridge-matrix.js:5211-5246).
pub trait SyncState: Send + Sync {
    fn cursor(&self) -> BoxFuture<'_, Option<String>>;
    fn write_cursor(&self, cursor: String) -> BoxFuture<'_, Result<(), Error>>;
    /// TS `readPendingReconcile()` → `Object.keys(stored.pending ?? {})`.
    fn pending(&self) -> BoxFuture<'_, Vec<String>>;
    /// TS `writePendingReconcile(roomId, verdict, detail)`.
    fn write_pending(
        &self,
        room: String,
        verdict: PendingVerdict,
        from: Option<String>,
    ) -> BoxFuture<'_, Result<(), Error>>;
    /// TS `[...(stored.observed?.[roomId] ?? [])]` — the known-event boundary
    /// TS `onObservedTimeline(events)` (last 512 ids per room, bridge-matrix.js:5242).
    /// The caller keeps them; the recovery's `knownEventIds` comes from here.
    fn record_observed(&self, events: Vec<Value>) -> BoxFuture<'_, ()>;
}

/// TS `onEvents(events, { provenance, txnId })` (:61) — the second argument is
/// the provenance the caller feeds its router: the side, its registration and
/// the mode that carried the batch.
#[derive(Debug, Clone, PartialEq)]
pub struct EventMeta {
    pub provenance: Value,
    pub txn_id: String,
}

/// The injected delivery and reporting side. Every one of these is the caller's
/// (bridge-matrix.js:5215-5270): native keeps no room router in this module.
pub trait SyncHooks: Send + Sync {
    /// TS `onEvents(events, { provenance, txnId })` (:61).
    fn events(&self, events: Vec<Value>, meta: EventMeta) -> BoxFuture<'_, Result<(), Error>>;
    /// TS `onLeaves(result.leaves)` (:64) — leaves revoke local reachability.
    fn leaves(&self, rooms: Vec<String>) -> BoxFuture<'_, Result<(), Error>>;
    /// TS `onReconcile(roomId)` (:30): run the bounded recovery for one room.
    fn reconcile(&self, room: String) -> BoxFuture<'_, Result<(), SyncError>>;
    /// TS `onReconcileBlocked(roomId, error.message)` (:35).
    fn reconcile_blocked(&self, room: String, reason: String) -> BoxFuture<'_, ()>;
    /// TS `onCircuitBreak({ attempts, heldCursor, lastError })` (:90).
    fn circuit_break(&self, detail: CircuitBreak) -> BoxFuture<'_, ()>;
}

/// One page of room history, as TS `readPage(cursor)` returns it (:111).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryPage {
    /// TS `page?.known !== true` refuses with `page.reason` (:112).
    pub known: bool,
    pub reason: Option<String>,
    pub chunk: Vec<Value>,
    /// TS `page.end` (:120).
    pub end: Option<String>,
}

/// TS the `readPage` the caller supplies to `reconcileRepresentativeTimeline`.
pub trait PageReader: Send + Sync {
    fn read(&self, from: String) -> BoxFuture<'_, Result<HistoryPage, SyncError>>;
}

/// TS the `onEvents` a recovery delivers recovered history through (:115).
pub trait PageSink: Send + Sync {
    fn deliver(&self, events: Vec<Value>) -> BoxFuture<'_, Result<(), Error>>;
}

/// TS `reconcileRepresentativeTimeline({ from, knownEventIds, readPage, onEvents,
/// maxPages = 10 })` (lib/representative-sync.js:104-124).
///
/// Recover a limited timeline only as far back as a previously observed event:
/// the walk refuses (retryably) when it has no proven boundary, when history is
/// unavailable, and when it runs out of pages without reaching one. It never
/// guesses a boundary and never replays an unreached tail.
pub async fn reconcile_timeline<R: PageReader, S: PageSink>(
    from: Option<&str>,
    known_event_ids: &[String],
    read_page: &R,
    on_events: &S,
) -> Result<(), SyncError> {
    let unproven = |message: &str| SyncError::boundary(message);
    let Some(from) = from.filter(|f| !f.is_empty()) else {
        return Err(unproven("sync gap has no proven cursor/event boundary"));
    };
    if known_event_ids.is_empty() {
        return Err(unproven("sync gap has no proven cursor/event boundary"));
    }
    let known = known_event_ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut pending: Vec<Value> = Vec::new();
    let mut cursor = from.to_owned();
    for page_number in 0..MAX_RECONCILE_PAGES {
        let _ = page_number;
        let page = read_page.read(cursor.clone()).await?;
        if !page.known {
            return Err(SyncError::new(
                page.reason
                    .unwrap_or_else(|| "sync gap history is unavailable".into()),
            ));
        }
        for event in &page.chunk {
            let event_id = event.get("event_id").and_then(Value::as_str);
            if event_id.is_some_and(|id| known.contains(id)) {
                // TS `onEvents(pending.reverse())` — the walk pushed newest
                // first, so the delivery is oldest first.
                pending.reverse();
                on_events
                    .deliver(pending)
                    .await
                    .map_err(|error| SyncError::new(error.to_string()))?;
                return Ok(());
            }
            if event.get("type").and_then(Value::as_str) == Some("m.room.message")
                && event_id.is_some()
            {
                pending.push(event.clone());
            }
        }
        // TS `if (!page.end || page.end === cursor || !page.chunk?.length) break`.
        if page.end.is_none()
            || page.end.as_deref() == Some(cursor.as_str())
            || page.chunk.is_empty()
        {
            break;
        }
        cursor = page.end.clone().unwrap_or_default();
    }
    Err(unproven(
        "sync gap history did not reach a previously observed event; recovery remains pending",
    ))
}

/// TS `startRepresentativeSyncCollector` — the collector's decision core.
///
/// One `run` call is one collector lifetime: it holds the counters, the
/// blocked-recovery set and the backoff across polls, exactly as the retained
/// closure did. It returns only when the caller stops it or after a
/// circuit-break; it never decides the intake is finished.
pub struct RepresentativeSync {
    side: String,
    representative_mxid: String,
    provenance: Value,
    stats: SyncStats,
    blocked: BTreeSet<String>,
    backoff_ms: u64,
}
impl RepresentativeSync {
    /// TS `buildSideProvenance({ sideId: side, registration, mode: 'sync' })`
    /// (lib/side-provenance.js:95).
    pub fn new(side: &str, registration: &str, representative_mxid: &str) -> Result<Self, Error> {
        let side = side.trim().to_lowercase();
        let registration = registration.trim();
        if side.is_empty() || registration.is_empty() || representative_mxid.is_empty() {
            return Err(Error::Config);
        }
        let provenance = json!({
            "registration": registration,
            "sideId": side,
            "mode": "sync",
        });
        Ok(Self {
            side,
            representative_mxid: representative_mxid.to_owned(),
            provenance,
            stats: SyncStats::default(),
            blocked: BTreeSet::new(),
            backoff_ms: START_BACKOFF_MS,
        })
    }

    pub fn stats(&self) -> &SyncStats {
        &self.stats
    }
    pub fn provenance(&self) -> &Value {
        &self.provenance
    }
    pub fn side(&self) -> &str {
        &self.side
    }

    /// TS `reconcile()` (:25-41): retry every recorded gap, once each, skipping
    /// rooms whose boundary was already proven unprovable this lifetime.
    pub async fn reconcile<S: SyncState, H: SyncHooks>(
        &mut self,
        driver: &impl SyncDriver,
        state: &S,
        hooks: &H,
    ) {
        for room in state.pending().await {
            if !driver.active() {
                return;
            }
            if self.blocked.contains(&room) {
                continue;
            }
            match hooks.reconcile(room.clone()).await {
                Ok(()) => {
                    if driver.active() {
                        let _ = state
                            .write_pending(room, PendingVerdict::Cleared, None)
                            .await;
                    }
                }
                Err(error) => {
                    if error.boundary {
                        self.blocked.insert(room.clone());
                        hooks
                            .reconcile_blocked(room.clone(), error.message.clone())
                            .await;
                    }
                    // TS warns and keeps the pending record (:38).
                }
            }
        }
    }

    /// The collect loop (lib/representative-sync.js:42-99). Returns when the
    /// caller stops it or the delivery lane circuit-breaks.
    pub async fn run<D: SyncDriver, S: SyncState, H: SyncHooks>(
        &mut self,
        driver: &D,
        state: &S,
        hooks: &H,
    ) {
        while driver.active() {
            match self.poll_once(driver, state, hooks).await {
                // TS every `if (!active()) break` (:48, :51, :62, :77) leaves
                // the loop WITHOUT bookkeeping: a poll that arrived after the
                // caller stopped is never a poll.
                Polled::Stopped => break,
                Polled::Ok => {
                    self.stats.processed += 1;
                    self.stats.batch_attempts = 0;
                    self.stats.last_error = None;
                    self.backoff_ms = START_BACKOFF_MS;
                    // TS the second `reconcile()` at :83.
                    self.reconcile(driver, state, hooks).await;
                }
                Polled::Failed { delivery, error } => {
                    // TS the catch's own `if (!active()) break` (:85).
                    if !driver.active() {
                        break;
                    }
                    self.stats.failed += 1;
                    self.stats.last_error = Some(error.message.clone());
                    if delivery {
                        self.stats.batch_attempts += 1;
                        if self.stats.batch_attempts >= MAX_DELIVERY_ATTEMPTS {
                            self.stats.gave_up = true;
                            let held_cursor = state.cursor().await;
                            hooks
                                .circuit_break(CircuitBreak {
                                    attempts: self.stats.batch_attempts,
                                    held_cursor,
                                    last_error: error.message,
                                })
                                .await;
                            break;
                        }
                    }
                    // TS `logger.warn?.()` (:94). This crate carries no logging
                    // dependency; `outgoing.rs` uses the same stderr lane.
                    eprintln!(
                        "[representative-sync] side {} intake failed: {}",
                        self.side,
                        self.stats.last_error.as_deref().unwrap_or_default()
                    );
                    driver
                        .sleep(if delivery {
                            DELIVERY_RETRY_MS
                        } else {
                            self.backoff_ms
                        })
                        .await;
                    if !delivery {
                        self.backoff_ms = (self.backoff_ms * 2).min(MAX_BACKOFF_MS);
                    }
                }
            }
        }
    }

    /// One poll (lib/representative-sync.js:46-83), minus the success/failure
    /// bookkeeping the caller does. `Polled::Failed{delivery: true}` is the
    /// retried-at-1 s lane: the refusal came from `onEvents`/`onLeaves`, so the
    /// cursor was never committed and the SAME batch is redelivered.
    async fn poll_once<D: SyncDriver, S: SyncState, H: SyncHooks>(
        &mut self,
        driver: &D,
        state: &S,
        hooks: &H,
    ) -> Polled {
        self.reconcile(driver, state, hooks).await;
        if !driver.active() {
            return Polled::Stopped;
        }
        let since = state.cursor().await;
        let result = match driver.sync_once(since.clone()).await {
            Ok(result) => result,
            Err(error) => {
                return Polled::Failed {
                    delivery: false,
                    error,
                };
            }
        };
        if !driver.active() {
            return Polled::Stopped;
        }
        self.stats.polls += 1;
        let events = result.delivered_events(&self.representative_mxid);
        if !events.is_empty()
            && let Err(error) = hooks
                .events(
                    events,
                    EventMeta {
                        provenance: self.provenance.clone(),
                        txn_id: result.txn_id(),
                    },
                )
                .await
        {
            return Polled::Failed {
                delivery: true,
                error: SyncError::new(error.to_string()),
            };
        }
        // TS leaves revoke local reachability before committing the response.
        if !result.leaves.is_empty()
            && let Err(error) = hooks.leaves(result.leaves.clone()).await
        {
            return Polled::Failed {
                delivery: true,
                error: SyncError::new(error.to_string()),
            };
        }
        // TS an initial timeline is historical baseline, not a missed interval:
        // a gap is only recorded for a non-initial response (:71-75). The
        // pending record is written BEFORE the cursor, so a crash between the
        // two re-pulls the same events and the gap is never lost.
        if !result.initial {
            for room in &result.rooms_needing_reconcile {
                let from = result.reconcile_tokens.get(room).cloned().flatten();
                if let Err(error) = state
                    .write_pending(room.clone(), PendingVerdict::Pending, from)
                    .await
                {
                    return Polled::Failed {
                        delivery: false,
                        error: SyncError::new(error.to_string()),
                    };
                }
            }
        }
        state.record_observed(result.timeline.clone()).await;
        // TS `if (!active()) break` at :77 — checked BEFORE the cursor write, so
        // a caller who stopped mid-poll never has its cursor consumed.
        if !driver.active() {
            return Polled::Stopped;
        }
        if let Err(error) = state.write_cursor(result.next_batch.clone()).await {
            return Polled::Failed {
                delivery: false,
                error: SyncError::new(error.to_string()),
            };
        }
        Polled::Ok
    }
}

/// How one collector iteration ended.
enum Polled {
    /// The caller stopped it mid-flight; no counter moves.
    Stopped,
    Ok,
    Failed {
        delivery: bool,
        error: SyncError,
    },
}

/// The production driver: the bounded `Http` client, one representative
/// access token, one homeserver (`lib/appservice-sync.js:100-141`).
pub struct RepresentativeHttp {
    http: Http,
    representative_mxid: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancel: crate::CancellationToken,
}
impl RepresentativeHttp {
    /// `base_url` is the side's homeserver; `token` the exact representative
    /// access token (never persisted by this type). Construction performs no
    /// I/O, like every other host client here.
    pub fn new(
        base_url: &str,
        token: &str,
        representative_mxid: &str,
        limits: &Limits,
        cancel: &crate::CancellationToken,
    ) -> Result<Self, Error> {
        let endpoint = reqwest::Url::parse(base_url).map_err(|_| Error::Config)?;
        if endpoint.scheme() != "https"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(Error::Config);
        }
        let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| Error::Config)?;
        authorization.set_sensitive(true);
        Ok(Self {
            http: Http::for_host(&endpoint, Some(&authorization), limits, &[])?,
            representative_mxid: representative_mxid.to_owned(),
            active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            cancel: cancel.clone(),
        })
    }
    pub fn representative_mxid(&self) -> &str {
        &self.representative_mxid
    }
    /// TS `stop: () => controller.abort()` (:100).
    pub fn stop(&self) {
        self.active
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
impl SyncDriver for RepresentativeHttp {
    fn active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::SeqCst) && !self.cancel.is_cancelled()
    }
    fn sync_once(&self, since: Option<String>) -> BoxFuture<'_, Result<SyncBatch, SyncError>> {
        Box::pin(async move {
            // TS `lib/appservice-sync.js:100-108`: presence offline, to-device
            // and account-data cut away, the timeline deliberately
            // UNRESTRICTED (a type allowlist silently swallowed every message
            // against the real homeserver).
            let filter =
                json!({"account_data": {"types": []}, "to_device": {"types": []}}).to_string();
            let timeout = SYNC_TIMEOUT_MS.to_string();
            let mut query = vec![
                ("timeout", timeout.as_str()),
                ("set_presence", "offline"),
                ("filter", filter.as_str()),
            ];
            if let Some(since) = since.as_deref() {
                query.push(("since", since));
            }
            let response = self
                .http
                .request(
                    &["_matrix", "client", "v3", "sync"],
                    Some(&query),
                    &self.cancel,
                )
                .await
                .map_err(|error| SyncError::new(error.to_string()))?;
            if response.status == 401 {
                return Err(SyncError::new("sync answered 401"));
            }
            let body = response
                .success()
                .map_err(|error| SyncError::new(error.to_string()))?;
            SyncBatch::from_body(&body, since.as_deref())
        })
    }
    fn sleep(&self, ms: u64) -> BoxFuture<'_, ()> {
        let cancel = self.cancel.clone();
        Box::pin(async move {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(ms)) => {}
                _ = cancel.cancelled() => {}
            }
        })
    }
}
