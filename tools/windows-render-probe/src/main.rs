pub use makepad_widgets;
use makepad_widgets::*;

app_main!(App);

script_mod! {
    use mod.prelude.widgets.*

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.inner_size: vec2(520, 360)
                body +: {
                    SolidView{
                        width: Fill
                        height: Fill
                        padding: 24
                        spacing: 20
                        flow: Down
                        draw_bg +: {
                            color: #123456
                            debug_code: true
                            debug_draw: true
                        }
                        Label{text: "Windows renderer probe"}
                        Button{text: "Rendered button"}
                        SolidView{
                            width: 160
                            height: 80
                            draw_bg +: {color: #ff3344}
                        }
                    }
                }
            }
        }
    }
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
