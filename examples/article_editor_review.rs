//! Real ArticlePanel input/rendering with an isolated offline account.
//! Run with --features onboarding-test-support and an explicit marked profile.
pub use makepad_widgets;
use makepad_widgets::*;
use rinx::article_app::{ArticleAction, ArticlePanelWidgetRefExt};
use rinx::theme::{self, Accent, Appearance, Selection};
app_main!(App);
fn review_size() -> Vec2d {
    let width = std::env::var("ARTICLE_REVIEW_WIDTH").ok().and_then(|s| s.parse().ok()).unwrap_or(1120.0);
    let height = std::env::var("ARTICLE_REVIEW_HEIGHT").ok().and_then(|s| s.parse().ok()).unwrap_or(820.0);
    Vec2d {x: width, y: height}
}
script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {
            main_window := Window {
                window.inner_size: #(review_size())
                window.title: "Hagency · Article editor · Offline review"
                body +: {editor := ArticlePanel {padding: 0}}
            }
        }
    }
}
#[derive(Script, ScriptHook)]
struct App { #[live] ui: WidgetRef }
impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        rinx::article_app::review::install_offline_account().unwrap();
        makepad_widgets::theme_mod(vm); theme::init_standalone(vm);
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        article_makepad::apple_fonts::install(vm);
        makepad_widgets::widgets_mod(vm); desktop_style::apply_widgets(vm);
        rinx::app::register_widgets(vm);
        // Production HomeScreen owns this list. The offline host has no rooms.
        let _ = vm.cx_mut().global::<rinx::home::rooms_list::RoomsListRef>();
        self::script_mod(vm)
    }
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if let Event::Custom(command) = event {
            match command.as_str() {
                "article:open" => self.ui.article_panel(cx, ids!(editor)).action(cx, ModalRef::default(), &ArticleAction::Open),
                "article:dark" | "article:light" => {
                    theme::select(cx, Selection {appearance: if command == "article:dark" {Appearance::Dark} else {Appearance::Light}, accent: Accent::Teal}).unwrap();
                }
                "article:large-text" => {
                    let mut package = theme::packages::export_current(cx).unwrap();
                    for dark in [false, true] {package.set(Some(dark), "typography.scale", octosense_theme_contract::number(1.3));}
                    let mut preferences = theme::packages::current(cx).unwrap(); preferences.package = Some(package);
                    theme::packages::apply(cx, preferences).unwrap();
                }
                _ => {}
            }
        }
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
