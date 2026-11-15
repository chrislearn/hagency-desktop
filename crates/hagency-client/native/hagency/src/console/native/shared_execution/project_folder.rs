//! Opt-in project folder of an Agent space. The server names the folder of a Room's
//! space; this computer keeps one directory per space under
//! `~/Hagency Projects`, and every coding Agent it runs in that space's Rooms
//! works there. A space without a folder, or a lookup that fails, leaves each
//! Agent in its private folder exactly as before.
use super::*;
use hagency_runtime::owned::SharedProject;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::Path,
    sync::Mutex,
};

/// The folder that holds every space folder, in the OS account's home.
const ROOT_NAME: &str = "Hagency Projects";
/// Which folder belongs to which space, so a folder is never shared by
/// accident: a name already on disk is never taken over.
const RECORDS_DIR: &str = ".hagency";
const RECORDS_FILE: &str = "folders.json";
const MAX_RECORDS: usize = 4096;
const MAX_RECORDS_BYTES: u64 = 1024 * 1024;
/// A second space with the same name gets "name 2", "name 3", ...
const MAX_SUFFIX: u32 = 99;

/// A run waits this long for the server's answer at most.
const LOOKUP_BOUND: Duration = Duration::from_secs(2);
const SCOPE_BOUND: Duration = Duration::from_secs(3);
/// A found folder is asked for again after this; until then, and whenever
/// the server cannot answer, the last answer for the Room stands, so a
/// conversation does not move between folders.
const KEEP_FOUND: Duration = Duration::from_secs(10 * 60);
/// A Room without a folder is asked for again sooner, so a space that gets one
/// (or a Room added to such a space) is picked up quickly.
const KEEP_NONE: Duration = Duration::from_secs(60);
const MAX_KNOWN: usize = 4096;

/// The Room id rule of the server route.
pub(super) fn room_id(room: &str) -> bool {
    room.starts_with('!')
        && (2..=255).contains(&room.len())
        && !room.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// The one folder name rule, the same as the server's.
fn valid_folder_name(name: &str) -> bool {
    super::super::valid_project_folder_name(name)
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomFolder {
    pub project_id: String,
    pub folder: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomFolderReply {
    pub project_folder: Option<RoomFolder>,
}

pub(super) struct Known {
    at: Instant,
    folder: Option<RoomFolder>,
}
/// The server's last answers, per (account, Room), kept by the transport.
pub(super) type KnownFolders = Arc<Mutex<HashMap<(String, String), Known>>>;
/// One placement at a time, so two Agents starting together in a new space
/// make one folder, not "name" and "name 2".
static PLACING: Mutex<()> = Mutex::new(());

fn fresh(known: &Known, now: Instant) -> bool {
    let keep = if known.folder.is_some() { KEEP_FOUND } else { KEEP_NONE };
    now.duration_since(known.at) < keep
}

fn remember(
    known: &KnownFolders,
    key: (String, String),
    folder: Option<RoomFolder>,
) -> Option<RoomFolder> {
    let mut known = known.lock().unwrap_or_else(|e| e.into_inner());
    if known.len() >= MAX_KNOWN && !known.contains_key(&key) {
        let now = Instant::now();
        known.retain(|_, k| fresh(k, now));
        if known.len() >= MAX_KNOWN {
            known.clear();
        }
    }
    known.insert(key, Known { at: Instant::now(), folder: folder.clone() });
    folder
}

/// The space folder for work in `room` on this computer, or `None` for the
/// Agent's private folder. This never fails a run.
pub(super) async fn for_room(io: &Io, room: &str) -> Option<SharedProject> {
    if !room_id(room) {
        return None;
    }
    let bound = match tokio::time::timeout(SCOPE_BOUND, io.bound()).await {
        Ok(Ok(bound)) => bound,
        _ => {
            tracing::warn!(code = "space_project_folder_unavailable", "Space project folder lookup had no scope; using the private folder");
            return None;
        }
    };
    let account = bound.scope.profile.account().to_owned();
    let key = (account.clone(), room.to_owned());
    let cached = {
        let known = io.project_folders.lock().unwrap_or_else(|e| e.into_inner());
        known.get(&key).map(|k| (fresh(k, Instant::now()), k.folder.clone()))
    };
    let folder = match cached {
        Some((true, folder)) => folder,
        stale => match tokio::time::timeout(LOOKUP_BOUND, bound.room_project_folder(room)).await {
            Ok(Ok(folder)) => remember(&io.project_folders, key, folder),
            // An older server has no such route: no space has a folder.
            Ok(Err(error)) if error.status == 404 => remember(&io.project_folders, key, None),
            _ => match stale {
                Some((_, folder)) => folder,
                None => {
                    tracing::warn!(code = "space_project_folder_unavailable", "The server did not answer the space project folder lookup; using the private folder");
                    None
                }
            },
        },
    };
    let folder = folder?;
    #[cfg(test)]
    let root = io.project_root.clone().ok_or(Refused::Root);
    #[cfg(not(test))]
    let root = hagency_platform::owner_home()
        .map(|home| home.join(ROOT_NAME))
        .map_err(|_| Refused::Root);
    let placed = tokio::task::spawn_blocking(move || place_in(&root?, &account, &folder)).await;
    match placed {
        Ok(Ok(project)) => Some(project),
        Ok(Err(refused)) => {
            tracing::warn!(code = "space_project_folder_unavailable", reason = refused.reason(), "Space project folder could not be used; using the private folder");
            None
        }
        Err(_) => None,
    }
}

#[cfg(test)]
impl super::SharedExecutionTransport {
    /// Space project folders of this transport's runs go under `root`.
    pub(in crate::console::native) fn with_project_root(mut self, root: std::path::PathBuf) -> Self {
        self.io.project_root = Some(root);
        self
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Name,
    Root,
    Records,
    Taken,
    Folder,
    Io,
}
impl Refused {
    fn reason(&self) -> &'static str {
        match self {
            Self::Name => "invalid_folder",
            Self::Root => "projects_root_unavailable",
            Self::Records => "folder_records_unavailable",
            Self::Taken => "folder_name_taken",
            Self::Folder => "folder_not_a_plain_directory",
            Self::Io => "filesystem_error",
        }
    }
}
impl From<std::io::Error> for Refused {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Records {
    version: u32,
    /// Hash of (account, project id) to the folder's name inside the root.
    folders: BTreeMap<String, String>,
}

fn record_key(account: &str, project: &str) -> Result<String, Refused> {
    let encoded = serde_json::to_vec(&[account, project]).map_err(|_| Refused::Name)?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn new_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Folder names on the usual macOS volume ignore case, so two names that
/// differ only in case are one folder.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn read_records(path: &Path) -> Result<Records, Refused> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Records { version: 1, folders: BTreeMap::new() });
        }
        Err(_) => return Err(Refused::Records),
        Ok(_) => {}
    }
    let file = hagency_store::private::open(path, false).map_err(|_| Refused::Records)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORDS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Refused::Records)?;
    if bytes.len() as u64 > MAX_RECORDS_BYTES {
        return Err(Refused::Records);
    }
    let records: Records = serde_json::from_slice(&bytes).map_err(|_| Refused::Records)?;
    if records.version != 1
        || records.folders.len() > MAX_RECORDS
        || records
            .folders
            .iter()
            .any(|(key, name)| key.len() != 64 || !placed_name(name))
    {
        return Err(Refused::Records);
    }
    Ok(records)
}

/// A recorded name: a valid folder name, or one with a " N" suffix.
fn placed_name(name: &str) -> bool {
    valid_folder_name(name)
        || name.rsplit_once(' ').is_some_and(|(base, n)| {
            valid_folder_name(base)
                && n.parse::<u32>().is_ok_and(|n| (2..=MAX_SUFFIX).contains(&n))
                && !n.starts_with('0')
        })
}

/// The directory of `folder` for `account` under `root`, made on first use.
fn place_in(root: &Path, account: &str, folder: &RoomFolder) -> Result<SharedProject, Refused> {
    if !valid_folder_name(&folder.folder) || super::super::key(&folder.project_id).is_err() {
        return Err(Refused::Name);
    }
    let key = record_key(account, &folder.project_id)?;
    let _placing = PLACING.lock().unwrap_or_else(|e| e.into_inner());
    match new_directory(root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(Refused::Root),
    }
    // The owner may have moved the root and left a link; the folders inside
    // it are used only as plain directories.
    let root = root.canonicalize().map_err(|_| Refused::Root)?;
    if !std::fs::metadata(&root).is_ok_and(|m| m.is_dir()) {
        return Err(Refused::Root);
    }
    let records_dir = root.join(RECORDS_DIR);
    hagency_store::private::directory(&records_dir).map_err(|_| Refused::Records)?;
    let records_path = records_dir.join(RECORDS_FILE);
    let mut records = read_records(&records_path)?;
    if let Some(name) = records.folders.get(&key) {
        let directory = root.join(name);
        match std::fs::symlink_metadata(&directory) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(Refused::Folder),
            // The owner deleted it: it is made again, empty, under its name.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                new_directory(&directory)?
            }
            Err(error) => return Err(error.into()),
        }
        return SharedProject::new(root, directory).map_err(|_| Refused::Folder);
    }
    if records.folders.len() >= MAX_RECORDS {
        return Err(Refused::Records);
    }
    for n in 1..=MAX_SUFFIX {
        let name = if n == 1 {
            folder.folder.clone()
        } else {
            format!("{} {n}", folder.folder)
        };
        // A recorded folder the owner deleted stays its space's name.
        if records.folders.values().any(|taken| same_name(taken, &name)) {
            continue;
        }
        let directory = root.join(&name);
        match new_directory(&directory) {
            Ok(()) => {}
            // Never take over a folder that is already there.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
        records.folders.insert(key, name);
        let written = serde_json::to_vec(&records)
            .map_err(|_| Refused::Records)
            .and_then(|bytes| {
                hagency_store::private::replace(&records_path, &bytes)
                    .map_err(|_| Refused::Records)
            });
        if let Err(refused) = written {
            let _ = std::fs::remove_dir(&directory);
            return Err(refused);
        }
        return SharedProject::new(root, directory).map_err(|_| Refused::Folder);
    }
    Err(Refused::Taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn folder(project: &str, name: &str) -> RoomFolder {
        RoomFolder { project_id: project.into(), folder: name.into() }
    }
    fn base() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join(ROOT_NAME);
        (dir, root)
    }

    #[test]
    fn a_space_gets_one_folder_that_stays_its_own() {
        let (_dir, root) = base();
        let app = place_in(&root, "@a:x", &folder("prj_1", "app")).unwrap();
        assert_eq!(app.directory(), root.join("app"));
        assert_eq!(app.root(), root);
        // Every later start of the same space uses the same folder.
        let again = place_in(&root, "@a:x", &folder("prj_1", "app")).unwrap();
        assert_eq!(again, app);
        // Its name is fixed at creation: a later name for the same space
        // does not move it.
        let renamed = place_in(&root, "@a:x", &folder("prj_1", "other")).unwrap();
        assert_eq!(renamed.directory(), root.join("app"));
        // Another space with the same name, or the same space of another
        // account, gets a folder of its own.
        let second = place_in(&root, "@a:x", &folder("prj_2", "app")).unwrap();
        assert_eq!(second.directory(), root.join("app 2"));
        let other = place_in(&root, "@b:x", &folder("prj_1", "app")).unwrap();
        assert_eq!(other.directory(), root.join("app 3"));
        // The records stay private and outside every space folder.
        let records = root.join(RECORDS_DIR).join(RECORDS_FILE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&records).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
        let text = std::fs::read_to_string(&records).unwrap();
        assert!(!text.contains("@a:x") && !text.contains("prj_1"));
    }

    #[test]
    fn a_folder_already_on_disk_is_never_taken_over() {
        let (_dir, root) = base();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app/mine.txt"), "x").unwrap();
        std::fs::write(root.join("app 2"), "a file").unwrap();
        let placed = place_in(&root, "@a:x", &folder("prj_1", "app")).unwrap();
        assert_eq!(placed.directory(), root.join("app 3"));
        assert!(std::fs::read_dir(placed.directory()).unwrap().next().is_none());
    }

    #[test]
    fn a_deleted_folder_is_made_again_and_keeps_its_name() {
        let (_dir, root) = base();
        let placed = place_in(&root, "@a:x", &folder("prj_1", "App")).unwrap();
        std::fs::remove_dir(placed.directory()).unwrap();
        // Another space does not get the deleted folder's name, in any case.
        let other = place_in(&root, "@a:x", &folder("prj_2", "app")).unwrap();
        assert_eq!(other.directory(), root.join("app 2"));
        let again = place_in(&root, "@a:x", &folder("prj_1", "App")).unwrap();
        assert_eq!(again.directory(), root.join("App"));
        assert!(again.check());
    }

    #[cfg(unix)]
    #[test]
    fn a_recorded_folder_replaced_by_a_link_or_file_is_refused() {
        let (dir, root) = base();
        let placed = place_in(&root, "@a:x", &folder("prj_1", "app")).unwrap();
        std::fs::remove_dir(placed.directory()).unwrap();
        let elsewhere = dir.path().canonicalize().unwrap().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, placed.directory()).unwrap();
        assert_eq!(place_in(&root, "@a:x", &folder("prj_1", "app")), Err(Refused::Folder));
        std::fs::remove_file(placed.directory()).unwrap();
        std::fs::write(placed.directory(), "x").unwrap();
        assert_eq!(place_in(&root, "@a:x", &folder("prj_1", "app")), Err(Refused::Folder));
    }

    #[test]
    fn names_and_records_that_break_the_rule_are_refused() {
        let (_dir, root) = base();
        for name in ["", ".app", "a/b", "a\\b", "c:d", " app", "app ", "a\nb", &"a".repeat(65)] {
            assert_eq!(place_in(&root, "@a:x", &folder("prj_1", name)), Err(Refused::Name), "{name:?}");
        }
        assert_eq!(place_in(&root, "@a:x", &folder("prj/1", "app")), Err(Refused::Name));
        assert!(valid_folder_name("项目") && valid_folder_name(&"é".repeat(64)) && valid_folder_name("~"));
        // Records changed by hand so they point outside the root are refused.
        place_in(&root, "@a:x", &folder("prj_1", "app")).unwrap();
        let records = root.join(RECORDS_DIR).join(RECORDS_FILE);
        let key = record_key("@a:x", "prj_1").unwrap();
        let bad = format!(r#"{{"version":1,"folders":{{"{key}":"../escape"}}}}"#);
        hagency_store::private::replace(&records, bad.as_bytes()).unwrap();
        assert_eq!(place_in(&root, "@a:x", &folder("prj_1", "app")), Err(Refused::Records));
        // A suffix may take a full-length name past 64 characters, never more.
        let full = "a".repeat(64);
        assert!(placed_name(&format!("{full} 2")) && placed_name(&format!("{full} 99")));
        for bad in [format!("{full} 1"), format!("{full} 02"), format!("{full} 100"), "../x 2".into()] {
            assert!(!placed_name(&bad), "{bad:?}");
        }
    }

    #[test]
    fn room_ids_follow_the_route_rule() {
        assert!(room_id("!abc:example.org"));
        assert!(!room_id("abc:example.org") && !room_id("!") && !room_id("!a b") && !room_id("!a\n"));
        assert!(!room_id(&format!("!{}", "a".repeat(255))));
    }

    #[test]
    fn found_folders_are_kept_longer_than_none() {
        let now = Instant::now();
        let found = Known { at: now, folder: Some(folder("prj_1", "app")) };
        let none = Known { at: now, folder: None };
        let later = now + Duration::from_secs(120);
        assert!(fresh(&found, later) && !fresh(&none, later));
        assert!(!fresh(&found, now + KEEP_FOUND));
    }
}
