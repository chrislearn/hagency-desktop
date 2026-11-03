//! Trusted native migration review. The script supplies no source, account,
//! ceiling or consent; original custody remains in the native flow on close.
use super::*;

#[cfg(not(feature = "resource-adoption"))]
pub(super) struct AdoptionPrompt;

#[cfg(not(feature = "resource-adoption"))]
impl MiniAppsPanel {
    pub(in crate::octoscript_apps::ui) fn begin_adoption_confirmation(
        &mut self,
        _: &mut Cx,
        _: Value,
        _: (usize, u64),
    ) -> Result<(), String> {
        Err("onboarding_unavailable".into())
    }
    pub(in crate::octoscript_apps::ui) fn present_adoption_confirmation(&mut self, cx: &mut Cx) {
        self.view
            .view(cx, ids!(adoption_confirmation))
            .set_visible(cx, false);
    }
    pub(in crate::octoscript_apps::ui) fn poll_adoption_confirmation(&mut self, _: &mut Cx) {}
    pub(in crate::octoscript_apps::ui) fn close_adoption_confirmation(&mut self, _: &mut Cx) {}
    pub(in crate::octoscript_apps::ui) fn capture_adoption_choices(&mut self, _: &mut Cx) {}
    pub(in crate::octoscript_apps::ui) fn adoption_primary(&mut self, _: &mut Cx) {}
    pub(in crate::octoscript_apps::ui) fn adoption_recover(&mut self, _: &mut Cx) {}
    pub(in crate::octoscript_apps::ui) fn adoption_key(&mut self, _: &mut Cx, _: &Event) -> bool {
        false
    }
}

#[cfg(feature = "resource-adoption")]
pub(super) use enabled::AdoptionPrompt;

#[cfg(feature = "resource-adoption")]
mod enabled {
    use super::*;
    use crate::host::hagency::adoption::{
        AdoptionContext, AdoptionFlow, AdoptionProgress, AdoptionResult, AdoptionReview, DeviceSetupKind,
    };

    pub(crate) struct AdoptionPrompt {
        flow: AdoptionFlow,
        reply: (usize, u64),
        phase: Phase,
        receiver: Option<Receiver<Result<AdoptionResult, String>>>,
        tokens: String,
    }
    enum Phase {
        Loading,
        Choose(AdoptionContext),
        Review(Box<AdoptionReview>),
        Progress(AdoptionProgress),
        Failed(String),
    }
    impl AdoptionPrompt {
        fn idle(&self) -> bool {
            self.receiver.is_none() && self.flow.active()
        }
        fn can_confirm(&self) -> bool {
            self.idle()
                && match &self.phase {
                    Phase::Choose(_) => true,
                    Phase::Review(review) => !review.expired(),
                    _ => false,
                }
        }
        fn can_recover(&self) -> bool {
            self.idle()
                && match &self.phase {
                    Phase::Failed(_) => true,
                    Phase::Progress(p) => p.state != "active",
                    _ => false,
                }
        }
    }
    fn message(code: &str) -> &'static str {
        if let Some(message)=crate::host::hagency::errors::message(code){return message;}
        match code {
            "invalid_adoption_budget" | "invalid_adoption_ceiling" => {
                "Enter a positive whole monthly token limit for the shared account."
            }
            "onboarding_review_expired" | "source_adoption_review_expired" => {
                "This migration review expired. Recover the original review before continuing."
            }
            "management_scope_changed" | "management_access_required" => {
                "The account or session changed. Open a fresh review after signing in."
            }
            "onboarding_unavailable" => "Resource migration is unavailable in this build.",
            "empty_device_review_changed" | "source_adoption_review_changed" => {
                "Resources or task history changed. Reopen the review before continuing."
            }
            "provider_identity_unavailable" => {
                "The provider account could not be verified. Check the provider signed in on this device, then recover the original migration."
            }
            "adopted_grant_device_reauthorization_required" => {
                "Use the original resource owner's device to continue this migration or grant review."
            }
            "source_installation_busy" => {
                "Another native operation owns this resource installation. Finish it before recovering this migration."
            }
            "source_adoption_preview_unavailable" => {
                "The saved migration cannot show its original task preview. Keep the original migration; do not create a replacement."
            }
            "source_adoption_inventory_unsupported" => {
                "This resource set cannot be migrated yet. Original resource and migration history remains available for recovery."
            }
            _ => {
                "The migration result is not confirmed. Recover the original migration; do not start a replacement."
            }
        }
    }
    fn context_text(context: &AdoptionContext) -> String {
        context_text_for(context, crate::i18n::language())
    }
    fn context_text_for(context: &AdoptionContext, language: crate::i18n::Language) -> String {
        let tr = |english| crate::i18n::text_for(language, english);
        let mut rows = vec![
            format!("{}\n{}", tr("Matrix server"), context.server),
            format!("{}\n{}", tr("Resource owner"), context.account),
        ];
        if context.kind == DeviceSetupKind::EmptyDevice {
            rows.push(tr("Initialize local management for this Matrix account. Then add a provider account, sign in, and choose a model.").into());
            rows.push(tr("Device setup creates no resource or shared quota. Account limits are reviewed when you register a resource.").into());
            if let Some(id) = &context.original_migration { rows.push(format!("{}\n{}",tr("Original setup request"),id)); }
            return rows.join("\n\n");
        }
        match (context.task_count, context.comment_count) {
            (Some(tasks), Some(comments)) => rows.push(format!("{}\n{}: {} · {}: {}\n{}", tr("Local task history"), tr("Tasks"), tasks, tr("Comments"), comments, tr("Task IDs, status, assignees and comments will be preserved. This does not import Matrix projects or run tasks."))),
            _ => rows.push(tr("This saved migration keeps its original resource-only scope. It does not add a task-history transfer.").into()),
        }
        match (context.alert_count, context.alert_note_count) {
            (Some(alerts), Some(notes)) => rows.push(format!("{}\n{}: {} · {}: {}\n{}", tr("Resource alert history"), tr("Resource alerts"), alerts, tr("Alert notes"), notes, tr("Alert IDs, status, assignment and notes will be preserved. Current alert checks resume after migration."))),
            _ => rows.push(tr("This saved migration keeps its original scope. It does not add an alert-history transfer.").into()),
        }
        for resource in &context.resources {
            rows.push(format!(
                "{}\n{} · {}\n{}",
                tr("Resource"),
                resource.model,
                resource.framework,
                resource.id
            ));
        }
        if let Some(id) = &context.original_migration {
            rows.push(format!("{}\n{}", tr("Original migration"), id));
        }
        if let Some(tokens) = context.original_monthly_tokens {
            rows.push(format!(
                "{}\n{}",
                tr("Shared monthly account limit"),
                crate::utils::format_token_count(tokens)
            ));
        }
        rows.join("\n\n")
    }
    fn close_reply(phase: Phase, in_flight: bool, active: bool) -> Result<String, String> {
        if !active {
            return Err("management_scope_changed".into());
        }
        if in_flight {
            return Ok(json!({"state":"outcome_unknown"}).to_string());
        }
        match phase {
            Phase::Progress(progress) => serde_json::to_string(&progress)
                .map_err(|_| "management_outcome_unknown".to_owned()),
            Phase::Failed(code) => Ok(json!({"state":"outcome_unknown","reason":code}).to_string()),
            _ => Ok(json!({"state":"review_closed"}).to_string()),
        }
    }
    impl MiniAppsPanel {
        pub(in crate::octoscript_apps::ui) fn begin_adoption_confirmation(
            &mut self,
            cx: &mut Cx,
            args: Value,
            reply: (usize, u64),
        ) -> Result<(), String> {
            if self.execution_confirmation.is_some()
                || self.adoption_confirmation.is_some()
                || self.grant_confirmation.is_some()
                || self.provider_confirmation.is_some()
                || self.account_login.is_some()
                || self.reauthentication.is_some()
                || self.project_room_reply.is_some()
                || self.pending.len() >= 16
            {
                return Err("provider_confirmation_busy".into());
            }
            let flow = self
                .hagency
                .as_ref()
                .ok_or("management_session_required")?
                .onboarding_handoff(
                    self.lease.clone().ok_or("management_session_required")?,
                    args,
                )?;
            self.adoption_confirmation = Some(AdoptionPrompt {
                flow: flow.clone(),
                reply,
                phase: Phase::Loading,
                receiver: None,
                tokens: String::new(),
            });
            self.adoption_job(async move { flow.open().await });
            self.present_adoption_confirmation(cx);
            cx.set_key_focus(
                self.view
                    .button(cx, ids!(adoption_confirmation.buttons.cancel))
                    .area(),
            );
            Ok(())
        }
        fn adoption_job(
            &mut self,
            task: impl std::future::Future<Output = Result<AdoptionResult, String>> + Send + 'static,
        ) {
            let Some(prompt) = &mut self.adoption_confirmation else {
                return;
            };
            let (tx, receiver) = mpsc::sync_channel(1);
            prompt.receiver = Some(receiver);
            prompt.phase = Phase::Loading;
            crate::sliding_sync::spawn_async_task(async move {
                let result = task.await;
                let _ = tx.try_send(result);
                SignalToUI::set_ui_signal();
            });
        }
        pub(in crate::octoscript_apps::ui) fn present_adoption_confirmation(
            &mut self,
            cx: &mut Cx,
        ) {
            self.view
                .view(cx, ids!(adoption_confirmation))
                .set_visible(cx, self.adoption_confirmation.is_some());
            let Some(prompt) = &self.adoption_confirmation else {
                return;
            };
            self.view.view(cx, ids!(app_content)).set_visible(cx, false);
            self.view.view(cx, ids!(approval)).set_visible(cx, false);
            self.view
                .label(cx, ids!(adoption_confirmation.scroll.owner))
                .set_text(cx, prompt.flow.account());
            let tr = crate::i18n::tr;
            let empty_setup = matches!(&prompt.phase,Phase::Review(review) if review.context().kind == DeviceSetupKind::EmptyDevice);
            let heading = if matches!(&prompt.phase, Phase::Progress(p) if p.state == "active") {
                "Device setup completed"
            } else if empty_setup || matches!(prompt.phase, Phase::Loading) {
                "Review device setup"
            } else {
                "Review resource migration"
            };
            self.view.label(cx, ids!(adoption_confirmation.scroll.heading.title)).set_text(cx, tr(heading));
            let choosing = matches!(prompt.phase, Phase::Choose(_));
            self.view
                .view(cx, ids!(adoption_confirmation.scroll.choices))
                .set_visible(cx, choosing);
            if choosing {
                self.view
                    .widget(cx, ids!(adoption_confirmation.scroll.choices.tokens))
                    .set_text(cx, &prompt.tokens);
            }
            let(mut status,details,primary)=match &prompt.phase {
                Phase::Loading=>(tr("Checking device setup…").to_owned(),String::new(),"Review migration"),
                Phase::Choose(context)=>(tr("Enter the shared monthly account limit. The next screen reviews the exact resources before any migration.").to_owned(),context_text(context),"Review migration"),
                Phase::Review(review)=>{
                    if review.context().kind == DeviceSetupKind::EmptyDevice {
                        (tr("Confirm to set up local management on this device. Existing records are preserved. You can add and authorize a provider account afterward.").to_owned(),context_text(review.context()),if review.recovering(){"Resume device setup"}else{"Set up this device"})
                    } else {
                        let mut details=context_text(review.context());
                        if let Some(tokens)=review.monthly_tokens(){details.push_str(&format!("\n\n{}\n{}",tr("Shared monthly account limit"),crate::utils::format_token_count(tokens)));}
                        (tr("Confirming stops existing local workers, preserves resource identities and original history, and moves management into the shared native registry. This does not reserve capacity for a server or start an agent.").to_owned(),details,if review.recovering(){"Resume migration"}else{"Move resources"})
                    }
                }
                Phase::Progress(p)=>(tr(match p.state.as_str(){
                    "active"=>"Device setup is ready. You can add an account and register resources. Shared capacity requires a separate review.",
                    "blocked"=>"Native requirements are incomplete. Recover the original migration to inspect its current result.",
                    _=>"The migration result is not confirmed. Recover the original migration; do not start a replacement.",
                }).to_owned(),p.migration_id.as_ref().map(|id|format!("{}\n{}",tr("Original migration"),id)).unwrap_or_default(),"Review migration"),
                Phase::Failed(code)=>(tr(message(code)).to_owned(),String::new(),"Review migration"),
            };
            if !prompt.flow.active() {
                status = tr(message("management_scope_changed")).into();
            }
            self.view
                .label(cx, ids!(adoption_confirmation.scroll.status))
                .set_text(cx, &status);
            self.view
                .label(cx, ids!(adoption_confirmation.scroll.details.copy))
                .set_text(cx, &details);
            self.view
                .view(cx, ids!(adoption_confirmation.scroll.details))
                .set_visible(cx, !details.is_empty());
            self.view
                .button(cx, ids!(adoption_confirmation.buttons.primary))
                .set_text(cx, tr(primary));
            self.view
                .widget(cx, ids!(adoption_confirmation.buttons.primary))
                .set_visible(cx, prompt.can_confirm());
            self.view
                .widget(cx, ids!(adoption_confirmation.buttons.recover))
                .set_visible(cx, prompt.can_recover());
            self.request_confirmation_redraw(cx);
        }
        pub(in crate::octoscript_apps::ui) fn poll_adoption_confirmation(&mut self, cx: &mut Cx) {
            let Some(prompt) = &mut self.adoption_confirmation else {
                return;
            };
            let result = prompt.receiver.as_ref().and_then(|rx| match rx.try_recv() {
                Ok(value) => Some(value),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("management_outcome_unknown".into()))
                }
                Err(_) => None,
            });
            let mut changed = false;
            if let Some(result) = result {
                prompt.receiver = None;
                prompt.phase = match result {
                    Ok(AdoptionResult::Choose(c)) => Phase::Choose(c),
                    Ok(AdoptionResult::Review(r)) => Phase::Review(r),
                    Ok(AdoptionResult::Progress(p)) => Phase::Progress(p),
                    Err(e) => Phase::Failed(e),
                };
                changed = true;
            }
            if matches!(&prompt.phase,Phase::Review(r) if r.expired()) {
                prompt.phase = Phase::Failed("onboarding_review_expired".into());
                changed = true;
            }
            if !prompt.flow.active()
                && !matches!(&prompt.phase,Phase::Failed(code) if code=="management_scope_changed")
            {
                prompt.phase = Phase::Failed("management_scope_changed".into());
                changed = true;
            }
            if changed {
                self.present_adoption_confirmation(cx);
            }
        }
        pub(in crate::octoscript_apps::ui) fn capture_adoption_choices(&mut self, cx: &mut Cx) {
            if let Some(prompt) = &mut self.adoption_confirmation
                && matches!(prompt.phase, Phase::Choose(_))
            {
                prompt.tokens = self
                    .view
                    .widget(cx, ids!(adoption_confirmation.scroll.choices.tokens))
                    .text();
            }
        }
        pub(in crate::octoscript_apps::ui) fn adoption_primary(&mut self, cx: &mut Cx) {
            self.capture_adoption_choices(cx);
            self.poll_adoption_confirmation(cx);
            let Some(prompt) = &mut self.adoption_confirmation else {
                return;
            };
            if !prompt.can_confirm() {
                return;
            }
            if matches!(prompt.phase, Phase::Choose(_)) && !valid_tokens(&prompt.tokens) {
                self.view
                    .label(cx, ids!(adoption_confirmation.scroll.status))
                    .set_text(cx, crate::i18n::tr(message("invalid_adoption_budget")));
                return;
            }
            let flow = prompt.flow.clone();
            match std::mem::replace(&mut prompt.phase, Phase::Loading) {
                Phase::Choose(_) => {
                    let tokens = prompt.tokens.clone();
                    self.adoption_job(async move { flow.prepare(tokens).await });
                }
                Phase::Review(review) => {
                    self.adoption_job(async move { flow.confirm(*review).await })
                }
                phase => {
                    prompt.phase = phase;
                    return;
                }
            }
            self.present_adoption_confirmation(cx);
        }
        pub(in crate::octoscript_apps::ui) fn adoption_recover(&mut self, cx: &mut Cx) {
            let Some(prompt) = &self.adoption_confirmation else {
                return;
            };
            if !prompt.can_recover() {
                return;
            }
            let flow = prompt.flow.clone();
            self.adoption_job(async move { flow.open().await });
            self.present_adoption_confirmation(cx);
        }
        pub(in crate::octoscript_apps::ui) fn close_adoption_confirmation(&mut self, cx: &mut Cx) {
            let Some(prompt) = self.adoption_confirmation.take() else {
                return;
            };
            let active = prompt.flow.active();
            let result = close_reply(prompt.phase, prompt.receiver.is_some(), active);
            splash_host_respond(
                cx,
                prompt.reply.0,
                prompt.reply.1,
                result.as_ref().map(String::as_str).map_err(String::as_str),
            );
            self.present_adoption_confirmation(cx);
            self.view.view(cx, ids!(app_content)).set_visible(cx, true);
            self.reply_redraw = cx.new_next_frame();
            self.view.redraw(cx);
        }
        pub(in crate::octoscript_apps::ui) fn adoption_key(
            &mut self,
            cx: &mut Cx,
            event: &Event,
        ) -> bool {
            let Some(prompt) = &self.adoption_confirmation else {
                return false;
            };
            let Event::KeyDown(key) = event else {
                return false;
            };
            if key.key_code == KeyCode::Escape {
                self.close_adoption_confirmation(cx);
                return true;
            }
            let cancel = self
                .view
                .button(cx, ids!(adoption_confirmation.buttons.cancel))
                .area();
            let primary = self
                .view
                .button(cx, ids!(adoption_confirmation.buttons.primary))
                .area();
            let recover = self
                .view
                .button(cx, ids!(adoption_confirmation.buttons.recover))
                .area();
            let mut targets = Vec::new();
            if matches!(prompt.phase, Phase::Choose(_)) {
                let decrease=self.view.button(cx, ids!(adoption_confirmation.scroll.choices.tokens.decrease));
                let increase=self.view.button(cx, ids!(adoption_confirmation.scroll.choices.tokens.increase));
                if !decrease.disabled(cx) {targets.push(decrease.area());}
                targets.push(
                    self.view
                        .text_input(cx, ids!(adoption_confirmation.scroll.choices.tokens.input))
                        .area(),
                );
                if !increase.disabled(cx) {targets.push(increase.area());}
            }
            targets.push(cancel);
            if prompt.can_confirm() {
                targets.push(primary);
            }
            if prompt.can_recover() {
                targets.push(recover);
            }
            if key.key_code == KeyCode::Tab {
                let current = targets.iter().position(|area| cx.has_key_focus(*area));
                cx.set_key_focus(targets[next_focus(current, targets.len(), key.modifiers.shift)]);
                return true;
            }
            if matches!(key.key_code, KeyCode::ReturnKey | KeyCode::Space) {
                if cx.has_key_focus(cancel) {
                    self.close_adoption_confirmation(cx);
                    return true;
                }
                if prompt.can_confirm() && cx.has_key_focus(primary) {
                    self.adoption_primary(cx);
                    return true;
                }
                if prompt.can_recover() && cx.has_key_focus(recover) {
                    self.adoption_recover(cx);
                    return true;
                }
            }
            false
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn empty_setup_preview_has_identity_and_recovery_without_migration_quota() {
            let context = AdoptionContext {
                kind: DeviceSetupKind::EmptyDevice,
                personal_database_present: Some(false),
                completed_private_records: Some(0),
                server: "https://matrix.test".into(),
                account: "@owner:test".into(),
                resources: vec![],
                original_migration: Some("original-setup".into()),
                original_monthly_tokens: None,
                task_count: None,
                comment_count: None,
                alert_count: None,
                alert_note_count: None,
            };
            for language in [crate::i18n::Language::English, crate::i18n::Language::Chinese] {
                let tr = |text| crate::i18n::text_for(language, text);
                let text = context_text_for(&context, language);
                assert!(text.contains("https://matrix.test") && text.contains("@owner:test"));
                assert!(text.contains("original-setup"));
                assert!(text.contains(tr("Original setup request")));
                assert!(text.contains(tr("Device setup creates no resource or shared quota. Account limits are reviewed when you register a resource.")));
                assert!(!text.contains(tr("Shared monthly account limit")));
                assert!(!text.contains(tr("This saved migration keeps its original resource-only scope. It does not add a task-history transfer.")));
            }
        }
        #[test]
        fn task_history_preview_distinguishes_zero_populated_and_saved_v1_in_both_languages() {
            for language in [
                crate::i18n::Language::English,
                crate::i18n::Language::Chinese,
            ] {
                let tr = |text| crate::i18n::text_for(language, text);
                let mut context = AdoptionContext {
                    kind: DeviceSetupKind::ExistingResources,
                    personal_database_present: None,
                    completed_private_records: None,
                    server: "https://matrix.test".into(),
                    account: "@owner:test".into(),
                    resources: vec![],
                    original_migration: Some("original-migration".into()),
                    original_monthly_tokens: Some(700),
                    task_count: Some(12),
                    comment_count: Some(23),
                    alert_count: Some(5),
                    alert_note_count: Some(9),
                };
                let text = context_text_for(&context, language);
                assert!(text.contains(&format!("{}: 12", tr("Tasks"))));
                assert!(text.contains(&format!("{}: 23", tr("Comments"))));
                assert!(text.contains("original-migration"));
                assert!(text.contains(&format!("{}: 5", tr("Resource alerts"))));
                assert!(text.contains(&format!("{}: 9", tr("Alert notes"))));
                context.alert_count = Some(0);
                context.alert_note_count = Some(0);
                assert!(
                    context_text_for(&context, language).contains(&format!("{}: 0", tr("Resource alerts")))
                );
                context.alert_count = None;
                context.alert_note_count = None;
                let prior = context_text_for(&context, language);
                assert!(prior.contains(tr("This saved migration keeps its original scope. It does not add an alert-history transfer.")));
                assert!(!prior.contains(&format!("{}: 0", tr("Resource alerts"))));
                context.task_count = Some(0);
                context.comment_count = Some(0);
                assert!(
                    context_text_for(&context, language).contains(&format!("{}: 0", tr("Tasks")))
                );
                context.task_count = None;
                context.comment_count = None;
                let text = context_text_for(&context, language);
                assert!(text.contains(tr("This saved migration keeps its original resource-only scope. It does not add a task-history transfer.")));
                assert!(!text.contains(&format!("{}: 0", tr("Tasks"))));
            }
        }
        #[test]
        fn closing_never_turns_interruption_or_retirement_into_activation() {
            let progress = || {
                Phase::Progress(AdoptionProgress {
                    state: "active".into(),
                    migration_id: Some("original".into()),
                    reason: None,
                })
            };
            let read = |reply: String| serde_json::from_str::<Value>(&reply).unwrap();
            assert_eq!(
                read(close_reply(progress(), true, true).unwrap())["state"],
                "outcome_unknown"
            );
            assert!(close_reply(progress(), false, false).is_err());
            assert_eq!(
                read(close_reply(Phase::Failed("interrupted".into()), false, true).unwrap())["state"],
                "outcome_unknown"
            );
            let p = read(close_reply(progress(), false, true).unwrap());
            assert_eq!(p["state"], "active");
            assert_eq!(p["migrationId"], "original");
            let choosing = Phase::Choose(AdoptionContext {
                kind: DeviceSetupKind::ExistingResources,
                personal_database_present: None,
                completed_private_records: None,
                server: "https://matrix.test".into(),
                account: "@owner:matrix.test".into(),
                resources: vec![],
                original_migration: None,
                original_monthly_tokens: None,
                task_count: Some(0),
                comment_count: Some(0),
                alert_count: Some(0),
                alert_note_count: Some(0),
            });
            assert_eq!(
                read(close_reply(choosing, false, true).unwrap())["state"],
                "review_closed"
            );
        }
    }
}

#[cfg(any(feature = "resource-adoption", test))]
fn valid_tokens(value: &str) -> bool {
    value
        .trim()
        .parse::<u64>()
        .is_ok_and(|n| (1..=9_007_199_254_740_991).contains(&n))
}
#[cfg(any(feature = "resource-adoption", test))]
fn next_focus(current: Option<usize>, count: usize, reverse: bool) -> usize {
    match current {
        None => {
            if reverse {
                count - 1
            } else {
                0
            }
        }
        Some(index) => {
            if reverse {
                (index + count - 1) % count
            } else {
                (index + 1) % count
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ceiling_is_explicit_bounded_and_not_fractional() {
        for value in ["", "0", "-1", "1.5", "9007199254740992", "unknown"] {
            assert!(!valid_tokens(value));
        }
        assert!(valid_tokens(" 700 "));
        assert!(valid_tokens("9007199254740991"));
    }
    #[test]
    fn keyboard_reaches_close_and_confirmation_without_auto_confirm() {
        assert_eq!(next_focus(Some(0), 3, false), 1);
        assert_eq!(next_focus(Some(2), 3, false), 0);
        assert_eq!(next_focus(Some(0), 3, true), 2);
        assert_eq!(next_focus(None, 2, false), 0);
    }
    #[test]
    fn native_migration_footer_stays_visible_with_long_english_and_chinese_history() {
        use makepad_widgets::makepad_draw::cx_draw::CxDraw;
        for (width, height, cn) in [
            (360.0, 640.0, false),
            (360.0, 640.0, true),
            (840.0, 700.0, false),
            (840.0, 700.0, true),
        ] {
            let mut cx = Cx::new(Box::new(|_, _| {}));
            let mut panel = cx.with_vm(|vm| {
                makepad_widgets::script_mod(vm);
                crate::i18n::install(vm);
                makepad_code_editor::script_mod(vm);
                crate::shared::script_mod(vm);
                vm.bx.captured_errors = Some(Vec::new());
                crate::miniapps::script_mod(vm);
                let value =
                    vm.eval(script! {use mod.prelude.widgets.* use mod.widgets.* MiniAppsPanel{}});
                let panel = MiniAppsPanel::script_from_value(vm, value);
                assert!(vm.take_errors().is_empty());
                panel
            });
            for id in [
                ids!(import_form),
                ids!(app_content),
                ids!(notice),
                ids!(approval),
                ids!(grant_confirmation),
            ] {
                panel.view.widget(&cx, id).set_visible(&mut cx, false);
            }
            panel
                .view
                .view(&cx, ids!(adoption_confirmation))
                .set_visible(&mut cx, true);
            let long = if cn {
                "原始资源标识\n资源模型与共享月度上限\n"
            } else {
                "Original resource identity\nResource model and shared monthly ceiling\n"
            }
            .repeat(80);
            panel
                .view
                .label(&cx, ids!(adoption_confirmation.scroll.details.copy))
                .set_text(&mut cx, &long);
            if cn {
                for (id, text) in [
                    (ids!(adoption_confirmation.buttons.cancel), "关闭核对"),
                    (ids!(adoption_confirmation.buttons.primary), "恢复迁移"),
                    (ids!(adoption_confirmation.buttons.recover), "恢复原始操作"),
                ] {
                    panel.view.button(&cx, id).set_text(&mut cx, text);
                }
            }
            let pass = DrawPass::new(&mut cx);
            pass.set_size(&mut cx, dvec2(width, height));
            let mut list = DrawList2d::new(&mut cx);
            let event = DrawEvent::default();
            let mut draw = CxDraw::new(&mut cx, &event);
            let mut cx2d = Cx2d::new(&mut draw);
            cx2d.begin_pass(&pass, None);
            list.begin_always(&mut cx2d);
            cx2d.begin_root_turtle(dvec2(width, height), Layout::flow_overlay());
            panel.draw_all(&mut cx2d, &mut Scope::empty());
            cx2d.end_pass_sized_turtle();
            list.end(&mut cx2d);
            cx2d.end_pass(&pass);
            drop(cx2d);
            drop(draw);
            for id in [
                ids!(adoption_confirmation.buttons.cancel),
                ids!(adoption_confirmation.buttons.primary),
                ids!(adoption_confirmation.buttons.recover),
            ] {
                let r = panel.view.widget(&cx, id).area().rect(&cx);
                assert!(
                    r.size.x > 0.0
                        && r.size.y >= 40.0
                        && r.pos.x >= 0.0
                        && r.pos.y >= 0.0
                        && r.pos.x + r.size.x <= width + 0.5
                        && r.pos.y + r.size.y <= height + 0.5,
                    "clipped migration control {id:?}: {r:?} at {width}x{height}"
                );
            }
        }
    }
}
