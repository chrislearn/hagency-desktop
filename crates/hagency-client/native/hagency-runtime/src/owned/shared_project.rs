//! A space's shared project folder on this computer. When a space has one,
//! every coding Agent of that space that runs here works in this directory
//! instead of its private room folder. The host chooses the directory; these
//! checks only refuse paths that could lead anywhere else.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSharedProject;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedProject {
    root: PathBuf,
    directory: PathBuf,
}

impl SharedProject {
    /// Both paths must exist, be absolute and contain no symbolic link, and
    /// the directory must sit directly inside the root.
    pub fn new(root: PathBuf, directory: PathBuf) -> Result<Self, InvalidSharedProject> {
        let project = Self { root, directory };
        if project.check() {
            Ok(project)
        } else {
            Err(InvalidSharedProject)
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Checked again right before each use: the folder may have been
    /// replaced since it was chosen.
    pub fn check(&self) -> bool {
        let real = |path: &Path| {
            path.is_absolute() && path.canonicalize().ok().as_deref() == Some(path)
        };
        real(&self.root)
            && real(&self.directory)
            && self.directory.parent() == Some(self.root.as_path())
            && std::fs::symlink_metadata(&self.directory).is_ok_and(|m| m.is_dir())
    }
    /// A private conversation scope and the project folders never contain
    /// each other, so neither grants access to the other.
    pub fn apart_from(&self, scope: &Path) -> bool {
        !self.root.starts_with(scope) && !scope.starts_with(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_folder_directly_inside_the_root_is_accepted() {
        let base = tempfile::tempdir().unwrap();
        let base = base.path().canonicalize().unwrap();
        let root = base.join("Projects");
        let folder = root.join("app");
        std::fs::create_dir_all(&folder).unwrap();
        let project = SharedProject::new(root.clone(), folder.clone()).unwrap();
        assert_eq!(project.directory(), folder);
        assert!(project.apart_from(&base.join("state/scope")));
        assert!(!project.apart_from(&root));
        assert!(!project.apart_from(&folder.join("inner")));
        assert!(!project.apart_from(&base));

        let deeper = folder.join("deeper");
        std::fs::create_dir(&deeper).unwrap();
        assert!(SharedProject::new(root.clone(), deeper).is_err());
        assert!(SharedProject::new(root.clone(), root.clone()).is_err());
        assert!(SharedProject::new(root.clone(), root.join("missing")).is_err());
        assert!(SharedProject::new(PathBuf::from("Projects"), PathBuf::from("Projects/app")).is_err());

        std::fs::write(root.join("file"), "x").unwrap();
        assert!(SharedProject::new(root.clone(), root.join("file")).is_err());
        #[cfg(unix)]
        {
            let elsewhere = base.join("elsewhere");
            std::fs::create_dir(&elsewhere).unwrap();
            std::os::unix::fs::symlink(&elsewhere, root.join("link")).unwrap();
            assert!(SharedProject::new(root.clone(), root.join("link")).is_err());
            std::os::unix::fs::symlink(&root, base.join("root-link")).unwrap();
            assert!(SharedProject::new(base.join("root-link"), base.join("root-link/app")).is_err());
        }
        // A folder replaced after it was chosen no longer passes.
        std::fs::remove_dir_all(&folder).unwrap();
        assert!(!project.check());
    }
}
