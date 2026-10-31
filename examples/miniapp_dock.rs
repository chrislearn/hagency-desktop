//! Offline visual exercise for native readers/editors in the production mini-app dock.
//! Uses an isolated profile; no Matrix client, executor or publication is started.
pub use makepad_widgets;
use makepad_widgets::*;
use rinx::miniapps::{MiniAppsAction, side::{MiniAppsSideAction, MiniAppsSideRegionWidgetRefExt}, window::{MiniAppsWindowAction, MiniAppsWindowHostWidgetRefExt}};
use rinx::shared::web_browser::WebBrowserAction;
use rinx::article_app::ArticleAction;
use rinx::home::sidebar_action_menu::{SidebarActionMenuWidgetRefExt, SidebarMenuKind, new_or_join_menu_pos};
use rinx::shared::navigation_bar_button::NavigationBarButtonWidgetRefExt;
use rinx::theme::{self, Accent, Appearance, Selection};
app_main!(App);
script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {
            popped := MiniAppsWindowHost {}
            main_window := Window {
                window.inner_size: vec2(1200, 850)
                window.title: "Hagency · Dock review · Offline"
                body +: {flow: Overlay
                    main := View {width: Fill height: Fill flow: Down padding: 16 spacing: 12 show_bg: true draw_bg.color: RINX_PAGE
                    View {width: Fill height: Fit flow: Right spacing: 12
                        apps := RinxPrimaryButton {text: "Mini apps"}
                        web := RinxPrimaryButton {text: "Web page"}
                        read := RinxPrimaryButton {text: "Read article"}
                        edit := RinxPrimaryButton {text: "Article editor"}
                        plus := NewOrJoinButton {}
                    }
                    View {width: Fill height: Fill flow: Right
                        View {width: Fill height: Fill flow: Down spacing: 16
                            RinxPageTitle {text: "Conversation"}
                            RinxLabel {text: "The conversation remains open beside native tabs."}
                            draft := RinxInput {text: "Unsent chat draft"}
                        }
                        dock := MiniAppsSideRegion {}
                    }
                    }
                    menu := SidebarActionMenu {}
                }
            }
        }
    }
}
#[derive(Script, ScriptHook)]
struct App { #[live] ui: WidgetRef }
impl MatchEvent for App {
    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        let dock = self.ui.mini_apps_side_region(cx, ids!(dock));
        let host = self.ui.mini_apps_window_host(cx, ids!(popped));
        let plus = self.ui.navigation_bar_button(cx, ids!(plus));
        if plus.clicked(actions) {
            let position = new_or_join_menu_pos(cx, plus.area().rect(cx));
            self.ui.sidebar_action_menu(cx, ids!(menu)).show(cx, SidebarMenuKind::Add, position,
                Rect {pos: dvec2(0.0, 0.0), size: dvec2(1200.0, 850.0)});
        }
        if self.ui.button(cx, ids!(apps)).clicked(actions) {dock.open(cx, &MiniAppsAction::Open);}
        if self.ui.button(cx, ids!(web)).clicked(actions) {
            let url = std::env::var("HAGENCY_DOCK_URL").unwrap_or_else(|_| "https://example.com/".into());
            dock.web_action(cx, &WebBrowserAction::Open(url.parse().unwrap()));
        }
        if self.ui.button(cx, ids!(read)).clicked(actions) {
            dock.web_action(cx, &WebBrowserAction::OpenMarkdown {title: "Docked article".into(), source: "# Docked article\n\nReading keeps the conversation and editor draft open.\n\n## Native Markdown\n\n- Switch tabs\n- Pop out\n- Dock back\n".into(), account: None, attachment: None});
        }
        if self.ui.button(cx, ids!(edit)).clicked(actions) {dock.article_action(cx, &ArticleAction::Open);}
        for action in actions {
            if let Some(action) = action.downcast_ref::<ArticleAction>() {dock.article_action(cx, action);}
            if matches!(action.downcast_ref(), Some(MiniAppsSideAction::PopOut)) {
                if let Some(surface) = dock.take_front(cx) {
                    if let Some(previous) = host.adopt(cx, surface) {dock.dock_surface(cx, previous);}
                }
            }
            if let Some(MiniAppsWindowAction::DockBack(id)) = action.downcast_ref() {
                if let Some(surface) = host.take_for_dock(cx, *id) {dock.dock_surface(cx, surface);}
            }
        }
        dock.prepare(cx, 1168.0, 0.0, true, Some(760.0));
        self.ui.redraw(cx);
    }
}
impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        #[cfg(feature = "onboarding-test-support")]
        if std::env::var("HAGENCY_DOCK_OFFLINE_ACCOUNT").as_deref() == Ok("1") {
            rinx::article_app::review::install_offline_account().unwrap();
        }
        makepad_widgets::theme_mod(vm); theme::init_standalone(vm);
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        article_makepad::apple_fonts::install(vm);
        makepad_widgets::widgets_mod(vm); desktop_style::apply_widgets(vm);
        rinx::app::register_widgets(vm);
        let _ = vm.cx_mut().global::<rinx::home::rooms_list::RoomsListRef>();
        self::script_mod(vm)
    }
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if let Event::Custom(command) = event {
            for (name, appearance) in [("dock:dark", Appearance::Dark), ("dock:light", Appearance::Light)] {
                if command == name {theme::select(cx, Selection {appearance, accent: Accent::Teal}).unwrap();}
            }
        }
        self.match_event(cx, event);
        // Production HomeScreen prepares the region on each frame, including restyles.
        self.ui.mini_apps_side_region(cx, ids!(dock)).prepare(cx, 1168.0, 0.0, true, Some(760.0));
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
