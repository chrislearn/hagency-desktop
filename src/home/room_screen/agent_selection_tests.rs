use super::*;
use crate::agent_chat::reply::AgentReplyWidgetRefExt;
use makepad_widgets::makepad_draw::cx_draw::CxDraw;
use std::cell::Cell;

fn fixture(condensed: bool) -> (Cx, WidgetRef) {
    let mut cx = Cx::new(Box::new(|_, _| {}));
    let message = cx.with_vm(|vm| {
        makepad_widgets::script_mod(vm);
        crate::app::register_widgets(vm);
        vm.bx.captured_errors = Some(Vec::new());
        let value = if condensed {
            script_eval!(vm, {mod.widgets.CondensedMessage {}})
        } else {
            script_eval!(vm, {mod.widgets.Message {}})
        };
        let message = WidgetRef::script_from_value(vm, value);
        message.register_as_ui_root(vm);
        assert!(vm.take_errors().is_empty());
        message
    });
    message
        .widget(&mut cx, ids!(content.message))
        .set_visible(&mut cx, false);
    message.borrow_mut::<Message>().unwrap().details = Some(MessageDetails {
        item_id: 0,
        timeline_event_id: TimelineEventItemId::EventId("$selection".try_into().unwrap()),
        related_event_id: None,
        thread_root_event_id: None,
        room_screen_widget_uid: message.widget_uid(),
        should_be_highlighted: false,
        abilities: MessageAbilities::empty(),
        selected_text: None,
    });
    (cx, message)
}

fn draw(cx: &mut Cx, message: &WidgetRef) {
    cx.redraw_all();
    let pass = DrawPass::new(cx);
    pass.set_size(cx, dvec2(800.0, 1600.0));
    let mut list = DrawList2d::new(cx);
    let event = DrawEvent::default();
    let mut draw = CxDraw::new(cx, &event);
    let mut cx = Cx2d::new(&mut draw);
    cx.begin_pass(&pass, None);
    list.begin_always(&mut cx);
    cx.begin_root_turtle(dvec2(800.0, 1600.0), Layout::flow_overlay());
    message.draw_walk_all(
        &mut cx,
        &mut Scope::empty(),
        Walk {
            width: Size::Fixed(800.0),
            height: Size::Fit {
                min: None,
                max: None,
            },
            ..Walk::default()
        },
    );
    cx.end_pass_sized_turtle();
    list.end(&mut cx);
    cx.end_pass(&pass);
}

fn drag(cx: &mut Cx, message: &WidgetRef) -> String {
    let body = message.html_or_plaintext(cx, ids!(content.agent_reply.agent_reply_body));
    let rect = body.area().clipped_rect(cx);
    assert!(rect.size.x > 80.0 && rect.size.y > 0.0, "{rect:?}");
    let start = rect.pos + dvec2(3.0, 8.0);
    let end = start + dvec2(75.0, 0.0);
    let events = [
        Event::MouseDown(MouseDownEvent {
            abs: start,
            button: MouseButton::PRIMARY,
            window_id: WindowId(0, 0),
            modifiers: KeyModifiers::default(),
            handled: Cell::new(Area::Empty),
            time: 1.0,
        }),
        Event::MouseMove(MouseMoveEvent {
            abs: end,
            lock_delta: DVec2::default(),
            window_id: WindowId(0, 0),
            modifiers: KeyModifiers::default(),
            handled: Cell::new(Area::Empty),
            time: 1.2,
        }),
        Event::MouseUp(MouseUpEvent {
            abs: end,
            button: MouseButton::PRIMARY,
            window_id: WindowId(0, 0),
            modifiers: KeyModifiers::default(),
            time: 1.3,
        }),
    ];
    for event in events {
        message.handle_event(cx, &event, &mut Scope::empty());
    }
    let selected = body.selected_text(cx);
    assert!(
        !selected.is_empty(),
        "Dragging over a rendered Agent answer must select text"
    );
    selected
}

#[test]
fn agent_reply_drag_selection_reaches_context_menu_and_survives_redraw() {
    for condensed in [false, true] {
        for rich in [false, true] {
            let (mut cx, message) = fixture(condensed);
            let body = "Select this part of the answer 中文，leave the rest unselected.";
            let formatted = rich.then(|| FormattedBody::html(format!("<p>{body}</p>")));
            let reply = message.agent_reply(&mut cx, ids!(content.agent_reply));
            reply.populate(
                &mut cx,
                "account|room|first".into(),
                body,
                formatted.as_ref(),
                false,
                false,
            );
            draw(&mut cx, &message);
            let selected = drag(&mut cx, &message);
            assert!(selected.len() < body.len());
            draw(&mut cx, &message);
            assert_eq!(
                message
                    .html_or_plaintext(&mut cx, ids!(content.agent_reply.agent_reply_body))
                    .selected_text(&mut cx),
                selected
            );
            let actions = cx.capture_actions(|cx| {
                let mut row = message.borrow_mut::<Message>().unwrap();
                let details = row.details.clone().unwrap();
                row.open_context_menu(
                    cx,
                    details.room_screen_widget_uid,
                    details,
                    dvec2(100.0, 100.0),
                );
            });
            assert!(actions.iter().any(|action| matches!(action.as_widget_action().cast::<MessageAction>(),
                MessageAction::OpenMessageContextMenu {details, ..} if details.selected_text.as_deref() == Some(&selected))));
            // Same text, different event/room: never inherit another message's selection.
            reply.populate(
                &mut cx,
                "account|other-room|second".into(),
                body,
                formatted.as_ref(),
                false,
                false,
            );
            assert!(
                message
                    .html_or_plaintext(&mut cx, ids!(content.agent_reply.agent_reply_body))
                    .selected_text(&mut cx)
                    .is_empty()
            );
            draw(&mut cx, &message);
            drag(&mut cx, &message);
            reply.hide(&mut cx);
            assert!(
                message
                    .html_or_plaintext(&mut cx, ids!(content.agent_reply.agent_reply_body))
                    .selected_text(&mut cx)
                    .is_empty()
            );
        }
    }
}

#[test]
fn folded_and_expanded_agent_answers_both_support_drag_selection() {
    let (mut cx, message) = fixture(false);
    let body = "Select this line of a long answer 中文。\n".repeat(12);
    let formatted = FormattedBody::markdown(&body);
    let reply = message.agent_reply(&mut cx, ids!(content.agent_reply));
    reply.populate(
        &mut cx,
        "folded-selection".into(),
        &body,
        formatted.as_ref(),
        false,
        false,
    );
    for path in [ids!(agent_reply_expand), ids!(agent_reply_collapse)] {
        draw(&mut cx, &message);
        drag(&mut cx, &message);
        let button = reply.widget(&mut cx, path);
        assert!(button.visible());
        let actions = cx.capture_actions(|cx| {
            cx.widget_action(
                button.widget_uid(),
                crate::shared::navigation_bar_button::NavigationBarButtonAction::Clicked,
            )
        });
        reply.handle_event(&mut cx, &Event::Actions(actions), &mut Scope::empty());
        assert!(
            message
                .html_or_plaintext(&mut cx, ids!(content.agent_reply.agent_reply_body))
                .selected_text(&mut cx)
                .is_empty()
        );
    }
}
