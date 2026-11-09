//! A notice that slides into view when someone is typing.

use makepad_widgets::*;

use crate::shared::bouncing_dots::BouncingDotsWidgetExt;

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*


    mod.widgets.TYPING_NOTICE_ANIMATION_DURATION_SECS = 0.3

    mod.widgets.TypingNotice = set_type_default() do #(TypingNotice::register_widget(vm)) {
        ..mod.widgets.SolidView

        visible: false
        width: Fill
        height: 30
        flow: Right
        padding: Inset{left: 12.0, top: 8.0, bottom: 8.0, right: 10.0}

        show_bg: true
        draw_bg +: {
            color: mod.widgets.RINX_SELECTED
        }

        typing_label := Label {
            align: Align{x: 0.0, y: 0.5},
            padding: Inset{left: 5.0, right: 0.0, top: 0.0, bottom: 0.0}
            draw_text +: {
                color: (TYPING_NOTICE_TEXT_COLOR),
                text_style: REGULAR_TEXT {font_size: (10.5 * mod.widgets.RINX_TEXT_SCALE)}
            }
            text: #(crate::i18n::tr("Someone is typing")) i18n_text: "Someone is typing"
        }

        bouncing_dots := BouncingDots {
            margin: Inset{top: 1.1, left: -4 }
            padding: 0.0,
            draw_bg.color: (TYPING_NOTICE_TEXT_COLOR)
        }

        animator: Animator{
            typing_notice_animator: {
                default: @show
                show: AnimatorState{
                    redraw: true,
                    from: { all: Forward { duration: (mod.widgets.TYPING_NOTICE_ANIMATION_DURATION_SECS) } }
                    apply: { height: 30 }
                }
                hide: AnimatorState{
                    redraw: true,
                    from: { all: Forward { duration: (mod.widgets.TYPING_NOTICE_ANIMATION_DURATION_SECS) } }
                    apply: { height: 0 }
                }
            }
        }
    }
}

/// A notice that slides into view when someone is typing.
#[derive(Script, ScriptHook, Widget, Animator)]
pub struct TypingNotice {
    #[source] source: ScriptObjectRef,
    #[deref] view: View,
    #[apply_default] animator: Animator,
}

impl Widget for TypingNotice {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if self.animator_handle_event(cx, event).must_redraw() {
            self.redraw(cx);
        }
        self.view.handle_event(cx, event, scope);
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}

fn typing_text(users: &[String]) -> Option<String> {
    let (key, values) = match users {
        [] => return None,
        [user] => ("{user} is typing", vec![("user", user.clone())]),
        [user1, user2] => ("{user1} and {user2} are typing", vec![("user1", user1.clone()), ("user2", user2.clone())]),
        [user1, user2, user3] => ("{user1}, {user2}, and {user3} are typing", vec![("user1", user1.clone()), ("user2", user2.clone()), ("user3", user3.clone())]),
        [user1, user2, others @ ..] => ("{user1}, {user2}, and {count} others are typing", vec![("user1", user1.clone()), ("user2", user2.clone()), ("count", others.len().to_string())]),
    };
    Some(crate::i18n::format(key, &values) + " ")
}

impl TypingNotice {
    /// Shows or hides the typing notice based on whether there are any typing users.
    fn show_or_hide(&mut self, cx: &mut Cx, typing_users: &[String]) {
        let Some(typing_notice_text) = typing_text(typing_users) else {
            self.animator_play(cx, ids!(typing_notice_animator.hide));
            self.view.bouncing_dots(cx, ids!(bouncing_dots)).stop_animation(cx);
            return;
        };
        // Set the typing notice text and make its view visible.
        self.view.label(cx, ids!(typing_label)).set_text(cx, &typing_notice_text);
        self.view.set_visible(cx, true);
        // Animate in the typing notice view (sliding it up from the bottom).
        self.animator_play(cx, ids!(typing_notice_animator.show));
        // Start the typing notice text animation of bouncing dots.
        self.view.bouncing_dots(cx, ids!(bouncing_dots)).start_animation(cx);
    }
}

impl TypingNoticeRef {
    /// Shows or hides the typing notice based on whether there are any typing users.
    pub fn show_or_hide(&self, cx: &mut Cx, typing_users: &[String]) {
        if let Some(mut inner) = self.borrow_mut() {
            inner.show_or_hide(cx, typing_users);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn typing_names_and_other_count_cover_one_two_three_and_many_people() {
        let users = ["Alice", "Bob", "Carol", "Dora"].map(str::to_owned);
        assert_eq!(super::typing_text(&[]), None);
        assert_eq!(super::typing_text(&users[..1]).unwrap(), "Alice is typing ");
        assert_eq!(super::typing_text(&users[..2]).unwrap(), "Alice and Bob are typing ");
        assert_eq!(super::typing_text(&users[..3]).unwrap(), "Alice, Bob, and Carol are typing ");
        assert_eq!(super::typing_text(&users).unwrap(), "Alice, Bob, and 2 others are typing ");
    }
}
