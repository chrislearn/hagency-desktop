//! Owner-daemon connector shared by the standalone controller and Hagency.
//!
//! Wire format follows `hagency::daemon::protocol` (API version 1): a 4-byte
//! big-endian length, then one UTF-8 JSON object of at most 64 KiB, over the
//! private socket `<daemon home>/control.sock`. This module depends only on
//! `std`, `libc` and serde so the standalone controller links no Matrix SDK,
//! async runtime or Hagency engine.
//!
//! Safety checks mirror the daemon's `DaemonClient`: the home directory and
//! socket must belong to this user with no group/other access, and the peer
//! process must run as this user. A local connection grants no owner
//! identity; every scoped request names the owner and the daemon enforces it.
//!
//! [`DaemonConnector`] runs one worker thread so UI threads never block. It
//! coalesces queued snapshot reads, keeps the original request ID when
//! looking up an uncertain command, and reports results as [`ConnectorEvent`]s.
use crate::model::{CommandKind, CommandResult, CommandTicket, ConnectionStatus, CommandPhase, PendingCommand};
use hagency_metering::dashboard::{DashboardSnapshot, HostingState, UsageFilter, period::UsagePeriod};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

pub const API_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
pub const MAX_REPLY_FRAME: usize = 4 * 1024 * 1024;
pub const SOCKET: &str = "control.sock";
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Matches the daemon's `CALL_TIMEOUT`: a pause waits for its admission barrier.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(45);
/// Snapshot reads are cheap; a slow one is reported sooner.
pub const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Bound on queued jobs. Further submissions are refused, not buffered.
pub const MAX_QUEUED: usize = 32;

// ----- wire types (client side; tolerant of additive daemon fields) -------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Period {
    Day,
    Week,
    Month,
    #[serde(rename_all = "camelCase")]
    Range { start_ms: u64, end_ms: u64 },
}

impl Period {
    /// `All` has no daemon equivalent and is sent as "no period".
    pub fn from_usage(period: UsagePeriod) -> Option<Self> {
        match period {
            UsagePeriod::Day => Some(Self::Day),
            UsagePeriod::Week => Some(Self::Week),
            UsagePeriod::Month => Some(Self::Month),
            UsagePeriod::Range { start_ms, end_ms } if start_ms < end_ms => {
                Some(Self::Range { start_ms, end_ms })
            }
            UsagePeriod::Range { .. } | UsagePeriod::All => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Command {
    Hello,
    Snapshot { period: Option<Period>, #[serde(rename = "usageFilter", skip_serializing_if = "Option::is_none")] usage_filter: Option<UsageFilter> },
    PauseHosting,
    ResumeHosting,
    StopHosting,
    RefreshProviders,
}

impl Command {
    pub fn from_kind(kind: CommandKind) -> Self {
        match kind {
            CommandKind::Pause => Self::PauseHosting,
            CommandKind::Resume => Self::ResumeHosting,
            CommandKind::Stop => Self::StopHosting,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub owner_id: Option<String>,
    pub expected_revision: Option<u64>,
    pub command: Command,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub version: u32,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    Accepted,
    Completed,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutcome {
    pub state: OutcomeState,
    pub hosting: HostingState,
    pub revision: u64,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub api_version: u32,
    pub dashboard_version: u32,
    pub owner_id: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    pub hosting: HostingState,
}

impl Reply {
    fn typed<T: serde::de::DeserializeOwned>(&self) -> Result<T, ErrorBody> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let invalid = || ErrorBody { code: "invalid_reply".into(), message: String::new(), retryable: false };
        let value = self.result.clone().ok_or_else(invalid)?;
        serde_json::from_value(value).map_err(|_| invalid())
    }
    pub fn snapshot(&self) -> Result<DashboardSnapshot, ErrorBody> {
        self.typed()
    }
    pub fn outcome(&self) -> Result<CommandOutcome, ErrorBody> {
        self.typed()
    }
    pub fn hello(&self) -> Result<Hello, ErrorBody> {
        self.typed()
    }
}

// ----- framing -----------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    Closed,
    TooLarge,
    Io(std::io::ErrorKind),
}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof { Self::Closed } else { Self::Io(e.kind()) }
    }
}

pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    if payload.is_empty() || payload.len() > MAX_FRAME {
        return Err(FrameError::TooLarge);
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

/// Reads one frame; an oversized header is refused before allocating.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    read_bounded(reader, MAX_FRAME)
}

fn read_bounded<R: Read>(reader: &mut R, limit: usize) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > limit {
        return Err(FrameError::TooLarge);
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

// ----- blocking connection -----------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// No daemon is listening. Nothing was sent.
    Unavailable,
    /// The home or socket is not private to this user. Nothing was sent.
    Unsafe,
    /// The listening process is not this user. Nothing was sent.
    PeerRejected,
    /// The request may have been delivered; no reply was read.
    Uncertain,
    /// The reply could not be understood.
    Protocol,
    /// The daemon speaks another protocol version.
    Incompatible { version: Option<u32> },
}

/// Location of one daemon home (`control.sock` lives inside it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub home: PathBuf,
}

impl DaemonEndpoint {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
    pub fn socket(&self) -> PathBuf {
        self.home.join(SOCKET)
    }
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0 as libc::uid_t, 0 as libc::gid_t);
    // SAFETY: valid fd and out-pointers for the duration of the call.
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    (rc == 0).then_some(uid)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred/len describe a writable ucred buffer.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0).then_some(cred.uid)
}

/// Checks that `home` and its socket are private to this user.
pub fn check_private(home: &Path) -> Result<PathBuf, TransportError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let uid = euid();
    let dir = std::fs::symlink_metadata(home).map_err(|_| TransportError::Unavailable)?;
    if !dir.is_dir() || dir.uid() != uid || dir.mode() & 0o077 != 0 {
        return Err(TransportError::Unsafe);
    }
    let path = home.join(SOCKET);
    let socket = std::fs::symlink_metadata(&path).map_err(|_| TransportError::Unavailable)?;
    if !socket.file_type().is_socket() || socket.uid() != uid || socket.mode() & 0o077 != 0 {
        return Err(TransportError::Unsafe);
    }
    Ok(path)
}

/// One private, blocking connection. Use from a worker thread only.
pub struct DaemonConnection {
    stream: UnixStream,
}

impl DaemonConnection {
    pub fn connect(endpoint: &DaemonEndpoint) -> Result<Self, TransportError> {
        let path = check_private(&endpoint.home)?;
        let stream = UnixStream::connect(&path).map_err(|_| TransportError::Unavailable)?;
        if peer_uid(&stream) != Some(euid()) {
            return Err(TransportError::PeerRejected);
        }
        stream.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(|_| TransportError::Unavailable)?;
        Ok(Self { stream })
    }

    /// Send one request and read its reply. Any failure after the request
    /// bytes may have left is `Uncertain`; retry with the same request ID.
    pub fn call(&mut self, request: &Request, timeout: Duration) -> Result<Reply, TransportError> {
        self.call_guarded(request, timeout, None)
    }

    fn call_guarded(&mut self, request: &Request, timeout: Duration, retired: Option<&std::sync::atomic::AtomicBool>) -> Result<Reply, TransportError> {
        let bytes = serde_json::to_vec(request).map_err(|_| TransportError::Protocol)?;
        if bytes.len() > MAX_FRAME {
            return Err(TransportError::Protocol);
        }
        // Final admission point after discovery, connect and serialization.
        // A retired connector must not start another write. Already admitted
        // writes retain their original request ID for outcome reconciliation.
        if retired.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(TransportError::Unavailable);
        }
        if write_frame(&mut self.stream, &bytes).is_err() {
            return Err(TransportError::Uncertain);
        }
        self.stream.set_read_timeout(Some(timeout)).map_err(|_| TransportError::Uncertain)?;
        let payload = read_bounded(&mut self.stream, MAX_REPLY_FRAME).map_err(|_| TransportError::Uncertain)?;
        decode_reply(&payload, &request.request_id)
    }
}

/// Validates a reply payload against the request it answers.
pub fn decode_reply(payload: &[u8], request_id: &str) -> Result<Reply, TransportError> {
    let value: Value = serde_json::from_slice(payload).map_err(|_| TransportError::Protocol)?;
    let version = value.get("version").and_then(Value::as_u64);
    if version != Some(u64::from(API_VERSION)) {
        return Err(TransportError::Incompatible { version: version.and_then(|v| u32::try_from(v).ok()) });
    }
    let reply: Reply = serde_json::from_value(value).map_err(|_| TransportError::Protocol)?;
    // An error for an unparsable request carries an empty ID.
    if reply.request_id != request_id && !(reply.request_id.is_empty() && reply.error.is_some()) {
        return Err(TransportError::Protocol);
    }
    if reply.error.as_ref().is_some_and(|e| e.code == "unsupported_version") {
        return Err(TransportError::Incompatible { version: None });
    }
    Ok(reply)
}

/// A fresh request ID in the daemon's charset (`[A-Za-z0-9_-]{1,64}`).
pub fn new_request_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("{prefix}-{nanos:x}-{:x}-{:x}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect()
}

/// Maps a structured command error to a dashboard result. `in_progress` and
/// `outcome_unknown` mean "retry with the same ID", i.e. still uncertain.
pub fn command_error(error: ErrorBody) -> CommandResult {
    match error.code.as_str() {
        "in_progress" | "outcome_unknown" | "invalid_reply" => CommandResult::Uncertain { message: error.message },
        _ => CommandResult::Rejected { code: error.code, message: error.message, retryable: error.retryable },
    }
}

pub fn command_outcome(outcome: &CommandOutcome) -> CommandResult {
    match outcome.state {
        OutcomeState::Accepted => CommandResult::Accepted,
        OutcomeState::Completed => CommandResult::Completed,
        OutcomeState::Unchanged => CommandResult::Unchanged,
    }
}

/// Connection status for a failure to read a snapshot.
pub fn status_for_error(error: &ErrorBody) -> ConnectionStatus {
    match error.code.as_str() {
        "owner_scope_mismatch" | "daemon_owner_root_conflict" => ConnectionStatus::OwnerMismatch,
        "owner_daemon_unavailable" => ConnectionStatus::DaemonUnavailable,
        "unsupported_version" => ConnectionStatus::Incompatible { daemon_version: None },
        "auth_expired" | "reauthentication_required" => ConnectionStatus::AuthExpired,
        code => ConnectionStatus::Failed { message: code.to_owned() },
    }
}

pub fn status_for_transport(error: &TransportError) -> ConnectionStatus {
    match error {
        TransportError::Unavailable => ConnectionStatus::DaemonUnavailable,
        TransportError::Incompatible { version } => ConnectionStatus::Incompatible { daemon_version: *version },
        TransportError::Unsafe => ConnectionStatus::Failed { message: "daemon_socket_not_private".into() },
        TransportError::PeerRejected => ConnectionStatus::Failed { message: "daemon_peer_rejected".into() },
        TransportError::Uncertain => ConnectionStatus::Failed { message: "daemon_no_reply".into() },
        TransportError::Protocol => ConnectionStatus::Failed { message: "daemon_reply_invalid".into() },
    }
}

// ----- nonblocking connector ----------------------------------------------

/// Which owner the connector scopes requests to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerScope {
    /// Standalone: adopt the daemon's enrolled owner reported by `hello`.
    Discover,
    /// Embedded: the host's signed-in account. A different daemon owner is
    /// reported as [`ConnectionStatus::OwnerMismatch`] and never displayed.
    Fixed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Snapshot { period: Option<UsagePeriod>, usage_filter: Option<UsageFilter> },
    Command { kind: CommandKind, ticket: CommandTicket },
    RefreshProviders,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectorEvent {
    /// The owner this connector is scoped to (from `hello` or the host).
    Owner(Option<String>),
    Connection(ConnectionStatus),
    Snapshot { snapshot: DashboardSnapshot, period: Option<UsagePeriod> },
    CommandResult { request_id: String, result: CommandResult },
    PendingCommand(PendingCommand),
}

/// Nonblocking handle. Submit jobs; drain events after `notify` fires.
pub struct DaemonConnector {
    jobs: mpsc::SyncSender<Job>,
    events: Receiver<ConnectorEvent>,
    retired: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for DaemonConnector {
    fn drop(&mut self) { self.retired.store(true, std::sync::atomic::Ordering::Release); }
}

impl DaemonConnector {
    /// Starts the worker. `notify` is called (from the worker thread) after
    /// events are queued; Makepad hosts pass `SignalToUI::set_ui_signal`.
    pub fn spawn(endpoint: DaemonEndpoint, scope: OwnerScope, notify: impl Fn() + Send + 'static) -> Self {
        let (jobs, job_rx) = mpsc::sync_channel(MAX_QUEUED);
        let (event_tx, events) = mpsc::channel();
        let retired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_retired = retired.clone();
        let failure = event_tx.clone();
        if std::thread::Builder::new()
            .name("owner-daemon-connector".into())
            .spawn(move || Worker { endpoint, scope, owner: None, connection: None, events: event_tx, notify: Box::new(notify), retired: worker_retired }.run(job_rx))
            .is_err() {
            let _ = failure.send(ConnectorEvent::Connection(ConnectionStatus::Failed { message: "connector_unavailable".into() }));
        }
        Self { jobs, events, retired }
    }

    /// Queue a job. Returns false when the queue is full or the worker ended.
    pub fn submit(&self, job: Job) -> bool {
        self.jobs.try_send(job).is_ok()
    }

    pub fn try_recv(&self) -> Option<ConnectorEvent> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

struct Worker {
    endpoint: DaemonEndpoint,
    scope: OwnerScope,
    /// Owner resolved for the current connection (hello).
    owner: Option<Option<String>>,
    connection: Option<DaemonConnection>,
    events: Sender<ConnectorEvent>,
    notify: Box<dyn Fn() + Send>,
    retired: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Worker {
    fn emit(&self, event: ConnectorEvent) {
        let _ = self.events.send(event);
        (self.notify)();
    }

    fn run(mut self, jobs: Receiver<Job>) {
        loop {
            let job = match jobs.recv_timeout(Duration::from_secs(3600)) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            // Coalesce: keep every command, but only the latest snapshot read.
            let mut batch = vec![job];
            while let Ok(job) = jobs.try_recv() {
                batch.push(job);
            }
            let last_snapshot = batch.iter().rposition(|j| matches!(j, Job::Snapshot { .. }));
            for (i, job) in batch.into_iter().enumerate() {
                if self.retired.load(std::sync::atomic::Ordering::Acquire) { return }
                if matches!(job, Job::Snapshot { .. }) && Some(i) != last_snapshot {
                    continue;
                }
                self.handle(job);
            }
        }
    }

    fn call(&mut self, request: &Request, timeout: Duration) -> Result<Reply, TransportError> {
        if self.retired.load(std::sync::atomic::Ordering::Acquire) { return Err(TransportError::Unavailable); }
        if self.connection.is_none() {
            self.connection = Some(DaemonConnection::connect(&self.endpoint)?);
        }
        let result = self.connection.as_mut().map(|c| c.call_guarded(request, timeout, Some(&self.retired))).unwrap_or(Err(TransportError::Unavailable));
        if result.is_err() {
            // The daemon closes idle connections and caps requests per
            // connection; reconnect lazily on the next job.
            self.connection = None;
            self.owner = None;
        }
        result
    }

    /// Calls a read-only request, reconnecting once if a kept connection was
    /// closed by the daemon in the meantime (safe: reads are idempotent).
    fn call_read(&mut self, request: &Request) -> Result<Reply, TransportError> {
        let had_connection = self.connection.is_some();
        match self.call(request, READ_TIMEOUT) {
            Err(TransportError::Uncertain) if had_connection => self.call(request, READ_TIMEOUT),
            other => other,
        }
    }

    /// Resolve the owner for this connection. `Err` carries the status to show.
    fn resolve_owner(&mut self) -> Result<Option<String>, ConnectionStatus> {
        if let Some(owner) = &self.owner {
            return Ok(owner.clone());
        }
        let request = Request {
            version: API_VERSION,
            request_id: new_request_id("odh"),
            owner_id: None,
            expected_revision: None,
            command: Command::Hello,
        };
        let reply = self.call_read(&request).map_err(|e| status_for_transport(&e))?;
        let hello = reply.hello().map_err(|e| status_for_error(&e))?;
        if hello.api_version != API_VERSION || hello.dashboard_version != hagency_metering::dashboard::DASHBOARD_API_VERSION {
            return Err(ConnectionStatus::Incompatible { daemon_version: Some(hello.api_version) });
        }
        let owner = match (&self.scope, hello.owner_id) {
            (OwnerScope::Fixed(mine), Some(theirs)) if *mine != theirs => return Err(ConnectionStatus::OwnerMismatch),
            (OwnerScope::Fixed(mine), _) => Some(mine.clone()),
            (OwnerScope::Discover, theirs) => theirs,
        };
        self.emit(ConnectorEvent::Owner(owner.clone()));
        self.owner = Some(owner.clone());
        Ok(owner)
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Snapshot { period, usage_filter } => self.snapshot(period, usage_filter),
            Job::Command { kind, ticket } => self.command(kind, ticket),
            Job::RefreshProviders => {
                let owner = match self.resolve_owner() {
                    Ok(owner) => owner,
                    Err(status) => return self.emit(ConnectorEvent::Connection(status)),
                };
                let request = Request {
                    version: API_VERSION,
                    request_id: new_request_id("odp"),
                    owner_id: owner,
                    expected_revision: None,
                    command: Command::RefreshProviders,
                };
                // Provider refresh is advisory; the next snapshot shows its effect.
                match self.call(&request, READ_TIMEOUT) {
                    Ok(reply) => if let Some(error) = reply.error { self.emit(ConnectorEvent::Connection(status_for_error(&error))); },
                    Err(error) => self.emit(ConnectorEvent::Connection(status_for_transport(&error))),
                }
            }
        }
    }

    fn snapshot(&mut self, period: Option<UsagePeriod>, usage_filter: Option<UsageFilter>) {
        let period = Some(period.unwrap_or(UsagePeriod::Day));
        let owner = match self.resolve_owner() {
            Ok(owner) => owner,
            Err(status) => return self.emit(ConnectorEvent::Connection(status)),
        };
        let Some(owner_id) = owner else {
            // Daemon running but no owner enrolled: an honest unconfigured view.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            return self.emit(ConnectorEvent::Snapshot { snapshot: DashboardSnapshot::unconfigured(now), period: None });
        };
        if let Ok(store) = crate::command_store::CommandStore::open(&self.endpoint.home, &owner_id) {
            match store.load() {
                Ok(Some(command)) => self.emit(ConnectorEvent::PendingCommand(command)),
                Ok(None) => {},
                Err(_) => return self.emit(ConnectorEvent::Connection(ConnectionStatus::Failed { message: "command_recovery_unavailable".into() })),
            }
        }
        let request = Request {
            version: API_VERSION,
            request_id: new_request_id("ods"),
            owner_id: Some(owner_id),
            expected_revision: None,
            command: Command::Snapshot { period: period.and_then(|period| {
                let clock = crate::Clock::system();
                period.resolve(clock.now_ms, clock.utc_offset_minutes).ok().map(|range|
                    Period::Range { start_ms: range.start_ms, end_ms: range.end_ms })
            }), usage_filter },
        };
        match self.call_read(&request) {
            Err(e) => self.emit(ConnectorEvent::Connection(status_for_transport(&e))),
            Ok(reply) => match reply.snapshot() {
                Ok(snapshot) => {
                    if let Some(owner) = &snapshot.owner {
                        if let Ok(store) = crate::command_store::CommandStore::open(&self.endpoint.home, &owner.user_id) {
                            if let Ok(Some(mut command)) = store.load() {
                                command.observe(&snapshot);
                                if !command.in_flight() { let _ = store.clear(); }
                            }
                        }
                    }
                    let period = period.filter(|p| Period::from_usage(*p).is_some());
                    self.emit(ConnectorEvent::Snapshot { snapshot, period })
                }
                Err(error) if error.code == "owner_not_adopted" => {
                    self.owner = None;
                    self.emit(ConnectorEvent::Snapshot { snapshot: DashboardSnapshot::unconfigured(0), period: None })
                }
                Err(error) => self.emit(ConnectorEvent::Connection(status_for_error(&error))),
            },
        }
    }

    fn command(&mut self, kind: CommandKind, ticket: CommandTicket) {
        let request_id = ticket.request_id.clone();
        // Snapshot polling can exhaust the daemon's per-connection request
        // cap. Mutations always get a fresh connection and owner handshake.
        self.connection = None;
        self.owner = None;
        let owner = match self.resolve_owner() {
            Ok(owner) => owner,
            Err(status) => {
                // Only this attempt was not sent; an earlier one may have run.
                self.emit(ConnectorEvent::CommandResult {
                    request_id,
                    result: CommandResult::NotSubmitted { code: "not_connected".into(), retryable: true },
                });
                return self.emit(ConnectorEvent::Connection(status));
            }
        };
        let Some(owner_id) = owner.as_deref() else {
            return self.emit(ConnectorEvent::CommandResult { request_id, result: CommandResult::NotSubmitted { code: "not_connected".into(), retryable: true } });
        };
        let store = match crate::command_store::CommandStore::open(&self.endpoint.home, owner_id) {
            Ok(store) => store,
            Err(_) => return self.emit(ConnectorEvent::CommandResult { request_id, result: CommandResult::NotSubmitted { code: "command_recovery_unavailable".into(), retryable: true } }),
        };
        let previous = match store.load() {
            Ok(previous) => previous,
            Err(_) => return self.emit(ConnectorEvent::CommandResult { request_id, result: CommandResult::NotSubmitted { code: "command_recovery_unavailable".into(), retryable: false } }),
        };
        if let Some(saved) = &previous {
            if saved.ticket != ticket || saved.kind != kind {
                self.emit(ConnectorEvent::CommandResult { request_id, result: CommandResult::NotSubmitted { code: "pending_command_exists".into(), retryable: false } });
                return self.emit(ConnectorEvent::PendingCommand(saved.clone()));
            }
        }
        let mut saved = previous.clone().unwrap_or(PendingCommand { kind, ticket: ticket.clone(), phase: CommandPhase::Uncertain });
        if store.save(&saved).is_err() {
            return self.emit(ConnectorEvent::CommandResult { request_id, result: CommandResult::NotSubmitted { code: "command_recovery_unavailable".into(), retryable: true } });
        }
        let request = Request {
            version: API_VERSION,
            request_id: ticket.request_id,
            owner_id: owner,
            expected_revision: ticket.expected_revision,
            command: Command::from_kind(kind),
        };
        let result = match self.call(&request, CALL_TIMEOUT) {
            Ok(reply) => match reply.outcome() {
                Ok(outcome) => command_outcome(&outcome),
                Err(error) => command_error(error),
            },
            Err(TransportError::Unavailable | TransportError::Unsafe | TransportError::PeerRejected) => {
                CommandResult::NotSubmitted { code: "not_connected".into(), retryable: true }
            }
            Err(TransportError::Incompatible { .. }) => {
                CommandResult::Uncertain { message: "unsupported_version".into() }
            }
            Err(TransportError::Uncertain | TransportError::Protocol) => CommandResult::Uncertain { message: String::new() },
        };
        // A failed lookup is not evidence that an earlier submission failed.
        // Keep accepted commands until an authoritative later state settles.
        match &result {
            CommandResult::Accepted => { saved.phase = CommandPhase::Accepted; let _ = store.save(&saved); },
            CommandResult::Completed | CommandResult::Unchanged => { let _ = store.clear(); },
            CommandResult::Rejected { .. } | CommandResult::NotSubmitted { .. } if previous.is_none() => { let _ = store.clear(); },
            _ => {},
        }
        self.emit(ConnectorEvent::CommandResult { request_id, result });
    }
}

#[cfg(test)]
mod tests;
