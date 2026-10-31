//! Isolated real Makepad wizard rendering. This process never signs in.
pub use makepad_widgets;
use makepad_widgets::*;
use rinx::home::space_management::SpaceManagementPanelWidgetRefExt;
app_main!(App);
script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {main_window := Window {
            window.inner_size: vec2(720,720)
            window.title: "Hagency · Space creation review"
            body +: {wizard := SpaceManagementPanel {}}
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
            let parts = command.split(':').collect::<Vec<_>>();
            if parts.len() == 4 && parts[0] == "wizard" {
                self.ui.space_management_panel(cx, ids!(wizard)).review(
                    cx,
                    parts[1].parse().unwrap(),
                    parts[2].parse().unwrap(),
                    parts[3] == "pending",
                );
            }
        }
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
