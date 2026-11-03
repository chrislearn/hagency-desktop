//! Durable, owner-scoped command identity, shared by both controller hosts.
//! The exclusive lock spans submission; quitting releases the lock, never the
//! saved ticket. A reconnect offers that exact ticket for an explicit check.
use crate::{handover::{HandoverStore, private_entry, read_private, write_new}, model::PendingCommand};
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io, os::fd::AsRawFd, os::unix::fs::OpenOptionsExt, path::{Path, PathBuf}};

#[derive(Serialize, Deserialize)]
struct Saved { owner: String, command: PendingCommand }
pub(crate) struct CommandStore { root: PathBuf, path: PathBuf, owner: String, _lock: File }
impl CommandStore {
    pub fn open(home: &Path, owner: &str) -> io::Result<Self> {
        let root = home.join("controller-commands");
        HandoverStore::open(&root)?;
        // A short stable filename avoids filesystem name limits. The complete
        // owner is validated in the document; even a collision fails closed.
        let hash = owner.bytes().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3));
        let path = root.join(format!("{hash:016x}.json"));
        let lock_path = root.join(format!("{hash:016x}.lock"));
        let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&lock_path)?;
        private_entry(&lock_path, false)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 { return Err(io::Error::last_os_error()); }
        Ok(Self { root, path, owner: owner.into(), _lock: lock })
    }
    pub fn load(&self) -> io::Result<Option<PendingCommand>> {
        let saved: Saved = match read_private(&self.path) {
            Ok(saved) => saved,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if saved.owner != self.owner { return Err(io::Error::other("command owner mismatch")); }
        Ok(Some(saved.command))
    }
    pub fn save(&self, command: &PendingCommand) -> io::Result<()> {
        let temporary = self.root.join(format!("{}.tmp", crate::transport::new_request_id("pending")));
        write_new(&temporary, &Saved { owner: self.owner.clone(), command: command.clone() })?;
        if let Err(error) = fs::rename(&temporary, &self.path) { let _ = fs::remove_file(temporary); return Err(error); }
        File::open(&self.root)?.sync_all()
    }
    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) { Ok(()) => File::open(&self.root)?.sync_all(), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()), Err(error) => Err(error) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CommandKind, CommandPhase, CommandTicket};
    #[test]
    fn pending_ticket_survives_reopen_and_other_controller_cannot_replace_it() {
        let home = std::env::temp_dir().join(crate::transport::new_request_id("command-store"));
        let command = PendingCommand { kind: CommandKind::Stop, ticket: CommandTicket { request_id: "original".into(), expected_revision: Some(12) }, phase: CommandPhase::Uncertain };
        let store = CommandStore::open(&home, "@one:example.test").unwrap();
        store.save(&command).unwrap();
        assert!(CommandStore::open(&home, "@one:example.test").is_err());
        assert!(CommandStore::open(&home, "@two:example.test").unwrap().load().unwrap().is_none());
        drop(store);
        let reopened = CommandStore::open(&home, "@one:example.test").unwrap();
        assert_eq!(reopened.load().unwrap(), Some(command));
        reopened.clear().unwrap();
        assert!(reopened.load().unwrap().is_none());
        drop(reopened);
        fs::remove_dir_all(home).unwrap();
    }
}
