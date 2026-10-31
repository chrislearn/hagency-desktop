//! Offline native invitation interaction fixture; never signs in or sends requests.
pub use makepad_widgets;
use makepad_widgets::*;
use rinx::home::mobile_chat_info::MobileChatInfoWidgetRefExt;
use rinx::home::invite_modal::{InviteModalWidgetRefExt, InviteModalAction};
app_main!(App);
fn review_size() -> Vec2d {
    Vec2d {
        x: std::env::var("WIZARD_REVIEW_WIDTH").ok().and_then(|s|s.parse().ok()).unwrap_or(720.0),
        y: std::env::var("WIZARD_REVIEW_HEIGHT").ok().and_then(|s|s.parse().ok()).unwrap_or(780.0),
    }
}
script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {main_window := Window {
            window.inner_size: #(review_size())
            window.title: "Hagency · Member invitation review"
            body +: {
                flow: Overlay
                picker := MobileChatInfo {}
                invite_modal := Modal {content := InviteModal {}}
            }
        }}
    }
}
#[derive(Script, ScriptHook)]
struct App {
    #[live]
    ui: WidgetRef,
}
impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        rinx::theme::init_standalone(vm);
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        article_makepad::apple_fonts::install(vm);
        makepad_widgets::widgets_mod(vm);
        desktop_style::apply_widgets(vm);
        rinx::app::register_widgets(vm);
        self::script_mod(vm)
    }
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if let Event::Custom(command) = event {
            if matches!(command.as_str(), "theme:dark" | "theme:light") {
                rinx::theme::select(cx, rinx::theme::Selection {
                    appearance: if command == "theme:dark" {rinx::theme::Appearance::Dark} else {rinx::theme::Appearance::Light},
                    accent: rinx::theme::Accent::Teal,
                }).unwrap();
            }
            if command == "theme:large-text" {
                let mut package = rinx::theme::packages::export_current(cx).unwrap();
                for dark in [false,true] {package.set(Some(dark), "typography.scale", octosense_theme_contract::number(1.3));}
                let mut preferences = rinx::theme::packages::current(cx).unwrap(); preferences.package = Some(package);
                rinx::theme::packages::apply(cx, preferences).unwrap();
            }
            if let Some(scenario) = command.strip_prefix("picker:") {
                self.ui.mobile_chat_info(cx, ids!(picker)).review_agent_picker(cx, scenario);
            }
        }
        if let Event::Custom(command)=event {
            if let Some(scenario)=command.strip_prefix("invite:") {
                self.ui.invite_modal(cx,ids!(invite_modal.content)).review_result(cx,scenario);
            }
        }
        if let Event::Actions(actions)=event {
            for action in actions {
                match action.downcast_ref() {
                    Some(InviteModalAction::Open(room)) => {
                        self.ui.invite_modal(cx,ids!(invite_modal.content)).show(cx,room.clone());
                        self.ui.invite_modal(cx,ids!(invite_modal.content)).review_result(cx,"ready");
                        self.ui.modal(cx,ids!(invite_modal)).open(cx);
                    }
                    Some(InviteModalAction::Close) => self.ui.modal(cx,ids!(invite_modal)).close(cx),
                    _ => {}
                }
            }
        }
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
