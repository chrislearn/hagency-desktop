//! Nonblocking, bounded, owner-scoped navigation persistence.
use owner_dashboard::{ViewState, handover::HandoverStore};
use std::{path::PathBuf, sync::mpsc};

pub fn default_daemon_home() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    #[cfg(target_os = "macos")]
    { home.join("Library/Application Support/Hagency/owner-daemon") }
    #[cfg(not(target_os = "macos"))]
    { std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/state")).join("hagency/owner-daemon") }
}

enum Job { Load(String, u64), Save(String, ViewState) }
pub struct ViewStore {
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<(u64, ViewState)>,
}
impl ViewStore {
    pub fn spawn(home: PathBuf, notify: impl Fn() + Send + 'static) -> Self {
        let (jobs, input) = mpsc::sync_channel(16);
        let (output, results) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(job) = input.recv() {
                // Opening a controller must not create daemon enrollment.
                if !home.exists() { continue; }
                let Ok(store) = HandoverStore::open(home.join("controller-view")) else { continue };
                match job {
                    Job::Load(owner, generation) => if let Ok(Some(view)) = store.load_view(&owner) { let _ = output.send((generation, view)); notify(); },
                    Job::Save(owner, view) => { let _ = store.save_view(&owner, &view); },
                }
            }
        });
        Self { jobs, results }
    }
    pub fn load(&self, owner: String, generation: u64) { let _ = self.jobs.try_send(Job::Load(owner, generation)); }
    pub fn save(&self, owner: String, view: ViewState) { let _ = self.jobs.try_send(Job::Save(owner, view)); }
    pub fn try_recv(&self) -> Option<(u64, ViewState)> { self.results.try_recv().ok() }
}
