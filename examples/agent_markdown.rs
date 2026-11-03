//! Offline native chat Markdown fixture; no accounts or network activity.
use makepad_widgets::*;
use rinx::shared::html_or_plaintext::HtmlOrPlaintextWidgetRefExt;
use matrix_sdk::ruma::{events::room::message::FormattedBody, user_id};
app_main!(App);
script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    startup() do #(App::script_component(vm)) {
        ui: Root {
            main_window := Window {
                window.inner_size: vec2(820, 840)
                window.title: "Hagency · Chinese Markdown"
                body +: {
                    content := ScrollYView {
                        flow: Down padding: 24 spacing: 12
                        title := Label {text: "小黄瓜"}
                        answer := HtmlOrPlaintext {selectable: true}
                    }
                }
            }
        }
    }
}
#[derive(Script, ScriptHook)]
struct App { #[live] ui: WidgetRef }
impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        let source = std::env::var_os("HAGENCY_MARKDOWN_FIXTURE").map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_else(||
            "## 最新判断\n\n**时间点很重要：**截至美国得州中部时间 **2026 年 11 月 3 日下午约 2 点**，投票仍在进行。\n\n- **民主党：**得州州众议员\n- **共和党：**得州总检察长\n\n| 调查 | 结果 |\n|---|---|\n| 民调 | **领先：**2.6% |\n\n[来源](https://example.org)\n\n代码原样保留：`**时间点：**截至`".into());
        let rich = FormattedBody::markdown(&source);
        let mut agent = rinx::agent_chat::presentation::AgentMessagePresentation::for_sender(user_id!("@_hagency_agt_chinese:test")).unwrap();
        let (_, formatted) = agent.present(&source, rich.as_ref());
        self.ui.html_or_plaintext(cx, ids!(answer)).show_html(cx, &formatted.unwrap().body);
    }
}
impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        script_eval!(vm, {mod.theme = mod.themes.light});
        #[cfg(any(target_os="macos", target_os="ios"))]
        article_makepad::apple_fonts::install(vm);
        makepad_widgets::widgets_mod(vm);
        rinx::i18n::install(vm);
        makepad_code_editor::script_mod(vm);
        rinx::shared::script_mod(vm);
        self::script_mod(vm)
    }
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
