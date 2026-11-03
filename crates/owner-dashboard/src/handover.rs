//! Private, bounded UI-state handover between the two dashboard hosts.
//!
//! A handover is navigation data, never an owner credential or a command grant.
//! The receiver supplies its independently authenticated owner identity and only
//! acknowledges after it has restored the view. Call filesystem operations from
//! the host's IO worker, not Makepad's event thread.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024;
const LIFETIME_MS: u64 = 120_000;
const MAX_PENDING: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Standalone,
    Embedded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    version: u32,
    id: String,
    owner_id: String,
    source: Host,
    target: Host,
    created_at_ms: u64,
    expires_at_ms: u64,
    view: Value,
}

/// Only the sender obtains this ticket. Its fields cannot be forged by a script.
#[derive(Clone, Debug)]
pub struct Ticket {
    envelope: Envelope,
    path: PathBuf,
}
impl Ticket {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug)]
pub struct Received {
    envelope: Envelope,
}
impl Received {
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms < self.envelope.created_at_ms || now_ms >= self.envelope.expires_at_ms
    }
    pub fn view(&self) -> &Value {
        &self.envelope.view
    }
    pub fn owner_id(&self) -> &str {
        &self.envelope.owner_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Waiting,
    Acknowledged,
    Expired,
}

#[derive(Debug)]
pub struct HandoverStore {
    root: PathBuf,
}

impl HandoverStore {
    /// One bounded navigation document, scoped to its authenticated owner.
    /// Hosts use a separate `controller-view` directory from the inbox.
    pub fn load_view(&self, owner: &str) -> io::Result<Option<crate::ViewState>> {
        let path = self.root.join("view.json");
        if !path.try_exists()? { return Ok(None); }
        let mut view: crate::ViewState = read_private(&path)?;
        view.intent = None;
        Ok((view.owner_id.as_deref() == Some(owner)).then_some(view))
    }
    pub fn save_view(&self, owner: &str, view: &crate::ViewState) -> io::Result<()> {
        valid_owner(owner)?;
        private_entry(&self.root, true)?;
        if view.owner_id.as_deref() != Some(owner) { return Err(denied("owner mismatch")); }
        let path = self.root.join("view.json");
        if path.try_exists()? { private_entry(&path, false)?; }
        let temporary = self.root.join(format!("{}.tmp", random_id()?));
        let mut saved = view.clone();
        saved.intent = None;
        write_new(&temporary, &saved)?;
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        File::open(&self.root)?.sync_all()
    }
    /// Both hosts use the same trusted, local control-directory child.
    /// This must not be chosen from an incoming URL or handover document.
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        if !root.try_exists()? {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
            }
            #[cfg(not(unix))]
            return Err(io::Error::new(io::ErrorKind::Unsupported, "local handover requires Unix"));
        }
        private_entry(root, true)?;
        Ok(Self { root: fs::canonicalize(root)? })
    }

    pub fn offer(
        &self,
        owner_id: &str,
        source: Host,
        target: Host,
        view: Value,
        now_ms: u64,
    ) -> io::Result<Ticket> {
        private_entry(&self.root, true)?;
        valid_owner(owner_id)?;
        if source == target || !view.is_object() {
            return Err(invalid("invalid handover destination or view"));
        }
        // A source can crash before removing its ticket. Expired handovers must
        // not permanently exhaust the small inbox; remove only validated files
        // for any owner, never arbitrary entries in the private directory.
        for entry in fs::read_dir(&self.root)?.filter_map(Result::ok).take(128) {
            let path = entry.path();
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|id| valid_id(id)) else { continue };
            if path.extension().is_none_or(|ext| ext != "json") { continue }
            if let Ok(envelope) = read_private::<Envelope>(&path) {
                if envelope.id == id && valid_owner(&envelope.owner_id).is_ok()
                    && envelope.version == VERSION && envelope.source != envelope.target
                    && envelope.expires_at_ms.checked_sub(envelope.created_at_ms) == Some(LIFETIME_MS)
                    && now_ms >= envelope.expires_at_ms
                {
                    self.remove(&Ticket { envelope, path })?;
                }
            }
        }
        let pending = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|v| v == "json"))
            .take(MAX_PENDING)
            .count();
        if pending >= MAX_PENDING {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "too many pending handovers"));
        }
        let envelope = Envelope {
            version: VERSION,
            id: random_id()?,
            owner_id: owner_id.into(),
            source,
            target,
            created_at_ms: now_ms,
            expires_at_ms: now_ms.checked_add(LIFETIME_MS)
                .ok_or_else(|| invalid("invalid handover time"))?,
            view,
        };
        let path = self.request_path(&envelope.id);
        write_new(&path, &envelope)?;
        Ok(Ticket { envelope, path })
    }

    /// Validate the destination and owner before exposing a saved view to UI.
    pub fn receive(
        &self,
        path: &Path,
        owner_id: &str,
        target: Host,
        now_ms: u64,
    ) -> io::Result<Received> {
        private_entry(&self.root, true)?;
        valid_owner(owner_id)?;
        let name = path.file_name().and_then(|n| n.to_str())
            .ok_or_else(|| invalid("invalid handover path"))?;
        let id = name.strip_suffix(".json")
            .filter(|id| valid_id(id))
            .ok_or_else(|| invalid("invalid handover path"))?;
        if path.parent().map(fs::canonicalize).transpose()?.as_ref() != Some(&self.root) {
            return Err(denied("handover is outside the controller directory"));
        }
        let envelope: Envelope = read_private(&self.request_path(id))?;
        validate(&envelope, owner_id, target, now_ms)?;
        if envelope.id != id {
            return Err(invalid("handover identity changed"));
        }
        if self.ack_path(id).try_exists()? {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "handover already restored"));
        }
        Ok(Received { envelope })
    }

    /// Invoke only after successful view restoration and a fresh account check.
    pub fn acknowledge(
        &self,
        received: &Received,
        current_owner: &str,
        now_ms: u64,
    ) -> io::Result<()> {
        private_entry(&self.root, true)?;
        validate(&received.envelope, current_owner, received.envelope.target, now_ms)?;
        let original: Envelope = read_private(&self.request_path(&received.envelope.id))?;
        if original != received.envelope {
            return Err(invalid("handover changed before acknowledgement"));
        }
        // The full original bounded envelope binds acknowledgement to its view.
        write_new(&self.ack_path(&original.id), &original)
    }

    /// The source may dismiss its view only after Acknowledged.
    pub fn poll(&self, ticket: &Ticket, current_owner: &str, now_ms: u64) -> io::Result<Status> {
        private_entry(&self.root, true)?;
        if ticket.envelope.owner_id != current_owner
            || ticket.path != self.request_path(&ticket.envelope.id)
        {
            return Err(denied("handover owner or directory changed"));
        }
        if now_ms < ticket.envelope.created_at_ms || now_ms >= ticket.envelope.expires_at_ms {
            return Ok(Status::Expired);
        }
        let path = self.ack_path(&ticket.envelope.id);
        match read_private::<Envelope>(&path) {
            Ok(ack) if ack == ticket.envelope => Ok(Status::Acknowledged),
            Ok(_) => Err(invalid("handover acknowledgement does not match")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Status::Waiting),
            Err(e) => Err(e),
        }
    }

    /// Cancel or finish this sender's ticket; the caller retains its view on error.
    pub fn remove(&self, ticket: &Ticket) -> io::Result<()> {
        private_entry(&self.root, true)?;
        if ticket.path != self.request_path(&ticket.envelope.id) {
            return Err(denied("handover directory changed"));
        }
        let current: Envelope = read_private(&ticket.path)?;
        if current != ticket.envelope {
            return Err(invalid("handover changed before removal"));
        }
        for path in [self.ack_path(&ticket.envelope.id), ticket.path.clone()] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn request_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }
    fn ack_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.ack"))
    }
}

fn validate(e: &Envelope, owner: &str, target: Host, now: u64) -> io::Result<()> {
    valid_owner(owner)?;
    if e.owner_id != owner || e.target != target || e.source == target {
        return Err(denied("handover owner or destination does not match"));
    }
    if e.version != VERSION || !valid_id(&e.id) || !e.view.is_object()
        || e.expires_at_ms.checked_sub(e.created_at_ms) != Some(LIFETIME_MS)
        || now < e.created_at_ms || now >= e.expires_at_ms
    {
        return Err(invalid("handover is invalid or expired"));
    }
    Ok(())
}

fn valid_owner(value: &str) -> io::Result<()> {
    let parts = value.strip_prefix('@').and_then(|s| s.split_once(':'));
    if value.len() > 320 || parts.is_none_or(|(local, server)| local.is_empty() || server.is_empty())
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        Err(invalid("invalid owner identity"))
    } else {
        Ok(())
    }
}
fn valid_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn random_id() -> io::Result<String> {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(not(unix))]
    Err(io::Error::new(io::ErrorKind::Unsupported, "local handover requires Unix"))
}

fn private_entry(path: &Path, directory: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(denied("handover entry has an invalid type"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0
            || (!directory && meta.nlink() != 1)
        {
            return Err(denied("handover entry is not private"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(io::Error::new(io::ErrorKind::Unsupported, "local handover requires Unix"))
}

fn read_private<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    private_entry(path, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let before = fs::symlink_metadata(path)?;
    let opened = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !opened.is_file() || opened.dev() != before.dev() || opened.ino() != before.ino()
            || opened.uid() != unsafe { libc::geteuid() } || opened.mode() & 0o077 != 0
            || opened.nlink() != 1
        {
            return Err(denied("handover entry changed while opening"));
        }
    }
    if opened.len() > MAX_BYTES {
        return Err(invalid("handover exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("handover exceeds the size limit"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("invalid handover data"))
}

fn write_new(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("invalid handover data"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("handover exceeds the size limit"));
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Fixture(PathBuf, HandoverStore);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("hagency-handover-{}", random_id().unwrap()));
            let store = HandoverStore::open(&root).unwrap();
            Self(root, store)
        }
        fn offer(&self) -> Ticket {
            self.1.offer("@owner:example.test", Host::Embedded, Host::Standalone,
                json!({"tab":"usage","resource":"pool-1","scroll":123.0}), 1000).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn expired_other_account_tickets_do_not_exhaust_the_inbox() {
        let f = Fixture::new();
        for _ in 0..MAX_PENDING {
            f.offer();
        }
        assert!(f.1.offer("@other:example.test", Host::Embedded, Host::Standalone, json!({}), 1001).is_err());
        let ticket = f.1.offer("@other:example.test", Host::Embedded, Host::Standalone, json!({}), 121000).unwrap();
        assert!(ticket.path().exists());
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    }

    #[test]
    fn source_waits_for_exact_restored_view_and_destination_consumes_once() {
        let f = Fixture::new();
        let ticket = f.offer();
        assert_eq!(f.1.poll(&ticket, "@owner:example.test", 1001).unwrap(), Status::Waiting);
        let received = f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 1002).unwrap();
        assert_eq!(received.view()["scroll"], 123.0);
        assert_eq!(f.1.poll(&ticket, "@owner:example.test", 1003).unwrap(), Status::Waiting);
        f.1.acknowledge(&received, "@owner:example.test", 1004).unwrap();
        assert_eq!(f.1.poll(&ticket, "@owner:example.test", 1005).unwrap(), Status::Acknowledged);
        assert!(f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 1006).is_err());
        f.1.remove(&ticket).unwrap();
        assert!(!ticket.path().exists());
    }

    #[test]
    fn changed_owner_destination_and_expiry_never_acknowledge() {
        let f = Fixture::new();
        let ticket = f.offer();
        assert!(f.1.receive(ticket.path(), "@other:example.test", Host::Standalone, 1002).is_err());
        assert!(f.1.receive(ticket.path(), "@owner:example.test", Host::Embedded, 1002).is_err());
        let received = f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 1002).unwrap();
        assert!(f.1.acknowledge(&received, "@other:example.test", 1003).is_err());
        assert!(f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 121000).is_err());
        assert_eq!(f.1.poll(&ticket, "@owner:example.test", 121000).unwrap(), Status::Expired);
        assert!(f.1.poll(&ticket, "@other:example.test", 1003).is_err());
    }

    #[test]
    fn tampered_view_does_not_acknowledge_the_senders_ticket() {
        let f = Fixture::new();
        let ticket = f.offer();
        let received = f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 1002).unwrap();
        let mut modified = ticket.envelope.clone();
        modified.view = json!({"tab":"resources"});
        fs::write(ticket.path(), serde_json::to_vec(&modified).unwrap()).unwrap();
        assert!(f.1.acknowledge(&received, "@owner:example.test", 1003).is_err());
        write_new(&f.1.ack_path(&modified.id), &modified).unwrap();
        assert!(f.1.poll(&ticket, "@owner:example.test", 1004).is_err());
    }

    #[test]
    fn foreign_paths_symlinks_and_non_private_files_are_rejected() {
        let f = Fixture::new();
        let other = Fixture::new();
        let ticket = f.offer();
        assert!(other.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 1002).is_err());
        fs::set_permissions(ticket.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 1002).is_err());
        fs::set_permissions(ticket.path(), fs::Permissions::from_mode(0o600)).unwrap();
        let link = f.0.join(format!("{}.json", random_id().unwrap()));
        symlink(ticket.path(), &link).unwrap();
        assert!(f.1.receive(&link, "@owner:example.test", Host::Standalone, 1002).is_err());
    }

    #[test]
    fn oversized_views_are_rejected_before_creating_a_ticket() {
        let f = Fixture::new();
        assert!(f.1.offer("@owner:example.test", Host::Embedded, Host::Standalone,
            json!({"draft":"x".repeat(MAX_BYTES as usize)}), 1000).is_err());
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    }

    #[test]
    fn saved_view_cannot_replay_an_approval_navigation() {
        let f = Fixture::new();
        let owner = "@owner:example.test";
        let view = crate::ViewState { owner_id: Some(owner.into()), intent: Some(crate::NavigationIntent::ReviewApprovals), ..Default::default() };
        f.1.save_view(owner, &view).unwrap();
        assert_eq!(f.1.load_view(owner).unwrap().unwrap().intent, None);
        assert!(!fs::read_to_string(f.0.join("view.json")).unwrap().contains("reviewApprovals"));
        assert!(f.1.load_view("@other:example.test").unwrap().is_none());
    }

    #[test]
    fn missing_owner_and_reverse_clock_cannot_restore_a_view() {
        let f = Fixture::new();
        for owner in ["", "@:", "@owner:", "@:example.test", "@owner:example.test\n"] {
            assert!(f.1.offer(owner, Host::Embedded, Host::Standalone,
                json!({"tab":"overview"}), 1000).is_err());
        }
        let ticket = f.offer();
        assert!(f.1.receive(ticket.path(), "@owner:example.test", Host::Standalone, 999).is_err());
        assert_eq!(f.1.poll(&ticket, "@owner:example.test", 999).unwrap(), Status::Expired);
    }
}
