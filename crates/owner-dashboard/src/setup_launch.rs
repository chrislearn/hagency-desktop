//! Explicit setup navigation. A separate setup instance receives startup
//! arguments even when Hosting is already open. Its acknowledgment means only
//! that the setup form was loaded, never that enrollment/hosting succeeded.
use crate::handover::{HandoverStore, Host, Status, Ticket};
use serde_json::json;
use std::{io, path::{Path, PathBuf}, process::Command};

pub struct SetupLaunch {
    store: HandoverStore,
    ticket: Ticket,
    owner: String,
}

fn payload(server: &str, root: &Path) -> serde_json::Value {
    json!({"kind":"setup", "server":server, "adoptRoot":root})
}

impl SetupLaunch {
    pub fn prepare(home: &Path, owner: &str, server: &str, root: &Path, now: u64) -> io::Result<Self> {
        let store = HandoverStore::open(home.join("setup-navigation"))?;
        let ticket = store.offer(owner, Host::Embedded, Host::Standalone, payload(server, root), now)?;
        Ok(Self { store, ticket, owner: owner.into() })
    }

    pub fn command(&self, home: &Path, server: &str, root: &Path) -> Command {
        let mut command = Command::new("/usr/bin/open");
        // macOS only delivers --args to a newly launched process. This window
        // is a controller; the daemon's single-writer custody is unchanged.
        command.args(["-n", "-b", "io.hagency.owner-controller", "--args", "--setup", "--owner"])
            .arg(&self.owner).arg("--server").arg(server).arg("--daemon-home").arg(home)
            .arg("--adopt-root").arg(root).arg("--setup-request").arg(self.ticket.path());
        command
    }

    pub fn status(&self, now: u64) -> io::Result<Status> {
        self.store.poll(&self.ticket, &self.owner, now)
    }
}

impl Drop for SetupLaunch {
    fn drop(&mut self) { let _ = self.store.remove(&self.ticket); }
}

/// Call on an IO worker only after showing the form with these exact values.
pub fn acknowledge(home: PathBuf, request: PathBuf, owner: String, server: String, root: PathBuf, now: u64) -> io::Result<()> {
    let store = HandoverStore::open(home.join("setup-navigation"))?;
    let received = store.receive(&request, &owner, Host::Standalone, now)?;
    if received.view() != &payload(&server, &root) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "setup context changed"));
    }
    store.acknowledge(&received, &owner, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_always_launches_with_arguments_and_waits_for_the_matching_form() {
        let home = std::env::temp_dir().join(crate::transport::new_request_id("setup-test"));
        let root = home.join("owner root");
        let launch = SetupLaunch::prepare(&home, "@owner:test", "https://test", &root, 1000).unwrap();
        let command = launch.command(&home, "https://test", &root);
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(&args[..5], ["-n", "-b", "io.hagency.owner-controller", "--args", "--setup"]);
        assert!(args.contains(&root.to_string_lossy().into_owned()));
        assert_eq!(launch.status(1001).unwrap(), Status::Waiting);
        assert!(acknowledge(home.clone(), launch.ticket.path().into(), "@other:test".into(), "https://test".into(), root.clone(), 1002).is_err());
        assert!(acknowledge(home.clone(), launch.ticket.path().into(), "@owner:test".into(), "https://wrong".into(), root.clone(), 1002).is_err());
        acknowledge(home.clone(), launch.ticket.path().into(), "@owner:test".into(), "https://test".into(), root, 1003).unwrap();
        assert_eq!(launch.status(1004).unwrap(), Status::Acknowledged);
        drop(launch);
        assert_eq!(std::fs::read_dir(home.join("setup-navigation")).unwrap().count(), 0);
        std::fs::remove_dir_all(home).unwrap();
    }
}
