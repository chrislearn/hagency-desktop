//! Explicit first-run enrollment, entirely outside the UI event thread.
use std::{io, path::PathBuf, process::{ChildStdin, Command, Stdio}, sync::{Arc, Mutex, mpsc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

#[derive(Clone, Debug)]
pub enum Progress { SigningIn, Enabling, Installed, Failed(&'static str) }
#[derive(Default)]
struct Cancellation { stopped: AtomicBool, stdin: Mutex<Option<ChildStdin>> }
pub struct Setup { events: mpsc::Receiver<Progress>, cancel: Arc<Cancellation> }
impl Drop for Setup {
    fn drop(&mut self) {
        self.cancel.stopped.store(true, Ordering::Release);
        // EOF also reaches the helper if the whole app exits or crashes. The
        // helper owns cancellation of browser enrollment, not a UI timer.
        if let Ok(mut pipe) = self.cancel.stdin.lock() { pipe.take(); }
    }
}

pub fn valid_server(server: &str) -> bool {
    server.len() <= 2048 && url::Url::parse(server).is_ok_and(|url|
        url.scheme() == "https" && url.host_str().is_some()
        && url.username().is_empty() && url.password().is_none()
        && url.query().is_none() && url.fragment().is_none())
}
pub fn valid_owner(owner: &str) -> bool {
    owner.is_empty() || (owner.len() <= 512 && !owner.chars().any(char::is_whitespace)
        && owner.strip_prefix('@').and_then(|id| id.split_once(':'))
            .is_some_and(|(name, server)| !name.is_empty() && !server.is_empty()
                && valid_server(&format!("https://{server}")) && !server.contains('/')))
}

fn binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let packaged = exe.parent()?.parent()?.join("Helpers/hagency");
    packaged.is_file().then_some(packaged)
}
fn run(mut command: Command, timeout: Duration, cancel: &Cancellation, enrollment: bool) -> io::Result<()> {
    if cancel.stopped.load(Ordering::Acquire) { return Err(io::ErrorKind::Interrupted.into()); }
    let mut child = command.stdin(if enrollment { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    if enrollment { *cancel.stdin.lock().unwrap_or_else(|e| e.into_inner()) = child.stdin.take(); }
    let start = Instant::now();
    let mut cancellation_started = None;
    let result = loop {
        if cancel.stopped.load(Ordering::Acquire) || start.elapsed() >= timeout {
            cancel.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
            cancellation_started.get_or_insert_with(Instant::now);
        }
        match child.try_wait() {
            Ok(Some(status)) => break if cancellation_started.is_some() { Err(io::ErrorKind::Interrupted.into()) } else if status.success() { Ok(()) } else { Err(io::Error::other("helper_failed")) },
            Err(error) => { if !enrollment { let _ = child.kill(); } let _ = child.wait(); break Err(error); },
            // Enrollment owns accepted native custody. EOF requests cancellation;
            // only that helper can determine when registration/drain is safe.
            Ok(None) if !enrollment && (start.elapsed() >= timeout || cancellation_started.is_some_and(|at| at.elapsed() >= Duration::from_secs(3))) => {
                let _ = child.kill(); let _ = child.wait(); break Err(io::ErrorKind::Interrupted.into());
            },
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    cancel.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
    result
}
impl Setup {
    pub fn start(home: PathBuf, adopt_root: Option<PathBuf>, server: String, owner: Option<String>, install_only: bool, notify: impl Fn() + Send + 'static) -> Self {
        let (send, events) = mpsc::channel();
        let cancel = Arc::new(Cancellation::default());
        let worker_cancel = cancel.clone();
        std::thread::spawn(move || {
            let emit = |event| { let _ = send.send(event); notify(); };
            let Some(binary) = binary() else { return emit(Progress::Failed("helper_missing")); };
            if !install_only {
                emit(Progress::SigningIn);
                let mut enroll = Command::new(&binary);
                enroll.args(["daemon", "enroll", "--cancel-on-stdin-close", "--home"]).arg(&home).arg("--server").arg(server);
                if let Some(owner) = owner { enroll.arg("--owner").arg(owner); }
                if let Some(root) = adopt_root { enroll.arg("--adopt-root").arg(root); }
                if run(enroll, Duration::from_secs(300), &worker_cancel, true).is_err() { return emit(Progress::Failed("sign_in_failed")); }
            }
            emit(Progress::Enabling);
            let mut enable = Command::new(binary);
            enable.args(["daemon", "install", "--home"]).arg(home);
            if run(enable, Duration::from_secs(30), &worker_cancel, false).is_err() { return emit(Progress::Failed("service_start_failed")); }
            // launchctl accepting a job is installation, not executor readiness.
            emit(Progress::Installed);
        });
        Self { events, cancel }
    }
    pub fn try_recv(&self) -> Option<Progress> { self.events.try_recv().ok() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enrollment_timeout_waits_for_accepted_custody_after_eof() {
        let mut command = Command::new("/bin/sh");
        // Exit status 0 only after EOF and a drain longer than the former kill grace.
        command.args(["-c", "cat >/dev/null; sleep 3.2; exit 0"]);
        let start = Instant::now();
        let error = run(command, Duration::from_millis(50), &Cancellation::default(), true).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(start.elapsed() >= Duration::from_millis(3200));
    }
}
