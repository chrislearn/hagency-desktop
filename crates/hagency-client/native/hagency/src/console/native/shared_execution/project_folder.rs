//! Pooled runs in a space with a shared project folder (see
//! `console::space_folder`): the hosting scope asks the server about the
//! work's Room with the execution device.
use super::*;
use crate::console::space_folder::{self, Answer};
use hagency_runtime::owned::SharedProject;

const SCOPE_BOUND: Duration = Duration::from_secs(3);

/// The space folder for work in `room` on this computer, or `None` for the
/// Agent's private folder. This never fails a run.
pub(super) async fn for_room(io: &Io, room: &str) -> Option<SharedProject> {
    if !space_folder::room_id(room) {
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
    let asked = async {
        match bound.room_project_folder(room).await {
            Ok(folder) => Answer::Folder(folder),
            // An older server has no such route: no space has a folder.
            Err(error) if error.status == 404 => Answer::Unsupported,
            Err(_) => Answer::Unavailable,
        }
    };
    let folder = io.project_folders.folder(&account, room, asked).await?;
    #[cfg(test)]
    let root = io.project_root.clone();
    #[cfg(not(test))]
    let root = space_folder::default_root();
    space_folder::place(root, &account, folder).await
}

#[cfg(test)]
impl super::SharedExecutionTransport {
    /// Space project folders of this transport's runs go under `root`.
    pub(in crate::console::native) fn with_project_root(mut self, root: std::path::PathBuf) -> Self {
        self.io.project_root = Some(root);
        self
    }
}
