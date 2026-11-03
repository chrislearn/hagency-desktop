//! Deterministic fixture mode for the headless harness (`fixtures` feature).
//!
//! No daemon is contacted. The production `OwnerDashboard` renders synthetic
//! `example.test` states at fixed time, and the Makepad remote bridge
//! (`--remote`) drives it: `/click` on stable widget IDs, `/snap` for bounds,
//! `/g` for PNGs, `/log` for the `od-action` instrumentation, and
//! `/event?data=COMMAND` for these commands:
//!
//! | Command | Effect |
//! | --- | --- |
//! | `scenario:NAME` | load a fixture scenario (see `owner_dashboard::fixtures::SCENARIOS`) |
//! | `locale:en` / `locale:zh` | switch language in place |
//! | `appearance:light` / `appearance:dark` | reinstall the stylesheet and reapply |
//! | `text-scale:F` | text enlargement (1.0–3.0), reapplied in place |
//! | `host:standalone` / `host:embedded` | which cross-host action is offered |
//! | `refresh-step:N` | apply the Nth refresh snapshot (stability check) |
//! | `result:ID:accepted\|completed\|unchanged\|uncertain\|rejected` | report a command outcome |
//! | `connection:failed\|absent\|connected\|incompatible\|auth` | report connection status |
//! | `dump` | log `od-state` JSON: view state, command phase, revision, sync count |
use crate::{App, Options, options};
use makepad_widgets::*;
use owner_dashboard::dto::HostingState;
use owner_dashboard::fixtures;
use owner_dashboard::{CommandResult, ConnectionStatus, DashboardAction, HostMode, Locale};
use serde_json::json;

pub fn parse_option(o: &mut Options, key: &str, value: &str) {
    match key {
        "--fixture" => o.fixture = Some(value.to_owned()),
        "--locale" => o.locale = Some(Locale::from_tag(value)),
        "--appearance" => o.dark = Some(value == "dark"),
        "--text-scale" => o.text_scale = value.parse().ok(),
        "--host" => o.host = Some(if value == "embedded" { HostMode::Embedded } else { HostMode::Standalone }),
        "--reduced-motion" => o.reduced_motion = Some(true),
        "--size" => {
            o.size = value.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
        }
        _ => {}
    }
}

pub fn active() -> bool {
    options().fixture.is_some()
}

/// Loads the initial scenario. Returns true when fixture mode is active.
pub fn start(app: &mut App, cx: &mut Cx) -> bool {
    let Some(name) = options().fixture.clone() else { return false };
    let dashboard = app.dashboard(cx);
    dashboard.set_clock(cx, Some(fixtures::clock()));
    load(app, cx, &name);
    if let Some(home) = std::env::var_os("HAGENCY_OWNER_HANDOVER_FIXTURE_HOME") {
        match owner_dashboard::handover_bridge::Bridge::spawn_fixture(home.into(), owner_dashboard::handover::Host::Standalone, SignalToUI::set_ui_signal) {
            Ok(bridge) => { bridge.set_owner(app.owner.clone()); app.bridge = Some(bridge); log!("od-fixture real handover transport enabled; synthetic owner, no daemon"); },
            Err(error) => log!("od-fixture handover unavailable: {error}"),
        }
    }
    if options().setup { app.show_setup(cx); }
    true
}

fn load(app: &mut App, cx: &mut Cx, name: &str) {
    let dashboard = app.dashboard(cx);
    match fixtures::scenario(name) {
        Some(state) => {
            app.owner = state.owner_scope.clone();
            dashboard.load_state(cx, state);
            log!("od-fixture loaded {name}");
        }
        None => log!("od-fixture unknown {name}"),
    }
}

/// In fixture mode actions are only recorded; outcomes come from `/event`.
pub fn perform(app: &mut App, cx: &mut Cx, action: DashboardAction) {
    if let DashboardAction::OpenInHagency(view) = &action {
        if std::env::var_os("HAGENCY_OWNER_HANDOVER_FIXTURE_HOME").is_some() && app.bridge.is_some() {
            // Same production source behavior: keep this process until the
            // receiving embedded UI acknowledges, then request normal quit.
            app.offer_handover(cx, view.clone(), true);
        }
        return;
    }
    if matches!(action, DashboardAction::OpenSetup) { app.show_setup(cx); return; }
    if let DashboardAction::Refresh { period } = action {
        // Re-deliver the current snapshot so "refreshing" resolves visibly.
        let dashboard = app.dashboard(cx);
        if let Some(snapshot) = dashboard.state().and_then(|s| s.snapshot) {
            dashboard.set_snapshot_for_period(cx, snapshot, period);
        }
    }
}

pub fn command(app: &mut App, cx: &mut Cx, data: &str) {
    let dashboard = app.dashboard(cx);
    let (verb, arg) = data.split_once(':').unwrap_or((data, ""));
    match verb {
        "notice" => dashboard.set_notice(cx, (!arg.is_empty()).then(|| owner_dashboard::model::Status::new(owner_dashboard::model::Tone::Caution, owner_dashboard::model::Glyph::Info, arg))),
        "inventory" => {
            if let Some(mut snapshot) = dashboard.state().and_then(|state| state.snapshot) {
                snapshot.revision += 1;
                snapshot.inventory_available = arg != "unavailable";
                snapshot.inventory_complete = arg == "complete";
                dashboard.set_snapshot(cx, snapshot);
            }
        }
        "restore-view" => {
            if let Ok(view) = serde_json::from_str(arg) { log!("od-restored {}", dashboard.restore_view_state(cx, view)); }
        }
        "owner-scope" => dashboard.set_owner_scope(cx, (!arg.is_empty()).then_some(arg)),
        "ax" => log!("od-ax {}", owner_dashboard::accessibility::diagnostics(cx)),
        #[cfg(all(feature = "native-accessibility", target_os = "macos"))]
        "ax-native" => log!("od-ax-native {}", owner_dashboard::accessibility::native_fixture_probe(cx, arg)),
        "scenario" => load(app, cx, arg),
        "setup" => app.show_setup(cx),
        "locale" => { dashboard.set_locale(cx, Locale::from_tag(arg)); app.localize_setup(cx); },
        "appearance" => app.set_dark(cx, arg == "dark"),
        "zoom" => {
            if let (Ok(zoom), Some(window)) = (arg.parse::<f64>(), app.ui.window(cx, ids!(main_window)).window_id()) {
                let dpi = cx.windows[window].native_dpi_factor() * zoom.clamp(0.75, 2.0);
                cx.set_window_dpi_override(window, Some(dpi));
                app.ui.redraw(cx);
            }
        }
        "text-scale" => {
            if let Ok(scale) = arg.parse::<f64>() {
                owner_dashboard::theme::set_text_scale(cx, scale);
                cx.request_style_reload();
            }
        }
        "host" => dashboard.set_host(cx, if arg == "embedded" { HostMode::Embedded } else { HostMode::Standalone }),
        "refresh-step" => {
            if let Ok(step) = arg.parse::<u64>() {
                let period = dashboard.state().and_then(|state| state.snapshot_period);
                let outcome = dashboard.set_snapshot_for_period(cx, fixtures::refresh_step(step), period);
                log!("od-refresh step={step} outcome={outcome:?}");
            }
        }
        "result" => {
            let (id, outcome) = arg.rsplit_once(':').unwrap_or((arg, ""));
            let result = match outcome {
                "accepted" => CommandResult::Accepted,
                "completed" => CommandResult::Completed,
                "unchanged" => CommandResult::Unchanged,
                "uncertain" => CommandResult::Uncertain { message: "fixture".into() },
                _ => CommandResult::Rejected { code: "busy".into(), message: String::new(), retryable: true },
            };
            dashboard.command_result(cx, id, result);
        }
        "hosting" => {
            // Deliver the ordinary snapshot in a given state, next revision.
            let state = match arg {
                "paused" => HostingState::Paused,
                "draining" => HostingState::Draining,
                "stopped" => HostingState::Stopped,
                _ => HostingState::Running,
            };
            let revision = dashboard.state().and_then(|s| s.snapshot.map(|s| s.revision)).unwrap_or(0);
            let mut snapshot = fixtures::ordinary(state);
            snapshot.revision = revision + 1;
            dashboard.set_snapshot(cx, snapshot);
        }
        "connection" => dashboard.set_connection(cx, match arg {
            "absent" => ConnectionStatus::DaemonUnavailable,
            "connected" => ConnectionStatus::Connected,
            "incompatible" => ConnectionStatus::Incompatible { daemon_version: Some(2) },
            "auth" => ConnectionStatus::AuthExpired,
            _ => ConnectionStatus::Failed { message: "fixture connection failure".into() },
        }),
        "dump" => {
            let state = dashboard.state();
            let view = dashboard.view_state(cx);
            let dump = json!({
                "focusId": if app.ui.text_input(cx, ids!(setup_server)).area() == cx.key_focus() { "setup_server" } else if app.ui.text_input(cx, ids!(setup_owner)).area() == cx.key_focus() { "setup_owner" } else { "other" },
                "focus": owner_dashboard::keyboard::controls(&app.ui, cx).into_iter()
                    .filter(|w| w.area() == cx.key_focus()).map(|w| w.text()).collect::<Vec<_>>(),
                "view": view,
                "revision": state.as_ref().and_then(|s| s.snapshot.as_ref().map(|s| s.revision)),
                "connection": state.as_ref().map(|s| format!("{:?}", s.connection)),
                "command": state.as_ref().and_then(|s| s.command.as_ref().map(|c| json!({
                    "kind": format!("{:?}", c.kind), "requestId": c.ticket.request_id, "phase": format!("{:?}", c.phase),
                }))),
                "confirmStop": state.as_ref().map(|s| s.confirm_stop),
                "refreshing": state.as_ref().map(|s| s.refreshing),
                "syncCount": dashboard.sync_count(),
            });
            log!("od-state {dump}");
        }
        _ => log!("od-fixture unknown command {data}"),
    }
}
