//! TS test oracle — bridge-layer routing/trust/sync (board #72).
//!
//! The retained TS suite is the parity ORACLE. Each test below names one TS case
//! (`tests/*.test.js:<line>`) and states the exact observable outcome it asserts.
//! Where native has NO surface for the behaviour, the test is `#[ignore]`d with a
//! one-line reason — this task changes no product code (see `.peer/report-72.md`
//! for the full pass/gap/cited/skip map).
//!
//! Out-of-scope skips (declared in the board): legacy group commands
//! `!mkgroup`/`!bindroom`, and tmux probes (`!sessions`/`!mcp`/`!spy`/`!ctl`).
//! Those are reported, not encoded here.

/// TS `tests/room-trust.test.js` — `getRoomTrust`/`markRoomTrusted` classifier
/// (bridge-matrix.js:2841, 2854). Native now has the classifier
/// (`hagency-matrix/src/room_trust.rs`, task #80): allowlist (env
/// `MATRIX_TRUSTED_ROOM_IDS`) / managed (frozen targets + store-marked rows) /
/// trusted inviter (env `MATRIX_TRUSTED_INVITER_MXIDS`) / unknown_room, with the
/// same precedence. The tests construct it directly (`with_sets`) so no env is
/// touched.

#[test]
fn ts_oracle_room_trust_allowlist_and_managed_and_inviter() {
    use hagency_matrix::{RoomTrust, RoomTrustReason, TrustMode};
    let set = |items: &[&str]| items.iter().map(|s| s.to_string()).collect();
    let trust = RoomTrust::with_sets(
        set(&["!allow1:matrix.test"]),
        set(&["@admin:matrix.test"]),
        vec!["!existing:matrix.test".to_string()],
        vec!["!marked:matrix.test".to_string()],
        TrustMode::Audit,
    );
    // allowlist
    assert_eq!(
        trust.classify("!allow1:matrix.test", None),
        RoomTrustReason::Allowlist
    );
    assert!(RoomTrustReason::Allowlist.trusted());
    // managed: a frozen target
    assert_eq!(
        trust.classify("!existing:matrix.test", None),
        RoomTrustReason::Managed
    );
    // managed: a store-marked (TS trustedManagedRooms) room
    assert_eq!(
        trust.classify("!marked:matrix.test", None),
        RoomTrustReason::Managed
    );
    // trusted inviter
    assert_eq!(
        trust.classify("!unknown:matrix.test", Some("@admin:matrix.test")),
        RoomTrustReason::TrustedInviter
    );
    // unknown room
    assert_eq!(
        trust.classify("!rando:evil.server", None),
        RoomTrustReason::UnknownRoom
    );
    assert!(!RoomTrustReason::UnknownRoom.trusted());
    // unknown room with an untrusted inviter
    assert_eq!(
        trust.classify("!rando:evil.server", Some("@hacker:evil.server")),
        RoomTrustReason::UnknownRoom
    );
}

#[test]
fn ts_oracle_room_trust_defaults_to_audit() {
    use hagency_matrix::{RoomTrust, TrustMode};
    // TS: MATRIX_TRUST_MODE defaults to 'audit'; every other spelling is audit.
    assert_eq!(TrustMode::from_word(""), TrustMode::Audit);
    assert_eq!(TrustMode::from_word("open"), TrustMode::Audit);
    assert_eq!(TrustMode::from_word("enforce"), TrustMode::Enforce);
    // audit: an untrusted room is admitted (only logged), never refused.
    let trust = RoomTrust::with_sets(
        Default::default(),
        Default::default(),
        Vec::new(),
        Vec::new(),
        TrustMode::Audit,
    );
    assert!(trust.admit("!rando:evil.server", None).is_ok());
    // enforce: an untrusted room is refused.
    let trust = RoomTrust::with_sets(
        Default::default(),
        Default::default(),
        Vec::new(),
        Vec::new(),
        TrustMode::Enforce,
    );
    assert!(trust.admit("!rando:evil.server", None).is_err());
}

/// TS `tests/pending-invite-store.test.js` — `createPendingInviteStore`
/// (lib/pending-invite-store.js). Native has no pending-invite store: invited
/// rooms are not a contributor-facing record anywhere in native.

#[test]
#[ignore = "parity gap: no native pending-invite store (lib/pending-invite-store.js)"]
fn ts_oracle_pending_invite_captures_and_derives_server() {
    // TS asserts: upsert captures {room, agent, inviter, state:'pending'};
    // projectServer is DERIVED from the room id, never taken from the caller.
    let _ = ();
}

#[test]
#[ignore = "parity gap: no native pending-invite store (lib/pending-invite-store.js)"]
fn ts_oracle_pending_invite_idempotent_per_agent_key() {
    // TS asserts: re-upsert of the same (room, agent) does not multiply; one room
    // holds several agents keyed (room, agent) each with its own inviter.
    let _ = ();
}

#[test]
#[ignore = "parity gap: no native pending-invite store (lib/pending-invite-store.js)"]
fn ts_oracle_pending_invite_settlement_conflicts_and_never_prunes_pending() {
    // TS asserts: settle records decidedBy+decidedAt; a settled record does NOT
    // reopen on re-report; double-decide throws; deciding an absent record 404s;
    // only accepted/declined are decisions; list is pending-only newest-first; a
    // PENDING record is never pruned; every mutation persists.
    let _ = ();
}

/// TS `tests/representative-sync.test.js` — `startRepresentativeSyncCollector`
/// and `reconcileRepresentativeTimeline` (lib/representative-sync.js), ported as
/// `hagency-matrix/src/representative_sync.rs` (task #80). The six TS cases are
/// asserted across the two tests below, in TS order.
///
/// The seams below are the TS test's own injections: `readCursor`/`writeCursor`,
/// `readPendingReconcile`/`writePendingReconcile`, `onEvents`/`onLeaves`/
/// `onReconcile{,Blocked}`/`onCircuitBreak` and the `fetchImpl`/`shouldContinue`
/// pair. One struct is both the durable state and the delivery side, so a case
/// reads top-to-bottom like the test it names.
mod rep_sync {
    use hagency_matrix::{
        BoxFuture, CircuitBreak, Error, EventMeta, HistoryPage, PageReader, PageSink,
        PendingVerdict, RepresentativeSync, SyncBatch, SyncDriver, SyncError, SyncHooks, SyncState,
    };
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeMap, VecDeque},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
        },
    };

    #[derive(Default)]
    pub(super) struct Shared {
        polls: AtomicUsize,
        /// TS `shouldContinue` — the loop runs while `polls < max_polls`.
        max_polls: AtomicUsize,
        /// TS case 2's `shouldContinue: () => cursor !== 'two'`.
        stop_at_cursor: Mutex<Option<String>>,
        /// TS an explicit `collector.stop()` (the abort gate).
        stop: AtomicBool,
        /// TS case 3 aborts the collector from INSIDE the fetch.
        stop_after_poll: AtomicBool,
        batches: Mutex<VecDeque<Value>>,
        /// TS the mock `fetchImpl` answers EVERY poll: the last scripted body is
        /// re-served once the queue drains.
        last: Mutex<Option<Value>>,
        cursor: Mutex<Option<String>>,
        /// Every `since` a poll was made with, in order (TS asserts the first is
        /// null and the second is `'one'`).
        sinces: Mutex<Vec<Option<String>>>,
        cursors: Mutex<Vec<String>>,
        /// room -> pending `from` (TS `stored.pending[roomId].from`).
        pending: Mutex<BTreeMap<String, Option<String>>>,
        /// The order durable writes happened in (TS case 4: `['pending','cursor','cleared']`).
        order: Mutex<Vec<String>>,
        delivered: Mutex<Vec<Vec<String>>>,
        txn_ids: Mutex<Vec<String>>,
        provenances: Mutex<Vec<Value>>,
        leaves: Mutex<Vec<Vec<String>>>,
        blocked: Mutex<Vec<(String, String)>>,
        circuits: Mutex<Vec<CircuitBreak>>,
        reconciles: AtomicUsize,
        /// TS case 1: `onReconcile` IS `reconcileRepresentativeTimeline({from:null,
        /// knownEventIds:[]})`, which always refuses with an unprovable boundary.
        reconcile_boundary: AtomicBool,
        /// TS case 4: the first N reconciles fail transiently.
        reconcile_transient: AtomicUsize,
        /// TS cases 2 and 5: the delivery lane refuses.
        poison_delivery: AtomicBool,
        /// TS case 2: the FIRST delivery of this event id refuses, then succeeds.
        refuse_event: Mutex<Option<String>>,
        refused: AtomicBool,
        sleeps: Mutex<Vec<u64>>,
    }

    pub(super) struct Fixture {
        pub(super) shared: Arc<Shared>,
    }
    impl Fixture {
        pub(super) fn new() -> Self {
            Self {
                shared: Arc::new(Shared {
                    max_polls: AtomicUsize::new(usize::MAX),
                    ..Shared::default()
                }),
            }
        }
        pub(super) fn max_polls(&self, polls: usize) -> &Self {
            self.shared.max_polls.store(polls, SeqCst);
            self
        }
        pub(super) fn stop_at_cursor(&self, cursor: &str) -> &Self {
            *self.shared.stop_at_cursor.lock().unwrap() = Some(cursor.to_owned());
            self
        }
        pub(super) fn cursor(&self, value: Option<&str>) -> &Self {
            *self.shared.cursor.lock().unwrap() = value.map(str::to_owned);
            self
        }
        pub(super) fn script(&self, body: Value) -> &Self {
            self.shared.batches.lock().unwrap().push_back(body);
            self
        }
        pub(super) fn pause(&self, room: &str, from: Option<&str>) -> &Self {
            self.shared
                .pending
                .lock()
                .unwrap()
                .insert(room.to_owned(), from.map(str::to_owned));
            self
        }
        pub(super) fn boundary(&self, refused: bool) -> &Self {
            self.shared.reconcile_boundary.store(refused, SeqCst);
            self
        }
        pub(super) fn transient(&self, failures: usize) -> &Self {
            self.shared.reconcile_transient.store(failures, SeqCst);
            self
        }
        pub(super) fn poison(&self, poison: bool) -> &Self {
            self.shared.poison_delivery.store(poison, SeqCst);
            self
        }
        pub(super) fn refuse_first(&self, event_id: &str) -> &Self {
            *self.shared.refuse_event.lock().unwrap() = Some(event_id.to_owned());
            self
        }
        pub(super) fn collector(&self) -> RepresentativeSync {
            RepresentativeSync::new(
                "project.test",
                "project.test@generation",
                "@rep:project.test",
            )
            .unwrap()
        }
        /// The same collectors run loop, on this fixture's own seams.
        pub(super) async fn run(&self, collector: &mut RepresentativeSync) {
            let driver = Driver {
                shared: self.shared.clone(),
            };
            collector.run(&driver, self, self).await;
        }
    }

    /// TS `expect(delivered.map(events => events.map(e => e.event_id || e.content.membership)))`.
    fn labels(events: &[Value]) -> Vec<String> {
        events
            .iter()
            .map(|event| {
                event
                    .get("event_id")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        event
                            .get("content")
                            .and_then(|c| c.get("membership"))
                            .and_then(Value::as_str)
                    })
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    /// TS the injected `fetchImpl` + `shouldContinue` + `sleep`.
    struct Driver {
        shared: Arc<Shared>,
    }
    impl Driver {
        fn allowed(&self) -> bool {
            if self.shared.stop.load(SeqCst) {
                return false;
            }
            if self.shared.polls.load(SeqCst) >= self.shared.max_polls.load(SeqCst) {
                return false;
            }
            if let Some(stop) = self.shared.stop_at_cursor.lock().unwrap().as_deref()
                && self.shared.cursor.lock().unwrap().as_deref() == Some(stop)
            {
                return false;
            }
            true
        }
    }
    impl SyncDriver for Driver {
        fn active(&self) -> bool {
            self.allowed()
        }
        fn sync_once(&self, since: Option<String>) -> BoxFuture<'_, Result<SyncBatch, SyncError>> {
            let shared = self.shared.clone();
            Box::pin(async move {
                shared.polls.fetch_add(1, SeqCst);
                shared.sinces.lock().unwrap().push(since.clone());
                let body = match shared.batches.lock().unwrap().pop_front() {
                    Some(body) => {
                        *shared.last.lock().unwrap() = Some(body.clone());
                        body
                    }
                    None => shared
                        .last
                        .lock()
                        .unwrap()
                        .clone()
                        .ok_or_else(|| SyncError::new("no scripted batch"))?,
                };
                let batch = SyncBatch::from_body(&body, since.as_deref())?;
                // TS case 3: the fixture calls `collector.stop()` while the poll
                // is still in flight. The response arrives, but the abort gate
                // has already closed, so the loop must discard it.
                if shared.stop_after_poll.load(SeqCst) {
                    shared.stop.store(true, SeqCst);
                }
                Ok(batch)
            })
        }
        fn sleep(&self, ms: u64) -> BoxFuture<'_, ()> {
            let shared = self.shared.clone();
            Box::pin(async move {
                shared.sleeps.lock().unwrap().push(ms);
            })
        }
    }

    impl SyncState for Fixture {
        fn cursor(&self) -> BoxFuture<'_, Option<String>> {
            let shared = self.shared.clone();
            Box::pin(async move { shared.cursor.lock().unwrap().clone() })
        }
        fn write_cursor(&self, cursor: String) -> BoxFuture<'_, Result<(), Error>> {
            let shared = self.shared.clone();
            Box::pin(async move {
                *shared.cursor.lock().unwrap() = Some(cursor.clone());
                shared.cursors.lock().unwrap().push(cursor);
                shared.order.lock().unwrap().push("cursor".into());
                Ok(())
            })
        }
        fn pending(&self) -> BoxFuture<'_, Vec<String>> {
            let shared = self.shared.clone();
            Box::pin(async move { shared.pending.lock().unwrap().keys().cloned().collect() })
        }
        fn write_pending(
            &self,
            room: String,
            verdict: PendingVerdict,
            from: Option<String>,
        ) -> BoxFuture<'_, Result<(), Error>> {
            let shared = self.shared.clone();
            Box::pin(async move {
                let mut pending = shared.pending.lock().unwrap();
                match verdict {
                    PendingVerdict::Pending => {
                        pending.insert(room, from);
                        shared.order.lock().unwrap().push("pending".into());
                    }
                    PendingVerdict::Cleared => {
                        pending.remove(&room);
                        shared.order.lock().unwrap().push("cleared".into());
                    }
                }
                Ok(())
            })
        }
        fn record_observed(&self, _events: Vec<Value>) -> BoxFuture<'_, ()> {
            Box::pin(async move {})
        }
    }

    impl SyncHooks for Fixture {
        fn events(&self, events: Vec<Value>, meta: EventMeta) -> BoxFuture<'_, Result<(), Error>> {
            let shared = self.shared.clone();
            Box::pin(async move {
                shared.delivered.lock().unwrap().push(labels(&events));
                shared.txn_ids.lock().unwrap().push(meta.txn_id);
                shared.provenances.lock().unwrap().push(meta.provenance);
                if shared.poison_delivery.load(SeqCst) {
                    return Err(Error::Remote(500));
                }
                // TS case 2: `if (events[0].event_id === '$work' && !failed)`.
                let refuse = shared.refuse_event.lock().unwrap().clone();
                if refuse.as_deref()
                    == events
                        .first()
                        .and_then(|e| e.get("event_id"))
                        .and_then(Value::as_str)
                    && !shared.refused.swap(true, SeqCst)
                {
                    return Err(Error::Remote(500));
                }
                Ok(())
            })
        }
        fn leaves(&self, rooms: Vec<String>) -> BoxFuture<'_, Result<(), Error>> {
            let shared = self.shared.clone();
            Box::pin(async move {
                shared.leaves.lock().unwrap().push(rooms);
                Ok(())
            })
        }
        fn reconcile(&self, _room: String) -> BoxFuture<'_, Result<(), SyncError>> {
            let shared = self.shared.clone();
            let boundary = self.shared.reconcile_boundary.load(SeqCst);
            let transient = self.shared.reconcile_transient.load(SeqCst);
            Box::pin(async move {
                let n = shared.reconciles.fetch_add(1, SeqCst);
                if boundary {
                    return Err(SyncError::boundary(
                        "sync gap has no proven cursor/event boundary",
                    ));
                }
                if n < transient {
                    return Err(SyncError::new("transient history failure"));
                }
                Ok(())
            })
        }
        fn reconcile_blocked(&self, room: String, reason: String) -> BoxFuture<'_, ()> {
            let shared = self.shared.clone();
            Box::pin(async move {
                shared.blocked.lock().unwrap().push((room, reason));
            })
        }
        fn circuit_break(&self, detail: CircuitBreak) -> BoxFuture<'_, ()> {
            let shared = self.shared.clone();
            Box::pin(async move {
                shared.circuits.lock().unwrap().push(detail);
            })
        }
    }

    /// TS case 1 (`tests/representative-sync.test.js:12`): an unprovable gap is
    /// reported ONCE and preserves blocked recovery.
    pub(super) async fn gap_reported_once_and_preserved() {
        let f = Fixture::new();
        f.max_polls(3)
            .cursor(Some("existing"))
            .pause("!p:project.test", None)
            .script(json!({"next_batch": "next-1", "rooms": {}}));
        f.boundary(true);
        let mut collector = f.collector();
        f.run(&mut collector).await;
        // TS `expect(polls).toBe(3)`: the mock polled three times. The third
        // response is discarded by `shouldContinue`, so it is not a processed poll.
        assert_eq!(f.shared.polls.load(SeqCst), 3);
        assert_eq!(collector.stats().polls, 2);
        // Reported once: the second and third iterations skip the blocked room.
        assert_eq!(f.shared.reconciles.load(SeqCst), 1);
        let blocked = f.shared.blocked.lock().unwrap();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].0, "!p:project.test");
        assert!(blocked[0].1.contains("no proven"), "{}", blocked[0].1);
        // The gap REMAINS recorded: recovery was never cleared.
        assert!(
            f.shared
                .pending
                .lock()
                .unwrap()
                .contains_key("!p:project.test")
        );
    }

    /// TS case 2 (`:33`): invites are delivered and a refused delivery is
    /// retried before the cursor is committed.
    pub(super) async fn invites_delivered_before_cursor() {
        let f = Fixture::new();
        let invite = json!({"type": "m.room.member", "sender": "@borrower:project.test",
            "state_key": "@rep:project.test", "content": {"membership": "invite"}});
        let work = json!({"type": "m.room.message", "event_id": "$work",
            "sender": "@borrower:project.test", "content": {"body": "work"}});
        f.stop_at_cursor("two")
            .script(json!({"next_batch": "one", "rooms": {
                "invite": {"!p:project.test": {"invite_state": {"events": [invite]}}},
                "join": {"!old:project.test": {"timeline": {"events": [
                    {"type": "m.room.message", "event_id": "$old", "content": {"body": "old"}}]}}}}}))
            .script(json!({"next_batch": "two", "rooms": {
                "join": {"!p:project.test": {"timeline": {"events": [work.clone()]}}}}}))
            // The retry re-serves the SAME response — TS's mock answers poll 3
            // with `next_batch: 'two'` again, so the cursor lands on 'two'.
            .script(json!({"next_batch": "two", "rooms": {
                "join": {"!p:project.test": {"timeline": {"events": [work]}}}}}))
            .refuse_first("$work");
        let mut collector = f.collector();
        f.run(&mut collector).await;
        // The refused $work is delivered AGAIN, so the cursor only advanced twice.
        assert_eq!(
            f.shared.delivered.lock().unwrap().as_slice(),
            [
                vec!["invite".to_owned()],
                vec!["$work".to_owned()],
                vec!["$work".to_owned()]
            ]
        );
        assert_eq!(f.shared.cursors.lock().unwrap().as_slice(), ["one", "two"]);
        assert_eq!(collector.stats().failed, 1);
        assert_eq!(collector.stats().batch_attempts, 0);
        // The FIRST poll carries no `since` (an initial sync replays no history);
        // the retry carries the cursor that was still committed.
        assert_eq!(
            f.shared.sinces.lock().unwrap().as_slice(),
            [None, Some("one".to_owned()), Some("one".to_owned())]
        );
        // TS `meta.provenance` is the built side provenance; txn is cursor-named.
        assert_eq!(
            f.shared.provenances.lock().unwrap().first().cloned(),
            Some(json!({"registration": "project.test@generation",
                "sideId": "project.test", "mode": "sync"}))
        );
        // TS `txnId: \`representative-sync-${result.nextBatch}\`` (:61).
        assert_eq!(
            f.shared.txn_ids.lock().unwrap().first().map(String::as_str),
            Some("representative-sync-one")
        );
    }

    /// TS case 3 (`:71`): a stopped poll never dispatches stale results — and the
    /// production driver's own `stop()` gate behaves the same.
    pub(super) async fn stopped_poll_discards() {
        let f = Fixture::new();
        f.script(json!({"next_batch": "stale", "rooms": {
            "join": {"!p:project.test": {"timeline": {"events": [
                {"type": "m.room.message", "event_id": "$stale", "content": {}}]}}}}}));
        f.shared.stop_after_poll.store(true, SeqCst);
        let mut collector = f.collector();
        f.run(&mut collector).await;
        assert!(f.shared.delivered.lock().unwrap().is_empty());
        assert!(f.shared.cursors.lock().unwrap().is_empty());
        // The poll happened and was discarded: exactly one `since` was spent,
        // and the cursor it would have committed is still uncommitted.
        assert_eq!(f.shared.sinces.lock().unwrap().as_slice(), [None]);

        let cancel = hagency_matrix::CancellationToken::new();
        let http = hagency_matrix::RepresentativeHttp::new(
            "https://side.example.test",
            "synthetic-representative-token-not-real",
            "@rep:project.test",
            &hagency_matrix::Limits::default(),
            &cancel,
        )
        .unwrap();
        assert!(SyncDriver::active(&http));
        http.stop();
        assert!(!SyncDriver::active(&http));
        // A side URL that is not a bare https origin is refused, like every
        // other host client here.
        assert!(
            hagency_matrix::RepresentativeHttp::new(
                "http://side.example.test/",
                "synthetic-representative-token-not-real",
                "@rep:project.test",
                &hagency_matrix::Limits::default(),
                &cancel,
            )
            .is_err()
        );
    }

    /// TS case 4 (`:94`): a pending gap is persisted BEFORE the cursor commits,
    /// and recovery is retried until it clears.
    pub(super) async fn pending_gap_persisted_before_cursor() {
        let f = Fixture::new();
        f.max_polls(2)
            .cursor(Some("previous"))
            .script(
                json!({"next_batch": "next-1", "rooms": {"join": {"!p:project.test": {
                "timeline": {"limited": true, "prev_batch": "gap-page", "events": []}}}}}),
            )
            .script(
                json!({"next_batch": "next-2", "rooms": {"join": {"!p:project.test": {
                "timeline": {"limited": false, "events": []}}}}}),
            );
        f.transient(1);
        let mut collector = f.collector();
        f.run(&mut collector).await;
        assert_eq!(
            f.shared.order.lock().unwrap().as_slice(),
            ["pending", "cursor", "cleared"]
        );
        // The recorded gap carried its page token (TS `from: prev_batch`).
        assert!(f.shared.pending.lock().unwrap().is_empty());
    }

    /// TS case 5 (`:114`): poison delivery circuit-breaks after 8 attempts
    /// without consuming the cursor.
    pub(super) async fn circuit_break_holds_cursor() {
        let f = Fixture::new();
        f.cursor(Some("held")).script(
            json!({"next_batch": "uncommitted", "rooms": {"join": {"!p:project.test": {
                "timeline": {"events": [
                    {"type": "m.room.message", "event_id": "$failed", "content": {}}]}}}}}),
        );
        f.poison(true);
        let mut collector = f.collector();
        f.run(&mut collector).await;
        assert_eq!(collector.stats().polls, 8);
        assert!(collector.stats().gave_up);
        let circuits = f.shared.circuits.lock().unwrap();
        assert_eq!(circuits.len(), 1);
        assert_eq!(circuits[0].attempts, 8);
        assert_eq!(circuits[0].held_cursor.as_deref(), Some("held"));
        assert!(f.shared.cursors.lock().unwrap().is_empty());
    }

    /// TS case 6 (`:129`): recovery stops at a recorded event and never guesses
    /// a history boundary.
    pub(super) async fn boundary_reached_or_refused() {
        struct Pages {
            pages: Mutex<VecDeque<HistoryPage>>,
            calls: Mutex<Vec<String>>,
        }
        impl PageReader for Pages {
            fn read(&self, from: String) -> BoxFuture<'_, Result<HistoryPage, SyncError>> {
                Box::pin(async move {
                    self.calls.lock().unwrap().push(from);
                    self.pages
                        .lock()
                        .unwrap()
                        .pop_front()
                        .ok_or_else(|| SyncError::new("no page"))
                })
            }
        }
        #[derive(Default)]
        struct Sink {
            seen: Mutex<Vec<Vec<String>>>,
        }
        impl PageSink for Sink {
            fn deliver(&self, events: Vec<Value>) -> BoxFuture<'_, Result<(), Error>> {
                Box::pin(async move {
                    self.seen.lock().unwrap().push(labels(&events));
                    Ok(())
                })
            }
        }
        let event = |id: &str| json!({"type": "m.room.message", "event_id": id});
        let pages = Pages {
            pages: Mutex::new(
                [
                    HistoryPage {
                        known: true,
                        reason: None,
                        chunk: vec![event("$newer")],
                        end: Some("older-page".into()),
                    },
                    HistoryPage {
                        known: true,
                        reason: None,
                        chunk: vec![
                            event("$older"),
                            event("$already-observed"),
                            event("$historical"),
                        ],
                        end: Some("end".into()),
                    },
                ]
                .into_iter()
                .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        };
        let sink = Sink::default();
        hagency_matrix::reconcile_timeline(
            Some("gap-page"),
            &["$already-observed".to_owned()],
            &pages,
            &sink,
        )
        .await
        .unwrap();
        assert_eq!(
            pages.calls.lock().unwrap().as_slice(),
            ["gap-page", "older-page"]
        );
        // Only the events AFTER the recorded boundary, oldest first.
        assert_eq!(
            sink.seen.lock().unwrap().as_slice(),
            [vec!["$older", "$newer"]]
        );

        // A walk that never reaches a recorded event refuses retryably and
        // delivers NOTHING — it never guesses a boundary.
        let sink = Sink::default();
        let unprovable = Pages {
            pages: Mutex::new(
                [HistoryPage {
                    known: true,
                    reason: None,
                    chunk: vec![event("$unsafe-history")],
                    end: None,
                }]
                .into_iter()
                .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        };
        let error = hagency_matrix::reconcile_timeline(
            Some("gap-page"),
            &["$absent".to_owned()],
            &unprovable,
            &sink,
        )
        .await
        .unwrap_err();
        assert!(error.boundary);
        assert!(error.message.contains("did not reach"), "{}", error.message);
        assert!(sink.seen.lock().unwrap().is_empty());

        // No cursor / no observed ids at all is the same unprovable refusal.
        let sink = Sink::default();
        for (from, known) in [
            (None, vec!["$x".to_owned()]),
            (Some("gap-page"), Vec::new()),
        ] {
            let error = hagency_matrix::reconcile_timeline(from, &known, &unprovable, &sink)
                .await
                .unwrap_err();
            assert!(error.boundary);
        }
        assert!(sink.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ts_oracle_representative_sync_gap_and_invite_and_cursor() {
        gap_reported_once_and_preserved().await;
        invites_delivered_before_cursor().await;
        stopped_poll_discards().await;
    }

    #[tokio::test]
    async fn ts_oracle_representative_sync_pending_gap_circuit_break_boundary() {
        pending_gap_persisted_before_cursor().await;
        circuit_break_holds_cursor().await;
        boundary_reached_or_refused().await;
    }
}

/// TS `tests/invited-room-routing.test.js` — `onInvitedAgentRoomMessage`
/// (bridge-matrix.js:6873). Native has no invited-room message routing surface.

#[test]
#[ignore = "parity gap: no native invited-room message routing (bridge onInvitedAgentRoomMessage)"]
fn ts_oracle_invited_room_routing_direct_and_group_and_loop() {
    // TS asserts: a direct-room command replies through the admitted agent and a
    // send failure does not wedge sync; a joined agent without a starting device
    // cannot wedge another device; invited group routing wakes only mentioned
    // targets and never loops agent output.
    let _ = ();
}

/// TS `tests/join-backfill.test.js` — `pendingJoinBackfill`
/// (bridge-matrix.js:2940). **Covered since board #79**: the selector is
/// ported 1:1 as `hagency_matrix::join_backfill::pending_join_backfill`, and
/// all 13 TS cases are real green assertions in
/// `native/hagency-matrix/src/join_backfill.rs::tests`
/// (`ts_oracle_backfill_*`). This stub stays only as the map entry; see
/// `.peer/report-79.md`.

#[test]
fn ts_oracle_join_backfill_covered_in_hagency_matrix() {
    use hagency_matrix::join_backfill::{Boundary, pending_join_backfill};
    use serde_json::json;
    // One smoke assertion per TS property group; the full 13-case port is the
    // source module's own test set (same names, same fixtures).
    let bot = "@bot:matrix.test";
    let member = |kind: &str| {
        json!({"type":"m.room.member","sender":"@o:m.test","origin_server_ts":1,
            "state_key":bot,"content":{"membership":kind}})
    };
    // invite..end window (:141) and delivery between invite and join (:123).
    let chunk = json!([
        {"type":"m.room.message","event_id":"$b","sender":"@lin:m.test","origin_server_ts":2,"content":{"body":"b"}},
        member("invite")
    ]);
    let window = pending_join_backfill(Some(&chunk), bot, None);
    assert_eq!(window.events, vec!["$b".to_owned()]);
    assert_eq!(window.boundary, Boundary::InviteToEnd);
    // Fail-closed: no provable invite -> nothing routed (:150).
    let none = json!([{"type":"m.room.message","event_id":"$x","sender":"@lin:m.test","origin_server_ts":1,"content":{"body":"x"}}]);
    assert_eq!(
        pending_join_backfill(Some(&none), bot, None).boundary,
        Boundary::Unproven
    );
}

/// TS `tests/bot-commands-request.test.js` — `cmdRequest` reply half.
/// **Covered since board #79**: `bot_commands.rs::request_reply` renders the
/// TS text 1:1; `dispatch` returns `Dispatched::Request(args)` and
/// `bootstrap/commands.rs` renders it through the production command-notice
/// send path. The submit credential half (requester-token preference) has no
/// native surface: admission is Matrix-event-verified
/// (`authority.rs::verify_request` enforces request-id = authenticated event
/// id, `source.event_id == request.source_event_id`, and requester = sender
/// `source.sender == request.requester_mxid`, authority.rs:222-224), not an
/// agent-token POST — that one row stays a gap.
use hagency::bot_commands::{
    Acl, Dispatched, HostObservation, OfferServing, RequestEngagement, RequestOutcome, dispatch,
    request_reply,
};
use serde_json::json;

/// TS `bot-commands-request.test.js:343` `a malformed token amount is refused
/// without reaching the backend`: the reply names the offending word and no
/// engagement is created — the validation is synchronous (:526-536).
#[test]
fn ts_oracle_request_malformed_token_refused_before_backend() {
    let reply = request_reply(&["coding".into(), "four-hundred-thousand".into()], None);
    assert_eq!(reply.plain, "Not a token amount: four-hundred-thousand");
}

/// TS `:365` `a pending request is told WHY, in the project's own terms` —
/// `notWhitelisted` vs `overCeiling` are different next steps (:619-628).
#[test]
fn ts_oracle_request_pending_told_why() {
    let pending = |route: &str| RequestOutcome {
        engagement: Some(RequestEngagement {
            role: "coding".into(),
            requested_tokens: 400_000,
            allocated_tokens: 0,
            agent: "claude-agent".into(),
            auto_joined: false,
            route: Some(route.into()),
        }),
        ..RequestOutcome::default()
    };
    assert_eq!(
        request_reply(
            &["coding".into(), "400000".into()],
            Some(&pending("notWhitelisted"))
        )
        .plain,
        "Requested coding for 400000 tokens — awaiting a decision, because this room is not on the contributor's whitelist."
    );
    assert_eq!(
        request_reply(
            &["coding".into(), "400000".into()],
            Some(&pending("overCeiling"))
        )
        .plain,
        "Requested coding for 400000 tokens — awaiting a decision, because the amount is above what the serving agent has left."
    );
}

/// TS `:295` `the reply names the framework, model and reasoning level`;
/// `:312` a failure does NOT hand the project the provider's configuration
/// (`HAGENCY_*` never appears); `:330` an unknown configuration degrades to
/// the agent alone, not to a fabricated one (:596-612).
#[test]
fn ts_oracle_request_auto_join_disclosure() {
    let base = RequestEngagement {
        role: "coding".into(),
        requested_tokens: 400_000,
        allocated_tokens: 400_000,
        agent: "claude-agent".into(),
        auto_joined: true,
        route: None,
    };
    let disclosed = RequestOutcome {
        engagement: Some(base.clone()),
        serving: Some(OfferServing {
            framework: Some("claude".into()),
            model: Some("claude-opus-5".into()),
            reasoning: Some("high".into()),
            tier: Some("strong".into()),
            provisioning_required: true,
        }),
        ..RequestOutcome::default()
    };
    let reply = request_reply(&["coding".into(), "400000".into()], Some(&disclosed));
    assert_eq!(
        reply.plain,
        "Joined automatically as coding — 400000 tokens, served by claude-agent running claude · claude-opus-5 (high) · strong."
    );
    // `serving: null` degrades to the agent alone, never a fabricated config.
    let unknown = RequestOutcome {
        engagement: Some(base),
        serving: None,
        ..RequestOutcome::default()
    };
    let reply = request_reply(&["coding".into(), "400000".into()], Some(&unknown));
    assert_eq!(
        reply.plain,
        "Joined automatically as coding — 400000 tokens, served by claude-agent."
    );
    assert!(!reply.plain.contains("running"));
    assert!(!reply.plain.contains("HAGENCY_"));
}

/// The dispatch half: a parsed `!request` line is authorized and handed to the
/// caller with its arguments (TS `:365`).
#[test]
fn ts_oracle_request_dispatch_hands_args_to_caller() {
    let acl = Acl::default();
    let observed = HostObservation::default();
    assert_eq!(
        dispatch(
            "!request coding 400000 20000",
            "@a:example.test",
            &acl,
            false,
            &observed
        ),
        Dispatched::Request(vec![
            "coding".to_owned(),
            "400000".to_owned(),
            "20000".to_owned()
        ])
    );
    assert_eq!(
        json!(true),
        json!(matches!(
            dispatch(
                "!request coding 400000",
                "@a:example.test",
                &acl,
                false,
                &observed
            ),
            Dispatched::Request(_)
        ))
    );
}

#[test]
#[ignore = "parity gap: no native requester-token submit credential (no /api/engagements POST)"]
fn ts_oracle_requester_token_preferred_over_operator() {
    // TS asserts: HAGENCY_REQUESTER_TOKEN is preferred over API_TOKEN for the
    // submit. Native has no HTTP submit path to carry either token; admission
    // is Matrix-event-verified instead.
    let _ = ();
}
