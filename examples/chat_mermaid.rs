//! Offline fixture for the production Agent reply renderer and native copy path.
use makepad_widgets::*;
use matrix_sdk::ruma::{events::room::message::FormattedBody, user_id};
use rinx::shared::html_or_plaintext::HtmlOrPlaintextWidgetRefExt;
app_main!(App);

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {
            main_window := Window {
                window.inner_size: vec2(820, 840)
                window.title: "Hagency · Chat Mermaid fixture"
                body +: {
                    flow: Down padding: 24 spacing: 12
                    content := ScrollYView {
                        width: Fill height: Fill flow: Down
                        answer := HtmlOrPlaintext {selectable: true}
                    }
                    View {height: Fit spacing: 8
                        valid := Button {text: "Original"}
                        edit := Button {text: "Edit"}
                        invalid := Button {text: "Invalid"}
                        narrow := Button {text: "Narrow"}
                        restyle := Button {text: "Restyle"}
                    }
                    inspect := Button {text: "Inspect native copy" grab_key_focus: false}
                    result := Label {width: Fill height: 40 text: "Ready"}
                }
            }
        }
    }
}

const SOURCE: &str = include_str!("../tests/fixtures/grapheneos.mmd");

#[derive(Script, ScriptHook)]
struct App {
    #[live]
    ui: WidgetRef,
    #[rust]
    original_html: String,
}

impl App {
    fn show(&mut self, cx: &mut Cx, diagram: &str) {
        let source = format!("Before 中文 **bold**.\n\n```mermaid\n{diagram}\n```\n\nAfter [link](https://example.org).\n\n```rust\nfn main() {{}}\n```");
        let rich = FormattedBody::markdown(&source);
        let mut agent = rinx::agent_chat::presentation::AgentMessagePresentation::for_sender(
            user_id!("@_hagency_agt_mermaid:test"),
        )
        .unwrap();
        let (_, formatted) = agent.present(&source, rich.as_ref());
        self.original_html = formatted.unwrap().into_owned().body;
        self.ui
            .html_or_plaintext(cx, ids!(answer))
            .show_html(cx, &self.original_html);
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.show(cx, SOURCE.trim_end());
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        if self.ui.button(cx, ids!(valid)).clicked(actions) {
            self.show(cx, SOURCE.trim_end());
        }
        if self.ui.button(cx, ids!(edit)).clicked(actions) {
            self.show(cx, &SOURCE.trim_end().replace("GrapheneOS", "EditedOS"));
        }
        if self.ui.button(cx, ids!(invalid)).clicked(actions) {
            self.show(cx, "not a diagram\n中文 remains visible");
        }
        if self.ui.button(cx, ids!(narrow)).clicked(actions) {
            let mut content = self.ui.view(cx, ids!(content));
            script_apply_eval!(cx, content, {width: 300});
            content.redraw(cx);
        }
        if self.ui.button(cx, ids!(restyle)).clicked(actions) {
            let mut html = self.ui.html(cx, ids!(answer.html_view.html));
            script_apply_eval!(cx, html, {font_size: 16});
            html.redraw(cx);
        }
        if self.ui.button(cx, ids!(inspect)).clicked(actions) {
            let response = std::rc::Rc::new(std::cell::RefCell::new(None));
            self.ui.handle_event(
                cx,
                &Event::TextCopy(TextClipboardEvent {
                    response: response.clone(),
                }),
                &mut Scope::empty(),
            );
            let html = self.ui.html(cx, ids!(answer.html_view.html));
            let html = html.borrow().unwrap();
            let diagrams = html
                .doc
                .nodes
                .iter()
                .filter(|node| {
                    matches!(node,
                        makepad_html::HtmlNode::OpenTag {lc, ..} if *lc == live_id!(chatdiagram)
                    )
                })
                .count();
            self.ui.label(cx, ids!(result)).set_text(
                cx,
                &serde_json::json!({
                    "copy": *response.borrow(), "text": html.get_full_text(),
                    "original_html_preserved": html.body.as_ref() == self.original_html,
                    "diagrams": diagrams,
                })
                .to_string(),
            );
        }
    }
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        script_eval!(vm, {mod.theme = mod.themes.light});
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        article_makepad::apple_fonts::install(vm);
        makepad_widgets::widgets_mod(vm);
        rinx::i18n::install(vm);
        makepad_code_editor::script_mod(vm);
        rinx::shared::script_mod(vm);
        // The full app registers article widgets after chat widgets, too.
        article_makepad::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
