//! Offline native @mention picker, composer and message rendering. Parses
//! messages for assertions but never starts Matrix or sends a message.
pub use makepad_widgets;
use makepad_widgets::*;
use rinx::shared::{
    mention_popup::{MentionItem, set_global_mention_popup},
    mentionable_text_input::{MentionableTextInputWidgetRefExt, MentionableTextInputState},
    html_or_plaintext::HtmlOrPlaintextWidgetRefExt,
    slash_commands::SlashCommandOutcome,
};

app_main!(App);
fn review_size() -> Vec2d {
    dvec2(std::env::var("WIZARD_REVIEW_WIDTH").unwrap().parse().unwrap(),
          std::env::var("WIZARD_REVIEW_HEIGHT").unwrap().parse().unwrap())
}
script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {main_window := Window {
            window.inner_size: #(review_size())
            window.title: "Hagency · Compact mention review"
            body +: {
                flow: Overlay
                content := SolidView {
                    width: Fill height: Fill flow: Down padding: 20 spacing: 16
                    draw_bg.color: mod.widgets.RINX_PAGE
                    Label {text: "@ Mention" draw_text.color: mod.widgets.RINX_INK}
                    preview := HtmlOrPlaintext {width: Fill height: Fit}
                    View {height: Fill width: Fill}
                    composer := RoomInputBar {width: Fill}
                }
                mention_popup := MentionablePopup {}
            }
        }}
    }
}
#[derive(Script, ScriptHook)]
struct App {
    #[live] ui: WidgetRef,
    #[rust] saved: Option<MentionableTextInputState>,
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
                for dark in [false, true] { package.set(Some(dark), "typography.scale", octosense_theme_contract::number(1.3)); }
                let mut preferences = rinx::theme::packages::current(cx).unwrap();
                preferences.package = Some(package);
                rinx::theme::packages::apply(cx, preferences).unwrap();
            }
            let input = self.ui.widget(cx, ids!(composer)).mentionable_text_input(cx, ids!(input_bar.mentionable_text_input));
            if command == "mentions:ready" {
                set_global_mention_popup(cx, &self.ui);
                input.review_members([
                    ("@_hagency_agent_yellow:example.invalid", "小黄"),
                    ("@first_alex:example.invalid", "Alex"),
                    ("@second_alex:example.invalid", "Alex"),
                    ("@chris:example.invalid", "Chris"),
                ].into_iter().map(|(id, name)| MentionItem::User {
                    user_id: id.try_into().unwrap(), display_name: name.into(), avatar_url: None,
                }).collect());
                input.set_key_focus(cx);
            }
            if command == "mentions:save" { self.saved = Some(input.save_state()); }
            if command == "mentions:clear" { input.set_text(cx, ""); }
            if command == "mentions:restore" { input.restore_state(cx, self.saved.clone().unwrap()); }
            if command == "mentions:roundtrip" { input.set_text(cx, &input.markdown_text()); }
            if command == "mentions:inspect" {
                let visible = input.text();
                let parsed = match input.parse_input(visible.trim()) {
                    SlashCommandOutcome::Message(message) => serde_json::to_value(message).unwrap(),
                    other => serde_json::json!({"other": format!("{other:?}")}),
                };
                let preview = self.ui.html_or_plaintext(cx, ids!(preview));
                if let Some(html) = parsed.get("formatted_body").and_then(|v| v.as_str()) {
                    preview.show_html(cx, html);
                } else { preview.show_plaintext(cx, parsed.get("body").and_then(|v| v.as_str()).unwrap_or("")); }
                let report = serde_json::json!({"visible": visible, "markdown": input.markdown_text(), "message": parsed,
                    "focused": cx.has_key_focus(input.text_input_ref().area())});
                let path = std::path::PathBuf::from(std::env::var("HAGENCY_DESKTOP_DATA_DIR").unwrap()).join("mention-review.json");
                std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            }
        }
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
