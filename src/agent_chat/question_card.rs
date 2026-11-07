//! The timeline card for a question an Agent asks during a run.
//!
//! One card sits (hidden by default) inside every `Message` widget's content
//! column, next to the approval card. `populate_message_view` shows it under
//! the message that started the run, from [`crate::hagency::pending_questions`].
//! Clicks and typed text go straight to that store; an answer ready to send
//! leaves as a [`QuestionAnswerRequested`] action, which the app sends.
//! The card holds no protocol state of its own.

use makepad_widgets::*;

use crate::hagency::pending_questions::{self, PendingQuestionsChanged, QuestionView};

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*

    mod.widgets.AgentQuestionOptionButton = mod.widgets.AgentApprovalSecondaryButton {}

    mod.widgets.AgentQuestionBlock = View {
        visible: false
        width: Fill, height: Fit
        flow: Down
        spacing: 6.0

        header_label := Label {
            width: Fill, height: Fit
            padding: 0
            draw_text +: {
                text_style: (mod.widgets.RBX_TEXT_META)
                color: (mod.widgets.RBX_INFO_FG)
            }
            text: ""
        }
        question_label := Label {
            width: Fill, height: Fit
            flow: Flow.Right{wrap: true}
            padding: 0
            draw_text +: {
                text_style: (mod.widgets.RBX_TEXT_BODY)
                color: (mod.widgets.RBX_FG_PRIMARY)
            }
            text: ""
        }
        options := View {
            width: Fill, height: Fit
            flow: Flow.Right{wrap: true}
            spacing: 8.0
            o0 := mod.widgets.AgentQuestionOptionButton {}
            o1 := mod.widgets.AgentQuestionOptionButton {}
            o2 := mod.widgets.AgentQuestionOptionButton {}
            o3 := mod.widgets.AgentQuestionOptionButton {}
        }
        details_label := Label {
            width: Fill, height: Fit
            flow: Flow.Right{wrap: true}
            padding: 0
            draw_text +: {
                text_style: (mod.widgets.RBX_TEXT_META)
                color: (mod.widgets.RBX_FG_SECONDARY)
            }
            text: ""
        }
        // TextInput has no `visible` of its own; this box shows or hides it.
        other_box := View {
            visible: false
            width: Fill, height: Fit
            other_input := TextInput {
                width: Fill, height: Fit
                empty_text: #(crate::i18n::tr("Other: type your own answer"))
                i18n_empty_text: "Other: type your own answer"
            }
        }
        answer_label := Label {
            visible: false
            width: Fill, height: Fit
            flow: Flow.Right{wrap: true}
            padding: 0
            draw_text +: {
                text_style: (mod.widgets.RBX_TEXT_BODY_STRONG)
                color: (mod.widgets.RBX_FG_PRIMARY)
            }
            text: ""
        }
    }

    mod.widgets.AgentQuestionCard = #(AgentQuestionCard::register_widget(vm)) {
        visible: false
        width: Fill, height: Fit
        flow: Down
        margin: Inset{top: 8.0, right: 10.0}

        // The plain View background draws no border; a rounded frame does.
        frame := RoundedView {
            width: Fill, height: Fit
            flow: Down
            spacing: 10.0
            padding: Inset{left: 16.0, right: 16.0, top: 12.0, bottom: 14.0}
            draw_bg +: {
                color: (mod.widgets.RBX_INFO_BG)
                border_radius: (mod.widgets.RBX_RADIUS_SM)
                border_size: 1.0
                border_color: (mod.widgets.RBX_INFO_FG)
            }

            header := View {
                width: Fill, height: Fit
                flow: Right
                spacing: 8.0
                align: Align{y: 0.5}

                title_label := Label {
                    width: Fill, height: Fit
                    flow: Flow.Right{wrap: true}
                    padding: 0
                    draw_text +: {
                        text_style: (mod.widgets.RBX_TEXT_CARD_TITLE)
                        color: (mod.widgets.RBX_INFO_FG)
                    }
                    text: ""
                }

                status_badge := RoundedView {
                    width: Fit, height: Fit
                    padding: Inset{left: 8.0, right: 8.0, top: 4.0, bottom: 4.0}
                    show_bg: true
                    draw_bg +: {
                        color: (mod.widgets.RBX_BG_SURFACE)
                        border_radius: (mod.widgets.RBX_RADIUS_PILL)
                    }
                    status_label := Label {
                        width: Fit, height: Fit
                        padding: 0
                        draw_text +: {
                            text_style: (mod.widgets.RBX_TEXT_BADGE)
                            color: (mod.widgets.RBX_INFO_FG)
                        }
                        text: ""
                    }
                }
            }

            q0 := mod.widgets.AgentQuestionBlock {}
            q1 := mod.widgets.AgentQuestionBlock {}
            q2 := mod.widgets.AgentQuestionBlock {}

            footer := View {
                width: Fill, height: Fit
                flow: Flow.Right{wrap: true}
                spacing: 8.0
                align: Align{y: 0.5}
                send_button := mod.widgets.AgentApprovalPrimaryButton {
                    text: #(crate::i18n::tr("Send answer"))
                    i18n_text: "Send answer"
                }
            }

            hint_label := Label {
                width: Fill, height: Fit
                flow: Flow.Right{wrap: true}
                padding: 0
                draw_text +: {
                    text_style: (mod.widgets.RBX_TEXT_META)
                    color: (mod.widgets.RBX_FG_SECONDARY)
                }
                text: ""
            }
        }
    }
}

/// Question blocks, in card order. A card holds at most three questions.
const BLOCKS: [&[LiveId]; 3] = [ids!(q0), ids!(q1), ids!(q2)];
/// Option buttons per block. A card offers at most four options a question.
const OPTIONS: [[&[LiveId]; 4]; 3] = [
    [ids!(q0.options.o0), ids!(q0.options.o1), ids!(q0.options.o2), ids!(q0.options.o3)],
    [ids!(q1.options.o0), ids!(q1.options.o1), ids!(q1.options.o2), ids!(q1.options.o3)],
    [ids!(q2.options.o0), ids!(q2.options.o1), ids!(q2.options.o2), ids!(q2.options.o3)],
];
const INPUTS: [&[LiveId]; 3] = [ids!(q0.other_box.other_input), ids!(q1.other_box.other_input), ids!(q2.other_box.other_input)];

/// A card for one Agent question, with its option buttons and "Other" input.
#[derive(Script, ScriptHook, Widget)]
pub struct AgentQuestionCard {
    #[deref] view: View,
    /// The card shown, so a click maps to the store's card.
    #[rust] card_id: Option<String>,
    /// Each block's question id and option labels, as drawn.
    #[rust] rows: Vec<(String, Vec<String>)>,
}

impl Widget for AgentQuestionCard {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
        let Event::Actions(actions) = event else { return };
        let Some(card_id) = self.card_id.clone() else { return };
        let mut changed = false;
        for (index, (question, labels)) in self.rows.clone().iter().enumerate().take(BLOCKS.len()) {
            for (slot, label) in labels.iter().enumerate().take(OPTIONS[index].len()) {
                if self.view.button(cx, OPTIONS[index][slot]).clicked(actions) {
                    if let Some(request) = pending_questions::choose(&card_id, question, label) {
                        cx.action(request);
                    }
                    changed = true;
                }
            }
            let input = self.view.text_input(cx, INPUTS[index]);
            if let Some(text) = input.changed(actions) {
                pending_questions::type_text(&card_id, question, &text);
                self.refresh_send(cx, &card_id);
            }
            if input.returned(actions).is_some()
                && let Some(request) = pending_questions::submit(&card_id)
            {
                cx.action(request);
                changed = true;
            }
        }
        if self.view.button(cx, ids!(footer.send_button)).clicked(actions) {
            if let Some(request) = pending_questions::submit(&card_id) {
                cx.action(request);
            }
            changed = true;
        }
        if changed {
            cx.action(PendingQuestionsChanged);
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}

impl AgentQuestionCard {
    /// Shows the card for `view`, or hides it entirely when `view` is `None`.
    pub fn set_view(&mut self, cx: &mut Cx, view: Option<&QuestionView>) {
        let Some(v) = view else {
            self.card_id = None;
            self.rows.clear();
            self.view.set_visible(cx, false);
            return;
        };
        // A recycled timeline item may have shown another card's draft.
        let rebinding = self.card_id.as_deref() != Some(v.card_id.as_str());
        self.card_id = Some(v.card_id.clone());
        self.view.set_visible(cx, true);
        self.view.label(cx, ids!(header.title_label)).set_text(cx, &v.title);
        self.view.label(cx, ids!(header.status_badge.status_label)).set_text(cx, &v.status);
        for (index, block_path) in BLOCKS.iter().enumerate() {
            let block = self.view.view(cx, block_path);
            let Some(row) = v.questions.get(index) else {
                block.set_visible(cx, false);
                continue;
            };
            block.set_visible(cx, true);
            let header = block.label(cx, ids!(header_label));
            header.set_visible(cx, !row.header.is_empty());
            header.set_text(cx, &row.header);
            block.label(cx, ids!(question_label)).set_text(cx, &row.question);
            for (slot, path) in OPTIONS[index].iter().enumerate() {
                let button = self.view.button(cx, path);
                let Some((label, chosen)) = row.options.get(slot) else {
                    button.set_visible(cx, false);
                    continue;
                };
                button.set_visible(cx, row.answer.is_none());
                button.set_text(cx, &if *chosen { format!("✓ {label}") } else { label.clone() });
                show_enabled(cx, &button, v.open);
                if v.open {
                    // A recycled portal-list item may carry stale hover state.
                    button.reset_hover(cx);
                }
            }
            block.view(cx, ids!(options)).set_visible(cx, !row.options.is_empty() && row.answer.is_none());
            let details = block.label(cx, ids!(details_label));
            details.set_visible(cx, v.open && !row.details.is_empty());
            details.set_text(cx, &row.details);
            block.view(cx, ids!(other_box)).set_visible(cx, row.text.is_some());
            let input = self.view.text_input(cx, INPUTS[index]);
            if let Some(draft) = &row.text
                && (rebinding || input.text() != *draft)
            {
                input.set_text(cx, draft);
            }
            let answer = block.label(cx, ids!(answer_label));
            answer.set_visible(cx, row.answer.is_some());
            if let Some(text) = &row.answer {
                answer.set_text(cx, &format!("→ {text}"));
            }
        }
        let send = self.view.button(cx, ids!(footer.send_button));
        send.set_visible(cx, v.send_visible);
        show_enabled(cx, &send, v.can_send);
        self.view.view(cx, ids!(footer)).set_visible(cx, v.send_visible);
        let hint = self.view.label(cx, ids!(hint_label));
        hint.set_visible(cx, !v.hint.is_empty());
        hint.set_text(cx, &v.hint);
        self.rows = v
            .questions
            .iter()
            .map(|row| (row.id.clone(), row.options.iter().map(|(label, _)| label.clone()).collect()))
            .collect();
        self.view.redraw(cx);
    }

    fn refresh_send(&mut self, cx: &mut Cx, card_id: &str) {
        let ready = pending_questions::get(card_id).is_some_and(|c| c.ready());
        show_enabled(cx, &self.view.button(cx, ids!(footer.send_button)), ready);
    }
}

/// `set_enabled` alone only stops clicks; the face and label also dim so a
/// button that cannot act yet looks it.
fn show_enabled(cx: &mut Cx, button: &ButtonRef, enabled: bool) {
    button.set_enabled(cx, enabled);
    let dim = if enabled { 0.0 } else { 1.0 };
    let mut button = button.clone();
    script_apply_eval!(cx, button, {
        draw_bg +: {disabled: #(dim)}
        draw_text +: {disabled: #(dim)}
    });
}

impl AgentQuestionCardRef {
    /// See [`AgentQuestionCard::set_view`].
    pub fn set_view(&self, cx: &mut Cx, view: Option<&QuestionView>) {
        if let Some(mut inner) = self.borrow_mut() {
            inner.set_view(cx, view);
        }
    }
}
