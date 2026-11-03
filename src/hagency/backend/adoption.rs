//! Trusted native initialization selection. No script arguments or registry paths.
use super::*;
use ::hagency::native_owner::{AdoptedManagementSelection, ManagementPackage};
use std::sync::atomic::Ordering;
use ::hagency::daemon::bridge::{OwnerGrantAccess, AdoptedSelection, DeviceSetupReview};

/// Static native codes only; never log provider text, paths or credentials.
fn activation_failure_code(code: &str) -> &'static str {
    match code {
        "adopted_grant_device_reauthorization_required" => {
            "adopted_grant_device_reauthorization_required"
        }
        "adopted_management_initializer_unavailable" => {
            "adopted_management_initializer_unavailable"
        }
        "adopted_management_scope_changed" => "adopted_management_scope_changed",
        "fixture_authority_already_open" => "fixture_authority_already_open",
        "fixture_personal_create_failed" => "fixture_personal_create_failed",
        "fixture_profile_outside_root" => "fixture_profile_outside_root",
        "fixture_profile_unavailable" => "fixture_profile_unavailable",
        "fixture_root_changed" => "fixture_root_changed",
        "fixture_root_unavailable" => "fixture_root_unavailable",
        "empty_device_history_not_empty" => "empty_device_history_not_empty",
        "empty_device_review_changed" => "empty_device_review_changed",
        "empty_device_setup_has_no_quota" => "empty_device_setup_has_no_quota",
        "empty_device_already_active" => "empty_device_already_active",
        "management_" => "management_",
        "management_access_required" => "management_access_required",
        "management_backing_changed" => "management_backing_changed",
        "management_capacity" => "management_capacity",
        "management_drain_outcome_unknown" => "management_drain_outcome_unknown",
        "management_drain_unavailable" => "management_drain_unavailable",
        "management_model_unqualified" => "management_model_unqualified",
        "management_not_found" => "management_not_found",
        "management_outcome_unknown" => "management_outcome_unknown",
        "management_profile_mismatch" => "management_profile_mismatch",
        "management_resource_in_use" => "management_resource_in_use",
        "management_revision_conflict" => "management_revision_conflict",
        "management_scope_changed" => "management_scope_changed",
        "management_state_unavailable" => "management_state_unavailable",
        "management_storage_failed" => "management_storage_failed",
        "management_unavailable" => "management_unavailable",
        "native_original" => "native_original",
        "native_owner_state_unavailable" => "native_owner_state_unavailable",
        "native_random_unavailable" => "native_random_unavailable",
        "owner_authorization_required" => "owner_authorization_required",
        "owner_source_adoption_required" => "owner_source_adoption_required",
        "provider_account_changed" => "provider_account_changed",
        "provider_choice_unavailable" => "provider_choice_unavailable",
        "provider_closing" => "provider_closing",
        "provider_directory_unavailable" => "provider_directory_unavailable",
        "provider_identity_timeout" => "provider_identity_timeout",
        "provider_identity_unavailable" => "provider_identity_unavailable",
        "provider_profile_mismatch" => "provider_profile_mismatch",
        "source_adoption_backup_capacity" => "source_adoption_backup_capacity",
        "source_adoption_backup_changed" => "source_adoption_backup_changed",
        "source_adoption_backup_missing" => "source_adoption_backup_missing",
        "source_adoption_binding_changed" => "source_adoption_binding_changed",
        "source_adoption_busy" => "source_adoption_busy",
        "source_adoption_custody_closed" => "source_adoption_custody_closed",
        "source_adoption_custody_missing" => "source_adoption_custody_missing",
        "source_adoption_custody_unknown" => "source_adoption_custody_unknown",
        "source_adoption_history_unsupported" => "source_adoption_history_unsupported",
        "source_adoption_inventory_unsupported" => "source_adoption_inventory_unsupported",
        "source_adoption_not_active" => "source_adoption_not_active",
        "source_adoption_original_ceiling_required" => "source_adoption_original_ceiling_required",
        "source_adoption_outcome_unknown" => "source_adoption_outcome_unknown",
        "source_adoption_platform_unqualified" => "source_adoption_platform_unqualified",
        "source_adoption_recovery_only" => "source_adoption_recovery_only",
        "source_adoption_review_changed" => "source_adoption_review_changed",
        "source_adoption_preview_unavailable" => "source_adoption_preview_unavailable",
        "source_adoption_review_expired" => "source_adoption_review_expired",
        "source_adoption_scope_changed" => "source_adoption_scope_changed",
        "source_adoption_storage_unavailable" => "source_adoption_storage_unavailable",
        "source_configuration_drain_unknown" => "source_configuration_drain_unknown",
        "source_domain_busy" => "source_domain_busy",
        "source_domain_drain_unknown" => "source_domain_drain_unknown",
        "source_files_changed" => "source_files_changed",
        "source_files_unavailable" => "source_files_unavailable",
        "source_installation_busy" => "source_installation_busy",
        "source_installation_lease_changed" => "source_installation_lease_changed",
        "source_inventory_adoption_not_assessed" => "source_inventory_adoption_not_assessed",
        "source_managed_runtime_drain_unknown" => "source_managed_runtime_drain_unknown",
        "source_personal_busy" => "source_personal_busy",
        "source_provider_drain_unknown" => "source_provider_drain_unknown",
        "source_quiescence_authority_changed" => "source_quiescence_authority_changed",
        "source_quiescence_owner_mismatch" => "source_quiescence_owner_mismatch",
        "source_quiescence_permission_required" => "source_quiescence_permission_required",
        "source_quiescence_timeout" => "source_quiescence_timeout",
        "source_quiescence_unknown" => "source_quiescence_unknown",
        "source_quiescence_unproven" => "source_quiescence_unproven",
        "source_recovery_required" => "source_recovery_required",
        "source_registry_changed" => "source_registry_changed",
        "source_registry_unavailable" => "source_registry_unavailable",
        "source_root_changed" => "source_root_changed",
        "source_runtime_drain_unknown" => "source_runtime_drain_unknown",
        "source_schema_unavailable" => "source_schema_unavailable",
        _ => "source_adoption_unclassified_failure",
    }
}

impl Service {
    /// The migration must have completed, drained its archive and released all
    /// retired source handles. Persist selection before any cancellation point;
    /// a busy original root remains retryable and never falls back to legacy.
    pub(crate) async fn select_adopted_management(
        &self,
        selection: AdoptedSelection,
    ) -> Result<(), String> {
        self.check()?;
        let selection = match selection {
            AdoptedSelection::Local(selection) => selection,
            AdoptedSelection::Remote => {
                if self.remote_client().await?.is_none() { return Err("daemon_owner_assignment_changed".into()); }
                return Ok(());
            }
        };
        if !selection.matches_host(&self.0.state, &self.0.origin, &self.0.owner) {
            return Err("adopted_management_scope_changed".into());
        }
        let _native = self.0.native.lock().await;
        self.check()?;
        *self
            .0
            .adopted_initialization
            .lock()
            .map_err(|_| "management_unavailable")? = Some(selection);
        self.0.adoption_pending.store(true, Ordering::Release);
        if let Some(owner) = self
            .0
            .native_retirement
            .lock()
            .map_err(|_| "management_unavailable")?
            .as_ref()
        {
            owner.retire();
        }
        Ok(())
    }
    // Caller holds native initialization lock. Retain BOTH handles until the
    // awaited shutdown completes, including cancellation/concurrent cleanup.
    pub(super) async fn drain_adoption_source(
        &self,
        native: &mut Option<NativeOwner>,
    ) -> Result<(), String> {
        if !self.0.adoption_pending.load(Ordering::Acquire) {
            return Ok(());
        }
        let registered = self
            .0
            .native_retirement
            .lock()
            .map_err(|_| "management_unavailable")?
            .clone();
        if let Some(owner) = native.clone().or(registered) {
            owner.retire();
            self.close_native_owner(&owner).await?;
        }
        native.take();
        self.0
            .native_retirement
            .lock()
            .map_err(|_| "management_unavailable")?
            .take();
        self.0.adoption_pending.store(false, Ordering::Release);
        self.check()
    }
    pub(super) async fn load_adopted_initialization(
        &self,
    ) -> Result<Option<AdoptedManagementSelection>, String> {
        let existing = self
            .0
            .adopted_initialization
            .lock()
            .map_err(|_| "management_unavailable")?
            .clone();
        if existing.is_some() {
            return Ok(existing);
        }
        let selected =
            AdoptedManagementSelection::load_for_host(&self.0.state, &self.0.origin, &self.0.owner)
                .await
                .map_err(|e| e.code)?;
        self.check()?;
        *self
            .0
            .adopted_initialization
            .lock()
            .map_err(|_| "management_unavailable")? = selected.clone();
        Ok(selected)
    }
    pub(crate) async fn adopted_grant_access_until(
        &self, mut package: ManagementPackage, until: std::time::Instant,
    ) -> Result<OwnerGrantAccess, String> {
        self.verify_matrix(false).await?;
        package.account_epoch = self.0.epoch.checked_add(1).ok_or("matrix_account_changed")?;
        if let Some(remote) = self.remote_client().await? {
            return OwnerGrantAccess::remote_until(remote, package, until).await.map_err(|e|e.code);
        }
        let owner = self.native_owner().await?;
        let selected = self.load_adopted_initialization().await?.ok_or("owner_source_adoption_required")?;
        let access = selected.grant_access(&owner, package).await.map_err(|e|e.code)?.restrict_to_deadline(until).map_err(|e|e.code)?;
        if let Err(error)=self.check(){access.retire();return Err(error)}
        Ok(OwnerGrantAccess::local(access))
    }
    pub(crate) async fn adopted_grant_access(
        &self,
        mut package: ManagementPackage,
    ) -> Result<OwnerGrantAccess, String> {
        self.verify_matrix(false).await?;
        package.account_epoch = self
            .0
            .epoch
            .checked_add(1)
            .ok_or("matrix_account_changed")?;
        if let Some(remote) = self.remote_client().await? {
            return OwnerGrantAccess::remote(remote, package).await.map_err(|e| e.code);
        }
        let owner = self.native_owner().await?;
        let selected = self
            .load_adopted_initialization()
            .await?
            .ok_or("owner_source_adoption_required")?;
        let access = selected
            .grant_access(&owner, package)
            .await
            .map_err(|e| e.code)?;
        if let Err(error) = self.check() {
            access.retire();
            return Err(error);
        }
        Ok(OwnerGrantAccess::local(access))
    }
}

#[derive(Default)]
pub(super) enum ActivationTask {
    #[default]
    Empty,
    Running(tokio::task::JoinHandle<Result<AdoptedManagementSelection, String>>),
    Complete(Result<AdoptedManagementSelection, String>),
}
impl Service {
    pub(crate) async fn activation_context(
        &self,
        package: ManagementPackage,
        until: std::time::Instant,
    ) -> Result<::hagency::native_owner::ResourceActivationContext, String> {
        self.verify_matrix(false).await?;
        if let Some(remote) = self.remote_client().await? {
            return remote.device_setup_context(package, until).await.map_err(|e| e.code);
        }
        let owner = self.native_owner().await?;
        let selected = self.load_adopted_initialization().await?;
        let value = owner
            .device_setup_context(package, selected.as_ref())
            .await
            .map_err(|e| e.code)?;
        self.check()?;
        Ok(value)
    }
    pub(crate) async fn activation_review(
        &self,
        package: ManagementPackage,
        tokens: Option<u64>,
        until: std::time::Instant,
    ) -> Result<DeviceSetupReview, String> {
        self.verify_matrix(false).await?;
        if let Some(remote) = self.remote_client().await? {
            return remote.review_device_setup(package, tokens, until).await.map_err(|e| e.code);
        }
        let owner = self.native_owner().await?;
        let selected = self.load_adopted_initialization().await?;
        let value = owner
            .review_device_setup(package, selected, tokens)
            .await
            .map_err(|e| e.code)?;
        self.check()?;
        Ok(DeviceSetupReview::local(value))
    }
    pub(crate) async fn activation_state(&self) -> Result<&'static str, String> {
        self.verify_matrix(false).await?;
        if let Some(remote) = self.remote_client().await? {
            return remote.device_setup_state(ManagementPackage {
                digest: blake3::hash(b"hagency-native-device-setup-state-v1").to_hex().to_string(),
                instance: 1, account_epoch: self.0.epoch + 1,
                permissions: vec![::hagency::native_owner::ManagementPermission::ReadResources],
            }).await.map_err(|e| e.code);
        }
        let owner = self.native_owner().await?;
        let selected = self.load_adopted_initialization().await?;
        let state = if let Some(selected) = selected {
            if selected
                .device_setup_phase(&owner)
                .await
                .map_err(|e| e.code)?
                == "active"
            {
                "active"
            } else {
                "recovery_required"
            }
        } else {
            "legacy"
        };
        self.check()?;
        Ok(state)
    }
    /// Retain one confirmed native task. Screen closure or a cancelled waiter
    /// cannot drop physical source custody or submit another migration.
    pub(crate) async fn confirm_activation(
        &self,
        package: ManagementPackage,
        review: DeviceSetupReview,
        gate: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<AdoptedSelection, String> {
        self.check()?;
        if !gate() { return Err("management_scope_changed".into()); }
        if let Some(remote) = self.remote_client().await? {
            return remote.confirm_device_setup(package, review).await.map_err(|e| e.code);
        }
        let review = review.into_local().map_err(|e| e.code)?;
        let mut task = self.0.activation.lock().await;
        if matches!(*task, ActivationTask::Empty) {
            let service = self.clone();
            *task =
                ActivationTask::Running(crate::sliding_sync::backend_runtime().spawn(async move {
                    let mut native = service.0.native.lock().await;
                    service.check()?;
                    let owner = native
                        .as_ref()
                        .cloned()
                        .ok_or("owner_authorization_required")?;
                    let live = Arc::downgrade(&service.0);
                    let result = owner
                        .confirm_device_setup(review, package, move || {
                            gate() && live.upgrade().is_some_and(|i| Service(i).check().is_ok())
                        })
                        .await
                        .map_err(|e| e.code);
                    service.finish_activation(&mut native, owner, result).await
                }));
        }
        Self::await_activation(&mut task).await.map(AdoptedSelection::Local)
    }
    // Caller holds the native initialization lock for the entire cleanup.
    // Keeping this shared with cancellation tests exercises actual native revoke.
    pub(super) async fn finish_activation(
        &self,
        native: &mut Option<NativeOwner>,
        owner: NativeOwner,
        result: Result<AdoptedManagementSelection, String>,
    ) -> Result<AdoptedManagementSelection, String> {
        if let Err(code) = &result {
            let code = activation_failure_code(code);
            makepad_widgets::warning!("Native resource activation failed before cleanup: {code}");
        }
        // Always retain slots until the existing owner is drained. On error the
        // immutable descriptor/source fences drive recovery on the next opening.
        self.close_native_owner(&owner).await?;
        native.take();
        self.0
            .native_retirement
            .lock()
            .map_err(|_| "management_unavailable")?
            .take();
        drop(owner);
        if let Ok(selected) = &result {
            *self
                .0
                .adopted_initialization
                .lock()
                .map_err(|_| "management_unavailable")? = Some(selected.clone());
        }
        if self.check().is_err() {
            return Err("management_outcome_unknown".into());
        }
        result
    }
    pub(super) async fn await_activation(
        task: &mut ActivationTask,
    ) -> Result<AdoptedManagementSelection, String> {
        if let ActivationTask::Running(join) = task {
            let result = join
                .await
                .map_err(|_| "source_adoption_outcome_unknown".to_owned())
                .and_then(|v| v);
            *task = ActivationTask::Complete(result);
        }
        match task {
            ActivationTask::Complete(value) => value.clone(),
            _ => Err("source_adoption_not_started".into()),
        }
    }
    pub(crate) async fn recover_activation_task(
        &self,
    ) -> Result<Option<AdoptedSelection>, String> {
        self.check()?;
        if self.remote_client().await?.is_some() { return Ok(None); }
        let mut task = self.0.activation.lock().await;
        if matches!(*task, ActivationTask::Empty) {
            return Ok(None);
        }
        let value = Self::await_activation(&mut task).await;
        // No source handle remains in a completed result. A failure is reported
        // once; the next native review reloads the exact durable descriptor.
        if value.is_err() {
            *task = ActivationTask::Empty;
        }
        value.map(|value| Some(AdoptedSelection::Local(value)))
    }
    pub(crate) async fn activation_handoff_complete(&self) {
        *self.0.activation.lock().await = ActivationTask::Empty;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn cancelled_activation_waiter_preserves_one_task_and_exact_result() {
        let starts = Arc::new(AtomicUsize::new(0));
        let count = starts.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let mut task = ActivationTask::Running(tokio::spawn(async move {
            count.fetch_add(1, Ordering::AcqRel);
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            Err("original_saved_outcome".into())
        }));
        let mut first = Box::pin(Service::await_activation(&mut task));
        tokio::select! {
            biased;
            _ = &mut first => panic!("waiter finished before original task"),
            _ = entered_rx => {}
        }
        drop(first);
        assert!(matches!(task, ActivationTask::Running(_)));
        release_tx.send(()).unwrap();
        assert_eq!(
            Service::await_activation(&mut task).await.err().unwrap(),
            "original_saved_outcome"
        );
        assert_eq!(
            Service::await_activation(&mut task).await.err().unwrap(),
            "original_saved_outcome"
        );
        assert_eq!(starts.load(Ordering::Acquire), 1);
        assert!(matches!(task, ActivationTask::Complete(Err(_))));
    }

    #[tokio::test]
    async fn interrupted_activation_task_never_becomes_success_or_reexecutes() {
        let join = tokio::spawn(async {
            std::future::pending::<Result<AdoptedManagementSelection, String>>().await
        });
        join.abort();
        let mut task = ActivationTask::Running(join);
        for _ in 0..2 {
            assert_eq!(
                Service::await_activation(&mut task).await.err().unwrap(),
                "source_adoption_outcome_unknown"
            );
        }
        assert!(matches!(task, ActivationTask::Complete(Err(_))));
    }
}

#[cfg(test)]
mod activation_error_tests {
    #[test]
    fn activation_diagnostics_only_accept_explicit_static_codes() {
        for code in [
            "provider_identity_unavailable",
            "source_adoption_custody_unknown",
            "source_quiescence_unknown",
            "source_adoption_binding_changed",
            "fixture_root_unavailable",
        ] {
            assert_eq!(super::activation_failure_code(code), code);
        }
        assert_eq!(
            super::activation_failure_code("/private/profile bearer secret"),
            "source_adoption_unclassified_failure"
        );
    }
}
