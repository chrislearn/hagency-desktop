//! Embedded host for the same component shipped in Hagency Hosting.app.
//! Execution and filesystem IO belong to background workers, never this widget.
use makepad_widgets::*;
use owner_dashboard::{
    CommandKind, CommandResult, ConnectionStatus, DashboardAction, HostMode, Locale,
    OwnerDashboardRef, OwnerDashboardWidgetExt, ViewState,
    handover::Host,
    handover_bridge::{Bridge, Event as HandoverEvent},
    transport::{ConnectorEvent, DaemonConnector, DaemonEndpoint, Job, OwnerScope},
};
use std::path::PathBuf;

/// Include the app's sidebar in the same native accessibility tree as Hosting.
pub fn register_host_accessibility(cx: &mut Cx) {
    use crate::shared::navigation_bar_button::{NavigationBarButton, NavigationBarButtonAction};
    use owner_dashboard::accessibility::{Control, ControlAdapter, register_control_adapter};
    register_control_adapter(cx, ControlAdapter {
        name: "desktop-navigation",
        describe: |widget, cx| {
            let button = widget.borrow::<NavigationBarButton>()?;
            let label = button.tooltip_text().to_owned();
            (!label.is_empty()).then(|| Control { label, selected: Some(button.is_selected(cx)), disabled: false })
        },
        activate: |widget, cx| cx.widget_action(widget.widget_uid(), NavigationBarButtonAction::Clicked),
    });
}

pub fn register(vm: &mut ScriptVm) {
    owner_dashboard::script_mod(vm);
    self::script_mod(vm);
}

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    mod.widgets.OwnerDashboardHost = #(OwnerDashboardHost::register_widget(vm)) {
        width: Fill height: Fill flow: Down
        host_notice := RinxHint {visible: false width: Fill height: Fit padding: 12 text: ""}
        dashboard := OwnerDashboard {}
    }
}

#[derive(Clone, Debug)]
pub enum EmbeddedAction {
    Restore { id: u64, view: ViewState },
    Restored { id: u64, owner: String, accepted: bool },
    Offer(ViewState),
    Notice(String),
    SetupResult { owner: String, message: String },
}

/// App-level receiver remains available while the Agent surface is closed.
#[derive(Default)]
pub struct Coordinator {
    owner: Option<String>,
    bridge: Option<Bridge>,
    #[cfg(feature = "onboarding-test-support")]
    fixture_owner: Option<String>,
}

pub fn endpoint() -> DaemonEndpoint {
    let home = std::env::var_os("HAGENCY_OWNER_DAEMON_HOME").map(PathBuf::from)
        .unwrap_or_else(|| {
            let user = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            if cfg!(target_os = "macos") {
                user.join("Library/Application Support/Hagency/owner-daemon")
            } else {
                std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
                    .unwrap_or_else(|| user.join(".local/state"))
                    .join("hagency/owner-daemon")
            }
        });
    DaemonEndpoint::new(home)
}

impl Coordinator {
    #[cfg(feature = "onboarding-test-support")]
    pub fn fixture(home: PathBuf) -> std::io::Result<Self> {
        let owner = owner_dashboard::fixtures::OWNER.to_owned();
        let bridge = Bridge::spawn_fixture(home, Host::Embedded, SignalToUI::set_ui_signal)?;
        bridge.set_owner(Some(owner.clone()));
        Ok(Self { owner: Some(owner.clone()), bridge: Some(bridge), fixture_owner: Some(owner) })
    }
    pub fn handle_event(&mut self, cx: &mut Cx, event: &Event) -> Option<EmbeddedAction> {
        let owner = crate::sliding_sync::current_user_id().map(|id| id.to_string());
        #[cfg(feature = "onboarding-test-support")]
        let owner = self.fixture_owner.clone().or(owner);
        if owner != self.owner {
            self.bridge = None;
            self.owner = owner.clone();
            if owner.is_some() {
                self.bridge = Bridge::spawn(endpoint().home, Host::Embedded, SignalToUI::set_ui_signal).ok();
                if let Some(bridge) = &self.bridge { bridge.set_owner(owner); }
            }
        }
        let bridge = self.bridge.as_ref()?;
        if let Event::Actions(actions) = event {
            for action in actions {
                match action.downcast_ref::<EmbeddedAction>() {
                    Some(EmbeddedAction::Offer(view)) if view.owner_id == self.owner => {
                        if !bridge.offer(view.clone()) {
                            cx.action(EmbeddedAction::Notice(message("handover_busy")));
                        }
                    }
                    Some(EmbeddedAction::Restored { id, owner, accepted }) if self.owner.as_deref() == Some(owner) => {
                        if *accepted { bridge.restored(*id); } else { bridge.rejected(*id); }
                    }
                    _ => {},
                }
            }
        }
        while let Some(event) = bridge.try_recv() {
            match event {
                HandoverEvent::Restoring { id, view } if view.owner_id == self.owner => return Some(EmbeddedAction::Restore { id, view }),
                HandoverEvent::Opening => cx.action(EmbeddedAction::Notice(message("opening"))),
                // "Open separately" keeps this view available. The other host
                // has acknowledged the state, so success can now be displayed.
                HandoverEvent::Completed => cx.action(EmbeddedAction::Notice(message("opened"))),
                HandoverEvent::Failed(code) => cx.action(EmbeddedAction::Notice(message(code))),
                _ => {},
            }
        }
        None
    }
}

fn locale() -> Locale {
    match crate::i18n::language() {
        crate::i18n::Language::Chinese => Locale::ZhHans,
        crate::i18n::Language::English => Locale::En,
    }
}

fn message(code: &str) -> String {
    let zh = locale() == Locale::ZhHans;
    match (code, zh) {
        ("opening", true) => "正在打开独立控制面板…",
        ("opening", false) => "Opening the separate dashboard…",
        ("opened", true) => "已在独立控制面板打开。两个窗口会同步显示最新用量。",
        ("opened", false) => "Opened in the separate dashboard. Both views will show current usage.",
        ("setup", true) => "先在 Hagency Hosting 中连接资源主人并启用后台服务。现有 Agent 的迁移需要完成执行权交接。",
        ("setup", false) => "Connect the resource owner in Hagency Hosting and enable background hosting. Existing agents require execution handover.",
        ("opening_setup", true) => "正在打开后台托管设置窗口…",
        ("opening_setup", false) => "Opening the hosting setup window…",
        ("sign_in_required", true) => "请先登录 Hagency，再打开此账号的资源控制面板。",
        ("sign_in_required", false) => "Sign in to Hagency to open this account’s hosting dashboard.",
        ("platform_unsupported", true) => "本版本的后台托管仅支持 macOS。Windows 和 Linux 的支持正在开发中；此设备仍可作为聊天和请求方使用。",
        ("platform_unsupported", false) => "Background hosting is macOS-only in this release. Windows and Linux support is in progress; this computer can still chat and request Agents.",
        (_, true) => "尚未确认目标窗口已打开。当前页面已保留，请确认 Hagency Hosting 已安装且登录同一账号后重试。",
        (_, false) => "The destination has not confirmed opening. This view is preserved. Check that Hagency Hosting is installed and uses the same account, then retry.",
    }.into()
}

fn open_setup() {
    static OPENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if OPENING.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
    let launched = std::thread::Builder::new().name("owner-dashboard-setup".into()).spawn(move || {
        struct Reset;
        impl Drop for Reset { fn drop(&mut self) { OPENING.store(false, std::sync::atomic::Ordering::Release); } }
        let _reset = Reset;
        let source = crate::hagency::backend::Service::for_current()
            .and_then(|service| service.daemon_handover_source());
        let Ok((origin, owner, root)) = source else {
            Cx::post_action(EmbeddedAction::Notice(message("sign_in_required")));
            return;
        };
        let home = endpoint().home;
        #[cfg(target_os = "macos")]
        let success = (|| -> std::io::Result<bool> {
            use owner_dashboard::{handover::Status, setup_launch::SetupLaunch};
            let now = || owner_dashboard::Clock::system().now_ms;
            let launch = SetupLaunch::prepare(&home, &owner, &origin, &root, now())?;
            let status = launch.command(&home, &origin, &root)
                .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()).status()?;
            if !status.success() { return Ok(false); }
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(115) {
                match launch.status(now())? {
                    Status::Acknowledged => return Ok(true),
                    Status::Expired => return Ok(false),
                    Status::Waiting => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
            Ok(false)
        })().unwrap_or(false);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (origin, home, root);
            Cx::post_action(EmbeddedAction::SetupResult { owner, message: message("platform_unsupported") });
            return;
        }
        #[cfg(target_os = "macos")]
        Cx::post_action(EmbeddedAction::SetupResult { owner, message: message(if success { "setup" } else { "destination_unavailable" }) });
    });
    if launched.is_err() {
        OPENING.store(false, std::sync::atomic::Ordering::Release);
        Cx::post_action(EmbeddedAction::Notice(message("handover_busy")));
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct OwnerDashboardHost {
    #[deref] view: View,
    #[rust] active: bool,
    #[rust] owner: Option<String>,
    #[rust] connector: Option<DaemonConnector>,
    #[rust] refresh: Timer,
    #[rust] configured: bool,
    #[rust] language: Option<Locale>,
    #[rust] fixture: bool,
    #[cfg(feature = "onboarding-test-support")]
    #[rust] fixture_handover: bool,
}

impl OwnerDashboardHost {
    fn dashboard(&self, cx: &mut Cx) -> OwnerDashboardRef { self.view.owner_dashboard(cx, ids!(dashboard)) }
    fn synchronize(&mut self, cx: &mut Cx) {
        if self.fixture { return }
        let owner = crate::sliding_sync::current_user_id().map(|id| id.to_string());
        let dashboard = self.dashboard(cx);
        if !self.configured {
            dashboard.set_host(cx, HostMode::Embedded);
            self.configured = true;
        }
        let language = locale();
        if self.language != Some(language) {
            dashboard.set_locale(cx, language);
            self.language = Some(language);
        }
        if owner != self.owner {
            // Drop old event queue before changing identities. Late replies
            // cannot populate the next account's dashboard.
            self.connector = None;
            self.owner = owner.clone();
            dashboard.set_owner_scope(cx, owner.as_deref());
        }
        if self.active && self.connector.is_none() {
            if let Some(owner) = owner {
                self.connector = Some(DaemonConnector::spawn(endpoint(), OwnerScope::Fixed(owner), SignalToUI::set_ui_signal));
                self.submit(cx, Job::Snapshot { period: dashboard.view_state(cx).period, usage_filter: Some(dashboard.view_state(cx).usage_filter()) });
            } else {
                dashboard.set_connection(cx, ConnectionStatus::AuthExpired);
            }
        }
    }
    fn submit(&self, cx: &mut Cx, job: Job) {
        let command = match &job { Job::Command { ticket, .. } => Some(ticket.request_id.clone()), _ => None };
        if self.fixture {
            if let Some(id) = command {
                self.dashboard(cx).command_result(cx, &id, CommandResult::Uncertain { message: "Offline review fixture".into() });
            }
            return;
        }
        if !self.connector.as_ref().is_some_and(|c| c.submit(job)) {
            let dashboard = self.dashboard(cx);
            if let Some(id) = command {
                dashboard.command_result(cx, &id, CommandResult::NotSubmitted { code: "not_connected".into(), retryable: true });
            } else { dashboard.set_connection(cx, ConnectionStatus::DaemonUnavailable); }
        }
    }
    fn notice(&mut self, cx: &mut Cx, text: &str) {
        self.view.label(cx, ids!(host_notice)).set_text(cx, text);
        self.view.widget(cx, ids!(host_notice)).set_visible(cx, !text.is_empty());
        self.view.redraw(cx);
    }
    fn drain(&self, cx: &mut Cx) {
        let Some(connector) = &self.connector else { return };
        let dashboard = self.dashboard(cx);
        while let Some(event) = connector.try_recv() {
            match event {
                ConnectorEvent::Owner(owner) if owner != self.owner => dashboard.set_connection(cx, ConnectionStatus::OwnerMismatch),
                ConnectorEvent::Connection(status) => dashboard.set_connection(cx, status),
                ConnectorEvent::PendingCommand(command) => dashboard.recover_command(cx, command),
                ConnectorEvent::Snapshot { snapshot, period } => { dashboard.set_snapshot_for_period(cx, snapshot, period); },
                ConnectorEvent::CommandResult { request_id, result } => dashboard.command_result(cx, &request_id, result),
                _ => {},
            }
        }
    }
}

impl Widget for OwnerDashboardHost {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if self.active || self.connector.is_some() { self.synchronize(cx); self.drain(cx); }
        if self.active && self.refresh.is_event(event).is_some() {
            let period = self.dashboard(cx).view_state(cx).period;
            let usage_filter = Some(self.dashboard(cx).view_state(cx).usage_filter());
            self.submit(cx, Job::Snapshot { period, usage_filter });
        }
        self.view.handle_event(cx, event, scope);
        if let Event::Actions(actions) = event {
            if self.active {
                for action in actions {
                    match action.downcast_ref::<EmbeddedAction>() {
                        Some(EmbeddedAction::Restore { id, view }) if view.owner_id == self.owner => {
                            let accepted = self.dashboard(cx).restore_view_state(cx, view.clone());
                            if let Some(owner) = &self.owner { cx.action(EmbeddedAction::Restored { id: *id, owner: owner.clone(), accepted }); }
                        }
                        Some(EmbeddedAction::Notice(text)) => self.notice(cx, text),
                        Some(EmbeddedAction::SetupResult { owner, message }) if self.owner.as_ref() == Some(owner) => self.notice(cx, message),
                        _ => {},
                    }
                }
            }
            for action in self.dashboard(cx).actions(actions) {
                let job = match action {
                    DashboardAction::Refresh { period } => Some(Job::Snapshot { period, usage_filter: Some(self.dashboard(cx).view_state(cx).usage_filter()) }),
                    DashboardAction::RefreshProviders => Some(Job::RefreshProviders),
                    DashboardAction::PauseHosting(ticket) => Some(Job::Command { kind: CommandKind::Pause, ticket }),
                    DashboardAction::ResumeHosting(ticket) => Some(Job::Command { kind: CommandKind::Resume, ticket }),
                    DashboardAction::StopHosting(ticket) => Some(Job::Command { kind: CommandKind::Stop, ticket }),
                    DashboardAction::CheckCommand { kind, ticket } => Some(Job::Command { kind, ticket }),
                    DashboardAction::OpenStandalone(view) => {
                        let allow = !self.fixture;
                        #[cfg(feature = "onboarding-test-support")]
                        let allow = allow || self.fixture_handover;
                        if allow { cx.action(EmbeddedAction::Offer(view)); }
                        None
                    },
                    DashboardAction::OpenSetup => { self.notice(cx, &message("opening_setup")); if !self.fixture { open_setup(); } None },
                    DashboardAction::ReviewApprovals => {
                        if !self.fixture && self.owner.is_some() && self.dashboard(cx).view_state(cx).owner_id == self.owner {
                            cx.action(crate::miniapps::MiniAppsAction::ReviewOwnerApprovals {
                                owner: self.owner.clone().unwrap(), handover_id: None,
                            });
                        }
                        None
                    },
                    _ => None,
                };
                if let Some(job) = job { self.submit(cx, job); }
            }
        }
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep { self.view.draw_walk(cx, scope, walk) }
}

impl OwnerDashboardHostRef {
    #[cfg(feature = "onboarding-test-support")]
    pub fn fixture_handover(&self, enabled: bool) {
        if let Some(mut inner) = self.borrow_mut() { inner.fixture_handover = enabled && inner.fixture; }
    }
    #[cfg(feature = "onboarding-test-support")]
    pub fn fixture_view(&self, cx: &mut Cx) -> Option<ViewState> {
        self.borrow().filter(|inner| inner.fixture).map(|inner| inner.dashboard(cx).view_state(cx))
    }
    #[cfg(feature = "onboarding-test-support")]
    pub fn load_fixture(&self, cx: &mut Cx, mut state: owner_dashboard::DashboardState) {
        if let Some(mut inner) = self.borrow_mut() {
            inner.fixture = true;
            inner.connector = None;
            inner.owner = state.owner_scope.clone();
            state.host = HostMode::Embedded;
            state.locale = locale();
            let dashboard = inner.dashboard(cx);
            // The shared fixture loader deliberately preserves host identity.
            // Initialize the real embedded host before loading synthetic data.
            dashboard.set_host(cx, HostMode::Embedded);
            dashboard.set_clock(cx, Some(owner_dashboard::fixtures::clock()));
            dashboard.load_state(cx, state);
        }
    }
    #[cfg(feature = "onboarding-test-support")]
    pub fn update_fixture(&self, cx: &mut Cx, snapshot: owner_dashboard::dto::DashboardSnapshot) {
        if let Some(inner) = self.borrow() {
            if inner.fixture {
                let dashboard = inner.dashboard(cx);
                let period = dashboard.state().and_then(|state| state.snapshot_period);
                dashboard.set_snapshot_for_period(cx, snapshot, period);
            }
        }
    }
    #[cfg(feature = "onboarding-test-support")]
    pub fn fixture_scope(&self, cx: &mut Cx, owner: Option<&str>) {
        if let Some(inner) = self.borrow() { inner.dashboard(cx).set_owner_scope(cx, owner); }
    }
    #[cfg(feature = "onboarding-test-support")]
    pub fn fixture_locale(&self, cx: &mut Cx, locale: Locale) {
        if let Some(inner) = self.borrow() {
            if inner.fixture { inner.dashboard(cx).set_locale(cx, locale); }
        }
    }
    pub fn set_host_visible(&self, cx: &mut Cx, visible: bool) {
        if let Some(mut inner) = self.borrow_mut() {
            if inner.active != visible {
                inner.active = visible;
                cx.stop_timer(inner.refresh);
                if visible { inner.refresh = cx.start_interval(2.0); }
            }
            // Keep an in-flight command's receiver until it has an outcome.
            // An idle hidden dashboard needs neither a socket nor a worker.
            if !visible && inner.dashboard(cx).state().is_none_or(|state| state.command.as_ref().is_none_or(|command| !command.in_flight())) {
                inner.connector = None;
            }
            if visible { inner.synchronize(cx); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use owner_dashboard::{fixtures, model::CommandPhase};

    fn host() -> (Cx, WidgetRef) {
        let mut cx = Cx::new(Box::new(|_, _| {}));
        let widget = cx.with_vm(|vm| {
            makepad_widgets::theme_mod(vm);
            crate::theme::init_standalone(vm);
            makepad_widgets::widgets_mod(vm);
            crate::app::register_widgets(vm);
            let value = script_eval!(vm, { mod.widgets.OwnerDashboardHost{} });
            WidgetRef::script_from_value(vm, value)
        });
        (cx, widget)
    }

    #[test]
    fn disconnected_embedded_check_keeps_the_original_uncertain_ticket() {
        let (mut cx, widget) = host();
        let host = widget.borrow::<OwnerDashboardHost>().unwrap();
        let mut state = fixtures::scenario("uncertain").unwrap();
        state.command.as_mut().unwrap().phase = CommandPhase::Checking;
        let original = state.command.as_ref().unwrap().clone();
        let dashboard = host.dashboard(&mut cx);
        dashboard.load_state(&mut cx, state);
        // Exercises the actual absent/full-queue fallback used by the host.
        host.submit(&mut cx, Job::Command { kind: original.kind, ticket: original.ticket.clone() });
        let command = dashboard.state().unwrap().command.unwrap();
        assert_eq!(command.phase, CommandPhase::Uncertain);
        assert_eq!(command.ticket, original.ticket);
    }

    #[test]
    fn hiding_an_idle_host_retires_its_connector_but_retains_uncertain_commands() {
        for uncertain in [false, true] {
            let (mut cx, widget) = host();
            {
                let mut host = widget.borrow_mut::<OwnerDashboardHost>().unwrap();
                host.dashboard(&mut cx).load_state(&mut cx, fixtures::scenario(if uncertain { "uncertain" } else { "running" }).unwrap());
                host.connector = Some(DaemonConnector::spawn(DaemonEndpoint::new(std::env::temp_dir().join("unused-owner-host-test")), OwnerScope::Fixed(fixtures::OWNER.into()), || {}));
            }
            OwnerDashboardHostRef(widget.clone()).set_host_visible(&mut cx, false);
            assert_eq!(widget.borrow::<OwnerDashboardHost>().unwrap().connector.is_some(), uncertain);
        }
    }
}
