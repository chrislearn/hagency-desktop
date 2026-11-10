//! Owner-private files for the controller's command store: a private
//! directory, non-blocking exclusive locks, bounded JSON reads and create-new
//! writes. Symbolic links, foreign owners, group/other access and hard links
//! are refused. Call from the host's IO worker, never Makepad's event thread.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 64 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// Create `root` (0700, with missing parents) or accept an existing private
/// directory. Returns its canonical path.
pub(crate) fn directory(root: &Path) -> io::Result<PathBuf> {
    if !root.try_exists()? {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
        }
        #[cfg(not(unix))]
        return Err(io::Error::new(io::ErrorKind::Unsupported, "private controller files require Unix"));
    }
    private_entry(root, true)?;
    fs::canonicalize(root)
}

/// Keep a private, nonblocking exclusive claim until the returned file closes.
#[cfg(unix)]
pub(crate) fn lock_private(path: &Path, create: bool) -> io::Result<File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let file = OpenOptions::new().create(create).truncate(false).read(true).write(true)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;
    private_entry(path, false)?;
    // SAFETY: a valid descriptor owned by `file` for the duration of the call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

#[cfg(not(unix))]
pub(crate) fn lock_private(_path: &Path, _create: bool) -> io::Result<File> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "private controller locking requires Unix"))
}

pub(crate) fn private_entry(path: &Path, directory: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(denied("controller entry has an invalid type"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0
            || (!directory && meta.nlink() != 1)
        {
            return Err(denied("controller entry is not private"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(io::Error::new(io::ErrorKind::Unsupported, "private controller files require Unix"))
}

pub(crate) fn read_private<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
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
        // SAFETY: geteuid has no preconditions.
        if !opened.is_file() || opened.dev() != before.dev() || opened.ino() != before.ino()
            || opened.uid() != unsafe { libc::geteuid() } || opened.mode() & 0o077 != 0
            || opened.nlink() != 1
        {
            return Err(denied("controller entry changed while opening"));
        }
    }
    #[cfg(not(unix))]
    let _ = before;
    if opened.len() > MAX_BYTES {
        return Err(invalid("controller entry exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("controller entry exceeds the size limit"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("invalid controller data"))
}

pub(crate) fn write_new(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("invalid controller data"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("controller entry exceeds the size limit"));
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
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn foreign_types_symlinks_and_non_private_files_are_rejected() {
        let root = std::env::temp_dir().join(crate::transport::new_request_id("private-files"));
        let root = directory(&root).unwrap();
        assert_eq!(fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        let file = root.join("value.json");
        write_new(&file, &serde_json::json!({"a": 1})).unwrap();
        assert!(write_new(&file, &serde_json::json!({"a": 2})).is_err(), "create-new only");
        assert_eq!(read_private::<serde_json::Value>(&file).unwrap()["a"], 1);
        let link = root.join("link.json");
        symlink(&file, &link).unwrap();
        assert!(read_private::<serde_json::Value>(&link).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private::<serde_json::Value>(&file).is_err());
        let held = lock_private(&root.join("x.lock"), true).unwrap();
        assert!(lock_private(&root.join("x.lock"), false).is_err());
        drop(held);
        assert!(lock_private(&root.join("x.lock"), false).is_ok());
        let oversized = root.join("big.json");
        assert!(write_new(&oversized, &"x".repeat(MAX_BYTES as usize)).is_err());
        assert!(!oversized.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
