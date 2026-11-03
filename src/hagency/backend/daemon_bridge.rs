//! Select custody from the durable handover journal, never socket liveness.
use super::*;
use ::hagency::daemon::{
    assignment::{self, Assignment},
    bridge::ControllerClient,
};

impl Service {
    pub(super) fn daemon_assignment(&self) -> Result<Option<PathBuf>, String> {
        Self::assignment_for(&self.0.origin, &self.0.owner, &self.0.state)
    }
    pub(super) fn assignment_for(
        origin: &str,
        owner: &str,
        state: &std::path::Path,
    ) -> Result<Option<PathBuf>, String> {
        let home = crate::hagency::owner_dashboard::endpoint().home;
        match assignment::resolve(&home, origin, owner, state).map_err(str::to_owned)? {
            Assignment::Local => Ok(None),
            Assignment::Daemon { home, .. } => Ok(Some(home)),
        }
    }

    /// Stop any source executor before attaching. Repeated calls retain a failed
    /// drain, and an absent daemon remains an error instead of opening locally.
    pub(super) async fn remote_client(&self) -> Result<Option<Arc<ControllerClient>>, String> {
        self.check()?;
        let mut native = self.0.native.lock().await;
        let Some(home) = self.daemon_assignment()? else {
            if self
                .0
                .remote
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
            {
                self.retire();
                return Err("native_owner_restart_required".into());
            }
            return Ok(None);
        };
        let registered = self
            .0
            .native_retirement
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(owner) = native.as_ref().cloned().or(registered) {
            owner.retire();
            #[cfg(feature = "resource-adoption")]
            {
                self.0.allocations.retire();
                self.0.allocations.drain().await;
            }
            #[cfg(feature = "shared-execution")]
            {
                self.0.shared.retire();
                self.0.shared.drain().await;
            }
            owner.suspend().await.map_err(|e| e.code)?;
            native.take();
            self.0
                .native_retirement
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
        }
        self.check()?;
        let mut slot = self.0.remote.lock().unwrap_or_else(|p| p.into_inner());
        Ok(Some(
            slot.get_or_insert_with(|| {
                ControllerClient::new(
                    home,
                    self.0.owner.clone(),
                    self.0.origin.clone(),
                    self.0.epoch + 1,
                )
            })
            .clone(),
        ))
    }

    pub fn daemon_handover_source(&self) -> Result<(String, String, PathBuf), String> {
        self.check()?;
        Ok((
            self.0.origin.clone(),
            self.0.owner.clone(),
            self.0.state.clone(),
        ))
    }

    pub(super) fn watch_daemon_assignment(&self) {
        let weak = Arc::downgrade(&self.0);
        crate::sliding_sync::backend_runtime().spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let Some(inner) = weak.upgrade() else { break };
                let service = Service(inner);
                if service.check().is_err() {
                    break;
                }
                if service.daemon_assignment().ok().flatten().is_some() {
                    let _ = service.remote_client().await;
                }
            }
        });
    }

    /// The setup host stages the exact current root before requesting daemon
    /// enrollment. Cancellation after staging keeps custody assigned remotely.
    pub async fn stage_daemon_handover(&self) -> Result<PathBuf, String> {
        self.check()?;
        let home = crate::hagency::owner_dashboard::endpoint().home;
        ::hagency::daemon::enrollment::Enrollment::stage_handover(
            &home,
            &self.0.origin,
            &self.0.owner,
            None,
            &self.0.state,
        )
        .map_err(|e| e.code().to_owned())?;
        self.remote_client().await?;
        Ok(self.0.state.clone())
    }
}
