pub use makepad_widgets;
use makepad_widgets::*;
use makepad_widgets::makepad_platform::event::NativeSystemBrowserNavigation;

const BROWSER: LiveId = live_id!(icons_webview_probe);

app_main!(App);

script_mod! {
    use mod.prelude.widgets.*

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.inner_size: vec2(520, 360)
                body +: {
                    flow: Right
                    SolidView {
                        width: 76 height: Fill flow: Down spacing: 32 padding: 24
                        draw_bg.color: #x1b2e3e
                        Icon {icon_walk: Walk{width: 24 height: 24} draw_icon +: {color: #xffffffff svg: crate_resource("self://resources/icons/home.svg")}}
                        Icon {icon_walk: Walk{width: 24 height: 24} draw_icon +: {color: #xffffffff svg: crate_resource("self://resources/icons/people.svg")}}
                        Icon {icon_walk: Walk{width: 24 height: 24} draw_icon +: {color: #xffffffff svg: crate_resource("self://resources/icons/globe.svg")}}
                    }
                    surface := SolidView {width: Fill height: Fill draw_bg.color: #xffeeee}
                }
            }
        }
    }
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust]
    opened: bool,
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if matches!(event, Event::Custom(command) if command == "open") {
            self.opened = true;
            cx.system_browser(BROWSER).spawn_navigable("data:text/html,%3Ctitle%3Eicons-wv-ready%3C/title%3E%3Cp%3EWebView%3C/p%3E");
            cx.redraw_all();
        }
        if let Event::Actions(actions) = event {
            for action in actions {
                if let Some(nav) = action.downcast_ref::<NativeSystemBrowserNavigation>() {
                    if nav.browser_id == BROWSER.0 && nav.title == "icons-wv-ready" {
                        println!("ICONS_WEBVIEW_READY");
                    }
                }
            }
        }
        self.ui.handle_event(cx, event, &mut Scope::empty());
        if self.opened && matches!(event, Event::Draw(_)) {
            let area = self.ui.view(cx, ids!(surface)).area();
            cx.system_browser(BROWSER).update(area, true);
        }
    }
}
