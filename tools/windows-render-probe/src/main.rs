pub use makepad_widgets;
use makepad_widgets::*;

#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn ShowCursor(show: i32) -> i32;
}

app_main!(App);

script_mod! {
    use mod.prelude.widgets.*

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.inner_size: vec2(520, 360)
                body +: {
                    split := Splitter{
                        width: Fill
                        height: Fill
                        axis: SplitterAxis.Horizontal
                        align: SplitterAlign.FromA(260.0)
                        size: 4.0
                        hit_margin: 6.0
                        a: SolidView{width: Fill height: Fill draw_bg.color: #x123456}
                        b: SolidView{width: Fill height: Fill draw_bg.color: #xff3344}
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
        if let Event::Custom(command) = event {
            match command.as_str() {
                "hide-cursor" => cx.set_cursor(MouseCursor::Hidden),
                "show-cursor" => cx.set_cursor(MouseCursor::Default),
                #[cfg(target_os = "windows")]
                "cursor-count" => unsafe {
                    // Probe this UI thread's counter without changing it.
                    ShowCursor(1);
                    let count = ShowCursor(0);
                    println!("CURSOR_DISPLAY_COUNT={count}");
                },
                _ => {}
            }
        }
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
