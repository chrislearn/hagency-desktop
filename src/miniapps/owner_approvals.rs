//! Account-bound, one-shot navigation into the shipped owner review flow.
//! A handover is acknowledged by the installed script after opening the page,
//! not merely because the hosting dashboard or application window appeared.
use super::*;

pub(super) struct Pending {
    owner: String,
    handover_id: Option<u64>,
    nonce: String,
    sent: bool,
    started: Instant,
}

impl Pending {
    fn matches(&self, owner: &str, nonce: &str) -> bool {
        self.owner == owner && self.nonce == nonce && self.started.elapsed() < Duration::from_secs(60)
    }
}

impl MiniAppsPanel {
    pub(super) fn open_owner_approvals(&mut self, cx: &mut Cx, owner: &str, handover_id: Option<u64>) {
        if crate::sliding_sync::current_user_id().as_deref().map(|id| id.as_str()) != Some(owner) {
            if let Some(id) = handover_id {
                cx.action(crate::hagency::owner_dashboard::EmbeddedAction::Restored { id, owner: owner.into(), accepted: false });
            }
            return;
        }
        self.finish_pending_owner_approvals(cx, false);
        let running = self.open && !self.showing_catalog && !self.showing_hub
            && self.package.as_ref().and_then(Package::builtin_id) == Some(octosense_app_contract::hagency::APP_ID);
        self.open = true;
        self.direct_agent = true;
        if !running {
            self.palpo_action = None;
            self.board = None;
            let result = crate::system_apps::apps().iter().position(|app| app.manifest.id == octosense_app_contract::hagency::APP_ID)
                .ok_or_else(|| "Hagency app is not installed".to_owned())
                .and_then(|index| self.open_builtin(cx, index));
            if let Err(error) = result {
                self.notice(cx, &error);
                if let Some(id) = handover_id {
                    cx.action(crate::hagency::owner_dashboard::EmbeddedAction::Restored { id, owner: owner.into(), accepted: false });
                }
                return;
            }
        }
        // May wait for the person's first Run consent. It expires and can
        // never migrate to a newly signed-in account or a different package.
        self.owner_approvals = Some(Pending {
            owner: owner.into(), handover_id, nonce: uuid::Uuid::new_v4().to_string(), sent: false, started: Instant::now(),
        });
        self.pump_owner_approvals(cx);
    }

    pub(super) fn pump_owner_approvals(&mut self, cx: &mut Cx) {
        let Some(pending) = &self.owner_approvals else { return };
        let owner = crate::sliding_sync::current_user_id().map(|id| id.to_string()).unwrap_or_default();
        if !pending.matches(&owner, &pending.nonce)
            || self.package.as_ref().and_then(Package::builtin_id) != Some(octosense_app_contract::hagency::APP_ID)
        {
            self.finish_pending_owner_approvals(cx, false);
            return;
        }
        if pending.sent || !self.open { return; }
        let Some(lease) = &self.lease else { return; };
        if lease.authorize(&owner, "hagency.navigation.open", None).is_err() {
            self.finish_pending_owner_approvals(cx, false);
            return;
        }
        let nonce = pending.nonce.clone();
        if self.view.splash(cx, ids!(card)).call_script_fn_with_strings(cx, id!(on_owner_approvals_requested), &[&owner, &nonce]) {
            if let Some(pending) = &mut self.owner_approvals { pending.sent = true; }
            self.reply_redraw = cx.new_next_frame();
        } else {
            self.finish_pending_owner_approvals(cx, false);
        }
    }

    pub(super) fn finish_owner_approvals(&mut self, cx: &mut Cx, args: &serde_json::Map<String, Value>, owner: &str) -> Result<(), String> {
        let nonce = args.get("requestId").and_then(Value::as_str).ok_or("invalid_management_navigation")?;
        if args.len() != 2 || !self.owner_approvals.as_ref().is_some_and(|pending| pending.sent && pending.matches(owner, nonce)) {
            return Err("management_navigation_expired".into());
        }
        let accepted = args.get("destination").and_then(Value::as_str) == Some("owner_approvals_ready");
        self.finish_pending_owner_approvals(cx, accepted);
        Ok(())
    }

    pub(super) fn finish_pending_owner_approvals(&mut self, cx: &mut Cx, accepted: bool) {
        if let Some(pending) = self.owner_approvals.take() {
            if let Some(id) = pending.handover_id {
                cx.action(crate::hagency::owner_dashboard::EmbeddedAction::Restored { id, owner: pending.owner, accepted });
            }
            if !accepted {
                self.notice(cx, match crate::i18n::language() {
                    crate::i18n::Language::Chinese => "未能打开审批。请完成当前操作，或按提示点击运行，然后重试。",
                    crate::i18n::Language::English => "Approvals could not be opened. Finish the current action, or select Run if asked, then retry.",
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgment_is_owner_nonce_and_time_bound() {
        let mut pending = Pending { owner: "@owner:example.org".into(), nonce: "fresh-request".into(), handover_id: Some(7), sent: true, started: Instant::now() };
        assert!(pending.matches("@owner:example.org", "fresh-request"));
        assert!(!pending.matches("@other:example.org", "fresh-request"));
        assert!(!pending.matches("@owner:example.org", "old-request"));
        pending.started -= Duration::from_secs(61);
        assert!(!pending.matches("@owner:example.org", "fresh-request"));
    }
}
