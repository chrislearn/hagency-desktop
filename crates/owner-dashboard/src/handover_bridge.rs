//! Background handover IO shared by both hosts. Hosts restore the view first,
//! then acknowledge; a launch result alone never counts as a successful move.
use crate::{handover::{HandoverStore, Host, Received, Status, Ticket}, ViewState};
use std::{collections::BTreeSet, io, path::PathBuf, sync::mpsc, time::Duration};

pub enum Event {
    Restoring { id: u64, view: ViewState },
    Opening,
    Completed,
    Failed(&'static str),
}

enum Job {
    Owner(Option<String>),
    Offer(ViewState),
    Restored(u64),
    Rejected(u64),
}

pub struct Bridge {
    jobs: mpsc::SyncSender<Job>,
    events: mpsc::Receiver<Event>,
}

impl Bridge {
    pub fn spawn(home: PathBuf, host: Host, notify: impl Fn() + Send + 'static) -> io::Result<Self> {
        Self::spawn_inner(home, host, Launcher::Native, notify)
    }
    /// Real handover transport for already-running, isolated UI fixtures.
    /// Only LaunchServices is bypassed: offer, receive, UI acknowledgment and
    /// source completion all use the production worker and private store.
    #[cfg(any(test, feature = "fixtures"))]
    pub fn spawn_fixture(home: PathBuf, host: Host, notify: impl Fn() + Send + 'static) -> io::Result<Self> {
        let temp = std::fs::canonicalize(std::env::temp_dir())?;
        let home = std::fs::canonicalize(home)?;
        if !home.starts_with(temp) || std::fs::read_to_string(home.join("owner-dashboard-ui-fixture"))? != "owner-dashboard-ui-fixture-v1\n" {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "isolated UI fixture directory required"));
        }
        Self::spawn_inner(home, host, Launcher::RunningFixture, notify)
    }
    fn spawn_inner(home: PathBuf, host: Host, launcher: Launcher, notify: impl Fn() + Send + 'static) -> io::Result<Self> {
        let (jobs, input) = mpsc::sync_channel(16);
        let (output, events) = mpsc::channel();
        std::thread::Builder::new().name("owner-dashboard-handover".into()).spawn(move || {
            Worker { home, host, launcher, owner: None, output, notify: Box::new(notify),
                outgoing: None, incoming: None, seen: BTreeSet::new(), next: 0 }.run(input);
        })?;
        Ok(Self { jobs, events })
    }
    pub fn set_owner(&self, owner: Option<String>) -> bool { self.jobs.try_send(Job::Owner(owner)).is_ok() }
    pub fn offer(&self, view: ViewState) -> bool { self.jobs.try_send(Job::Offer(view)).is_ok() }
    pub fn restored(&self, id: u64) -> bool { self.jobs.try_send(Job::Restored(id)).is_ok() }
    pub fn rejected(&self, id: u64) -> bool { self.jobs.try_send(Job::Rejected(id)).is_ok() }
    pub fn try_recv(&self) -> Option<Event> { self.events.try_recv().ok() }
}

enum Launcher {
    Native,
    #[cfg(any(test, feature = "fixtures"))]
    RunningFixture,
}

struct Worker {
    home: PathBuf,
    host: Host,
    launcher: Launcher,
    owner: Option<String>,
    output: mpsc::Sender<Event>,
    notify: Box<dyn Fn() + Send>,
    outgoing: Option<Ticket>,
    incoming: Option<(u64, Received)>,
    seen: BTreeSet<PathBuf>,
    next: u64,
}

impl Worker {
    fn emit(&self, event: Event) {
        #[cfg(any(test, feature = "fixtures"))]
        if matches!(self.launcher, Launcher::RunningFixture) {
            let value = match &event {
                Event::Restoring { id, view } => serde_json::json!({"event":"restoring","id":id,"view":view}),
                Event::Opening => serde_json::json!({"event":"opening"}),
                Event::Completed => serde_json::json!({"event":"completed"}),
                Event::Failed(code) => serde_json::json!({"event":"failed","code":code}),
            };
            eprintln!("od-handover-fixture {}", serde_json::json!({"host":self.host,"owner":self.owner,"atMs":now(),"detail":value}));
        }
        let _ = self.output.send(event); (self.notify)();
    }
    fn store(&self) -> io::Result<HandoverStore> { HandoverStore::open(self.home.join("handover")) }
    fn run(mut self, input: mpsc::Receiver<Job>) {
        loop {
            match input.recv_timeout(Duration::from_millis(500)) {
                Ok(Job::Owner(owner)) if owner != self.owner => {
                    // Old-account state must never be acknowledged after switching.
                    if let Some(ticket) = self.outgoing.take() {
                        let _ = self.store().and_then(|store| store.remove(&ticket));
                    }
                    self.owner = owner;
                    self.incoming = None;
                    self.outgoing = None;
                    self.seen.clear();
                }
                Ok(Job::Owner(_)) => {},
                Ok(Job::Offer(view)) => self.offer(view),
                Ok(Job::Restored(id)) => self.acknowledge(id),
                Ok(Job::Rejected(id)) => {
                    if self.incoming.as_ref().is_some_and(|(pending, _)| *pending == id) {
                        self.incoming = None;
                        self.emit(Event::Failed("view_restore_failed"));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {},
            }
            self.poll();
        }
    }
    fn offer(&mut self, view: ViewState) {
        let Some(owner) = self.owner.as_deref() else { return self.emit(Event::Failed("sign_in_required")); };
        if view.owner_id.as_deref() != Some(owner) { return self.emit(Event::Failed("owner_scope_mismatch")); }
        if self.outgoing.is_some() { return self.emit(Event::Failed("handover_pending")); }
        let Ok(store) = self.store() else { return self.emit(Event::Failed("handover_store_unavailable")); };
        let target = match self.host { Host::Embedded => Host::Standalone, Host::Standalone => Host::Embedded };
        let Ok(value) = serde_json::to_value(view) else { return self.emit(Event::Failed("view_restore_failed")); };
        let Ok(ticket) = store.offer(owner, self.host, target, value, now()) else { return self.emit(Event::Failed("handover_store_unavailable")); };
        let launched = match self.launcher {
            Launcher::Native => launch(target, &self.home, ticket.path()),
            #[cfg(any(test, feature = "fixtures"))]
            Launcher::RunningFixture => Ok(()),
        };
        if launched.is_err() {
            let _ = store.remove(&ticket);
            return self.emit(Event::Failed("destination_unavailable"));
        }
        self.outgoing = Some(ticket);
        self.emit(Event::Opening);
    }
    fn acknowledge(&mut self, id: u64) {
        let Some((pending, received)) = self.incoming.as_ref() else { return };
        if *pending != id { return }
        let result = self.owner.as_deref().ok_or_else(|| io::Error::other("owner changed"))
            .and_then(|owner| self.store()?.acknowledge(received, owner, now()));
        #[cfg(any(test, feature = "fixtures"))]
        if matches!(self.launcher, Launcher::RunningFixture) {
            eprintln!("od-handover-fixture {}", serde_json::json!({"host":self.host,"owner":self.owner,"atMs":now(),"detail":{"event":"uiAcknowledged","id":id,"accepted":result.is_ok()}}));
        }
        self.incoming = None;
        if result.is_err() { self.emit(Event::Failed("handover_acknowledgement_failed")); }
    }
    fn poll(&mut self) {
        self.poll_at(now());
    }
    fn poll_at(&mut self, now: u64) {
        let Some(owner) = self.owner.as_deref() else { return };
        // An idle observer does not create a daemon root or enrollment files.
        let root = self.home.join("handover");
        if !root.exists() { return }
        let Ok(store) = self.store() else { return };
        if let Some(ticket) = self.outgoing.as_ref() {
            match store.poll(ticket, owner, now) {
                Ok(Status::Waiting) => {},
                outcome => {
                    let _ = store.remove(ticket);
                    self.outgoing = None;
                    match outcome {
                        Ok(Status::Acknowledged) => self.emit(Event::Completed),
                        _ => self.emit(Event::Failed("destination_not_confirmed")),
                    }
                }
            }
        }
        // A hidden/unmounted UI may never acknowledge restoration. Let later
        // requests proceed after this ticket's deadline, without acknowledging it.
        if self.incoming.as_ref().is_some_and(|(_, received)| received.expired(now)) {
            self.incoming = None;
        }
        if self.incoming.is_some() { return }
        let Ok(entries) = std::fs::read_dir(root) else { return };
        for entry in entries.filter_map(Result::ok).take(64) {
            let path = entry.path();
            if self.seen.contains(&path) || path.extension().is_none_or(|ext| ext != "json") { continue }
            let Ok(received) = store.receive(&path, owner, self.host, now) else { continue };
            let Ok(view) = serde_json::from_value::<ViewState>(received.view().clone()) else { continue };
            if view.owner_id.as_deref() != Some(owner) { continue }
            // Bound memory even if a sender continually creates new handovers.
            if self.seen.len() >= 128 { self.seen.clear(); }
            self.seen.insert(path);
            self.next = self.next.wrapping_add(1);
            self.incoming = Some((self.next, received));
            self.emit(Event::Restoring { id: self.next, view });
            break;
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis() as u64).unwrap_or(0)
}

fn launch(target: Host, home: &std::path::Path, path: &std::path::Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let bundle = match target { Host::Embedded => "org.hagency.desktop", Host::Standalone => "io.hagency.owner-controller" };
        let mut command = std::process::Command::new("/usr/bin/open");
        command.arg("-b").arg(bundle);
        if target == Host::Standalone {
            command.arg("--args").arg("--daemon-home").arg(home).arg("--handover-file").arg(path);
        }
        let status = command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).status()?;
        if status.success() { Ok(()) } else { Err(io::Error::other("destination unavailable")) }
    }
    #[cfg(not(target_os = "macos"))]
    { let _ = (target, home, path); Err(io::Error::new(io::ErrorKind::Unsupported, "desktop handover requires a supported launcher")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        let root = std::env::temp_dir().join(format!("od-bridge-{}-{}", std::process::id(), crate::transport::new_request_id("test")));
        std::fs::create_dir(&root).unwrap();
        root
    }
    fn next(bridge: &Bridge) -> Event {
        bridge.events.recv_timeout(Duration::from_secs(3)).expect("handover event")
    }
    #[test]
    fn unanswered_restore_expires_and_later_handover_can_be_received() {
        let root = directory();
        let store = HandoverStore::open(root.join("handover")).unwrap();
        let owner = "@owner:example.test";
        let view = ViewState { owner_id: Some(owner.into()), ..Default::default() };
        let first = store.offer(owner, Host::Standalone, Host::Embedded, serde_json::to_value(&view).unwrap(), 1000).unwrap();
        let (output, events) = mpsc::channel();
        let mut worker = Worker { home: root.clone(), host: Host::Embedded, launcher: Launcher::RunningFixture,
            owner: Some(owner.into()), output, notify: Box::new(|| {}), outgoing: None, incoming: None,
            seen: BTreeSet::new(), next: 0 };
        worker.poll_at(1001);
        assert!(matches!(events.try_recv().unwrap(), Event::Restoring { id: 1, .. }));
        worker.poll_at(121000);
        assert!(worker.incoming.is_none());
        assert_eq!(store.poll(&first, owner, 121000).unwrap(), Status::Expired);
        store.offer(owner, Host::Standalone, Host::Embedded, serde_json::to_value(&view).unwrap(), 121001).unwrap();
        worker.poll_at(121002);
        assert!(matches!(events.try_recv().unwrap(), Event::Restoring { id: 2, .. }));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn account_switch_removes_the_senders_pending_ticket() {
        let root = directory();
        std::fs::write(root.join("owner-dashboard-ui-fixture"), "owner-dashboard-ui-fixture-v1\n").unwrap();
        let bridge = Bridge::spawn_fixture(root.clone(), Host::Standalone, || {}).unwrap();
        bridge.set_owner(Some("@owner:example.test".into()));
        bridge.offer(ViewState { owner_id: Some("@owner:example.test".into()), ..Default::default() });
        assert!(matches!(next(&bridge), Event::Opening));
        bridge.set_owner(Some("@other:example.test".into()));
        bridge.offer(ViewState::default());
        assert!(matches!(next(&bridge), Event::Failed("owner_scope_mismatch")));
        assert_eq!(std::fs::read_dir(root.join("handover")).unwrap().count(), 0);
        drop(bridge);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn receiver_waits_for_ui_restoration_before_acknowledging() {
        let root = directory();
        let store = HandoverStore::open(root.join("handover")).unwrap();
        let view = ViewState { version: 1, owner_id: Some("@owner:example.test".into()), ..ViewState::default() };
        let ticket = store.offer("@owner:example.test", Host::Standalone, Host::Embedded, serde_json::to_value(&view).unwrap(), now()).unwrap();
        let bridge = Bridge::spawn(root.clone(), Host::Embedded, || {}).unwrap();
        bridge.set_owner(view.owner_id.clone());
        let Event::Restoring { id, view: restored } = next(&bridge) else { panic!("expected restore") };
        assert_eq!(view, restored);
        assert_eq!(store.poll(&ticket, "@owner:example.test", now()).unwrap(), Status::Waiting);
        bridge.restored(id);
        for _ in 0..30 {
            if store.poll(&ticket, "@owner:example.test", now()).unwrap() == Status::Acknowledged { break }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(store.poll(&ticket, "@owner:example.test", now()).unwrap(), Status::Acknowledged);
        drop(bridge);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn switching_accounts_cannot_acknowledge_previous_restore() {
        let root = directory();
        let store = HandoverStore::open(root.join("handover")).unwrap();
        let view = ViewState { version: 1, owner_id: Some("@owner:example.test".into()), ..ViewState::default() };
        let ticket = store.offer("@owner:example.test", Host::Standalone, Host::Embedded, serde_json::to_value(view).unwrap(), now()).unwrap();
        let bridge = Bridge::spawn(root.clone(), Host::Embedded, || {}).unwrap();
        bridge.set_owner(Some("@owner:example.test".into()));
        let Event::Restoring { id, .. } = next(&bridge) else { panic!("expected restore") };
        bridge.set_owner(Some("@other:example.test".into()));
        bridge.restored(id);
        // A rejected offer establishes that preceding commands were processed.
        bridge.offer(ViewState::default());
        assert!(matches!(next(&bridge), Event::Failed("owner_scope_mismatch")));
        assert_eq!(store.poll(&ticket, "@owner:example.test", now()).unwrap(), Status::Waiting);
        drop(bridge);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fixture_peers_use_real_offer_receive_and_ui_acknowledgment() {
        let root = directory();
        assert!(Bridge::spawn_fixture(root.clone(), Host::Standalone, || {}).is_err(), "ordinary directories cannot enable the fixture launcher");
        std::fs::write(root.join("owner-dashboard-ui-fixture"), "owner-dashboard-ui-fixture-v1\n").unwrap();
        let sender = Bridge::spawn_fixture(root.clone(), Host::Standalone, || {}).unwrap();
        let receiver = Bridge::spawn_fixture(root.clone(), Host::Embedded, || {}).unwrap();
        let view = ViewState { version: 1, owner_id: Some("@owner:example.test".into()), tab: crate::Tab::Usage, ..Default::default() };
        sender.set_owner(view.owner_id.clone());
        receiver.set_owner(view.owner_id.clone());
        sender.offer(view.clone());
        assert!(matches!(next(&sender), Event::Opening));
        let Event::Restoring { id, view: received } = next(&receiver) else { panic!("actual peer did not receive ticket") };
        assert_eq!(received, view);
        assert!(sender.events.recv_timeout(Duration::from_millis(100)).is_err(), "launch alone cannot complete handover");
        receiver.restored(id);
        assert!(matches!(next(&sender), Event::Completed));
        drop(sender); drop(receiver);
        std::fs::remove_dir_all(root).unwrap();
    }
}
