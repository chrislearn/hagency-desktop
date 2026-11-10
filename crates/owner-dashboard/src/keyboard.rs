//! Dashboard keyboard handling, called by hosts before Window's navigation.
//! A handled key must not be dispatched again: the pinned Window otherwise
//! advances Tab a second time. Text entry and IME stay with the focused input.
use makepad_widgets::*;
use std::collections::HashSet;

pub fn contains_focus(root: &dyn WidgetNode, cx: &Cx) -> bool {
    if !root.visible() || !cx.key_focus().is_valid(cx) { return false; }
    if root.area().is_valid(cx) && root.area() == cx.key_focus() { return true; }
    let mut found = false;
    root.children(&mut |_, child| {
        if !found && child.visible() {
            found = controls(&child, cx).iter().any(|control| control.area() == cx.key_focus());
        }
    });
    found
}

pub fn controls(root: &WidgetRef, cx: &Cx) -> Vec<WidgetRef> {
    fn visit(root: &WidgetRef, cx: &Cx, seen: &mut HashSet<WidgetUid>, out: &mut Vec<WidgetRef>) {
        if !root.visible() || !seen.insert(root.widget_uid()) || seen.len() > 8192 { return; }
        let interactive = root.borrow::<Button>().is_some_and(|button| button.enabled())
            || root.borrow::<TextInput>().is_some() || root.borrow::<DropDown>().is_some()
            || root.borrow::<CheckBox>().is_some();
        if interactive && root.area().is_valid(cx) { out.push(root.clone()); }
        root.children(&mut |_, child| visit(&child, cx, seen, out));
    }
    let mut result = Vec::new();
    visit(root, cx, &mut HashSet::new(), &mut result);
    result
}

fn next(current: Option<usize>, count: usize, backward: bool, wrap: bool) -> Option<usize> {
    if count == 0 { return None; }
    match (current, backward) {
        (None, false) => Some(0),
        (None, true) => Some(count - 1),
        (Some(i), false) if i + 1 < count => Some(i + 1),
        (Some(i), true) if i > 0 => Some(i - 1),
        (_, false) if wrap => Some(0),
        (_, true) if wrap => Some(count - 1),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct RevealFocus(Area);

pub fn focus(cx: &mut Cx, target: Area) {
    cx.set_key_focus(target);
    // Setup and host controls can live outside a visible dashboard. Their
    // event-time navigation stack is already complete and must work without
    // relying on a dashboard to consume the correction below.
    if !cx.in_draw_event() { reveal(cx, target, true); }
    // A picker can focus a freshly drawn row before its enclosing ScrollY
    // finishes drawing. Its navigation BeginScroll still has Area::Empty at
    // that point. Resolve the stack after the complete UI draw, through the
    // ordinary action queue, and ignore requests superseded by later focus.
    cx.action(RevealFocus(target));
}

pub(crate) fn reveal_focus(cx: &mut Cx, event: &Event, content: &WidgetRef, scroll_y: f64) {
    let Event::Actions(actions) = event else { return };
    for request in actions.iter().filter_map(|action| action.downcast_ref::<RevealFocus>()) {
        if cx.key_focus() == request.0 && request.0.is_valid(cx) {
            let viewport = content.area().rect(cx);
            let target = request.0.rect(cx);
            if target.size.y > viewport.size.y - 10.0
                && controls(content, cx).iter().any(|control| control.area() == request.0) {
                // A fully wrapped name can itself exceed this viewport at
                // large text sizes. Show its beginning, not just the tail;
                // the ordinary scrollbar still reaches the rest of the name.
                let y = oversized_reveal_y(scroll_y, viewport.pos.y, target.pos.y);
                content.as_view().set_scroll_pos(cx, dvec2(0.0, y));
                cx.redraw_area_in_draw(content.area());
            } else {
                reveal(cx, request.0, false);
            }
        }
    }
}

fn oversized_reveal_y(scroll: f64, viewport_top: f64, target_top: f64) -> f64 {
    (scroll + target_top - viewport_top - 5.0).max(0.0)
}

fn reveal(cx: &mut Cx, target: Area, defer_oversized: bool) {
    // Use Makepad's navigation scroll stack, including nested scroll views.
    if let Some(mut root) = target.draw_list_id() {
        for _ in 0..128 {
            let Some(parent) = cx.draw_lists[root].codeflow_parent_id else { break };
            root = parent;
        }
        if let Some((_, stack)) = CxDraw::iterate_nav_stops(cx, root, |_, stop| (stop.area == target).then_some(target)) {
            // Do not start a smooth tail-alignment before the dashboard's
            // oversized correction. An already top-aligned target must stay
            // there even when its native Focus action is repeated.
            if defer_oversized && stack.iter().take(stack.len().saturating_sub(1))
                .any(|area| area.is_valid(cx) && target.rect(cx).size.y > area.rect(cx).size.y - 10.0) { return; }
            NavControl::send_trigger_to_scroll_stack(cx, stack);
        }
    }
}

/// Returns true only when this scope consumed the key. Without `wrap`, Tab
/// may leave the first/last control for the host's own navigation.
pub fn handle(root: &WidgetRef, cx: &mut Cx, key: &KeyEvent, wrap: bool) -> bool {
    if key.is_repeat || key.modifiers.is_primary() || key.modifiers.alt { return false; }
    if !matches!(key.key_code, KeyCode::Tab | KeyCode::ReturnKey | KeyCode::Space | KeyCode::Escape) { return false; }
    let controls = controls(root, cx);
    let current = controls.iter().position(|widget| widget.area() == cx.key_focus());
    // Without wrap, only a scope that already owns focus is handled. Empty
    // focus belongs to the window, which chooses its own first navigation stop.
    if !wrap && current.is_none() { return false; }
    if key.key_code == KeyCode::Tab {
        if let Some(index) = next(current, controls.len(), key.modifiers.shift, wrap) {
            focus(cx, controls[index].area());
            return true;
        }
    } else if let Some(index) = current {
        let control = &controls[index];
        if key.key_code == KeyCode::Escape {
            if let Some(mut dropdown) = control.borrow_mut::<DropDown>() {
                dropdown.set_closed(cx);
                return true;
            }
            return false;
        }
        if control.borrow::<Button>().is_some() {
            cx.widget_action(control.widget_uid(), ButtonAction::Clicked(key.modifiers));
            return true;
        }
        if let Some(mut dropdown) = control.borrow_mut::<DropDown>() {
            dropdown.set_active(cx);
            return true;
        }
        if let Some(mut check) = control.borrow_mut::<CheckBox>() {
            let active = !check.active(cx);
            check.set_active(cx, active, Animate::Yes);
            cx.widget_action(control.widget_uid(), CheckBoxAction::Change(active));
            return true;
        }
    }
    false
}

/// The host retains navigation outside the dashboard. Discover only a
/// visible dashboard; a hidden Agent tab cannot intercept chat typing.
pub fn embedded(root: &WidgetRef, cx: &mut Cx, event: &Event) -> bool {
    let Event::KeyDown(key) = event else { return false };
    fn find(root: &WidgetRef, cx: &Cx, seen: &mut HashSet<WidgetUid>) -> Option<WidgetRef> {
        if !root.visible() || !seen.insert(root.widget_uid()) || seen.len() > 8192 { return None; }
        if root.borrow::<crate::widget::OwnerDashboard>().is_some() {
            return controls(root, cx).iter().any(|control| control.area() == cx.key_focus()).then(|| root.clone());
        }
        let mut found = None;
        root.children(&mut |_, child| { if found.is_none() { found = find(&child, cx, seen); } });
        found
    }
    find(root, cx, &mut HashSet::new()).is_some_and(|dashboard| handle(&dashboard, cx, key, false))
}

#[cfg(test)]
mod tests {
    use super::{next, oversized_reveal_y};
    #[test]
    fn oversized_focus_reveals_the_start_without_oscillating() {
        assert_eq!(oversized_reveal_y(1000.0, 232.0, 700.0), 1463.0);
        assert_eq!(oversized_reveal_y(1463.0, 232.0, 237.0), 1463.0);
        assert_eq!(oversized_reveal_y(0.0, 232.0, 232.0), 0.0);
    }
    #[test]
    fn traversal_seeds_reverses_and_leaves_embedded_scope() {
        assert_eq!(next(None, 3, false, false), Some(0));
        assert_eq!(next(None, 3, true, false), Some(2));
        assert_eq!(next(Some(0), 3, false, false), Some(1));
        assert_eq!(next(Some(1), 3, true, false), Some(0));
        assert_eq!(next(Some(2), 3, false, false), None);
        assert_eq!(next(Some(0), 3, true, false), None);
        assert_eq!(next(Some(2), 3, false, true), Some(0));
        assert_eq!(next(Some(0), 3, true, true), Some(2));
        assert_eq!(next(None, 0, false, true), None);
    }
}
