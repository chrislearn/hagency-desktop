use super::*;
use crate::{OwnerDashboardWidgetRefExt, model::Tab};
use crate::picker::OdFilterPickerWidgetRefExt;
use makepad_platform::accessibility::{AccessibilityAction, WindowTreeUpdate, accesskit as ax};
use std::collections::{BTreeMap, HashSet};
use unicode_segmentation::UnicodeSegmentation;

const MAX_WIDGETS: usize = 16_384;

#[derive(Default)]
struct State { windows: BTreeMap<(usize, u64), WindowState>, adapters: Vec<ControlAdapter> }
struct WindowState {
    generation: u64,
    scope: Vec<Option<String>>,
    tree: ax::TreeUpdate,
    targets: BTreeMap<ax::NodeId, Target>,
}

#[derive(Clone)]
struct Target { widget: WidgetRef, selection: Option<usize>, activate: Option<fn(&WidgetRef, &mut Cx)> }
// Widget UIDs are monotonically allocated by Makepad. A new owner gets a new
// namespace so native objects retained by assistive software cannot act on the
// replacement owner's controls, even after the latest snapshot is installed.
fn node_id(widget: &WidgetRef, generation: u64) -> ax::NodeId {
    ax::NodeId((generation << 32) | (widget.widget_uid().0 << 1))
}
fn text_id(widget: &WidgetRef, generation: u64) -> ax::NodeId {
    ax::NodeId(node_id(widget, generation).0 | 1)
}
fn root_id(window: WindowId) -> ax::NodeId { ax::NodeId(u64::MAX - window.0 as u64) }

fn windows(root: &WidgetRef) -> Vec<(WindowId, WidgetRef)> {
    fn visit(widget: &WidgetRef, seen: &mut HashSet<WidgetUid>, out: &mut Vec<(WindowId, WidgetRef)>) {
        if !widget.visible() || !seen.insert(widget.widget_uid()) || seen.len() > MAX_WIDGETS { return; }
        if let Some(id) = widget.as_window().window_id() { out.push((id, widget.clone())); return; }
        widget.children(&mut |_, child| visit(&child, seen, out));
    }
    let mut out = Vec::new();
    visit(root, &mut HashSet::new(), &mut out);
    out
}

fn owner_scope(root: &WidgetRef) -> Vec<Option<String>> {
    fn visit(widget: &WidgetRef, seen: &mut HashSet<WidgetUid>, out: &mut Vec<Option<String>>) {
        if !seen.insert(widget.widget_uid()) || seen.len() > MAX_WIDGETS { return; }
        if let Some(state) = widget.as_owner_dashboard().state() {
            out.push(state.owner_scope);
        }
        widget.children(&mut |_, child| visit(&child, seen, out));
    }
    let mut result = Vec::new();
    visit(root, &mut HashSet::new(), &mut result);
    result
}

struct Builder<'a> {
    cx: &'a Cx,
    nodes: Vec<(ax::NodeId, ax::Node)>,
    targets: BTreeMap<ax::NodeId, Target>,
    seen: HashSet<WidgetUid>,
    raised: HashSet<ax::NodeId>,
    labels: BTreeMap<String, String>,
    owner_expanded: bool,
    filters_expanded: bool,
    picker_expanded: bool,
    notice_text: Option<String>,
    selected_tab: Option<Tab>,
    selected_period: crate::dto::period::UsagePeriod,
    locale: crate::i18n::Locale,
    focus: ax::NodeId,
    generation: u64,
    adapters: &'a [ControlAdapter],
    row_name: Option<String>,
    row_selected: Option<bool>,
    list_child: bool,
}

fn rect(rect: Rect) -> ax::Rect {
    ax::Rect::new(rect.pos.x, rect.pos.y, rect.pos.x + rect.size.x, rect.pos.y + rect.size.y)
}
fn valid_rect(rect: Rect) -> bool {
    [rect.pos.x, rect.pos.y, rect.size.x, rect.size.y].iter().all(|v| v.is_finite())
        && rect.size.x > 0.0 && rect.size.y > 0.0
}

fn tab_for(path: &[String]) -> Option<Tab> {
    for part in path {
        match part.as_str() {
            "od_tab_overview" => return Some(Tab::Overview),
            "od_tab_resources" => return Some(Tab::Resources),
            "od_tab_usage" => return Some(Tab::Usage),
            "od_tab_activity" => return Some(Tab::Activity),
            _ => {}
        }
    }
    None
}

impl Builder<'_> {
    fn visit(&mut self, widget: &WidgetRef, path: &mut Vec<String>) -> Option<ax::NodeId> {
        if !widget.visible() || !self.seen.insert(widget.widget_uid()) || self.seen.len() > MAX_WIDGETS { return None; }
        if let Some(state) = widget.as_owner_dashboard().state() { self.selected_tab = Some(state.view.tab); self.filters_expanded = state.view.filters_open; self.selected_period = state.view.period.unwrap_or(crate::dto::period::UsagePeriod::Day); self.locale = state.locale; }
        if let Some(dashboard) = widget.borrow::<crate::widget::OwnerDashboard>() { self.owner_expanded = dashboard.owner_controls_expanded(); self.notice_text = dashboard.header_notice_text(); }
        if widget.borrow::<crate::picker::OdFilterPicker>().is_some() { self.picker_expanded = widget.as_od_filter_picker().is_open(); }
        if widget.widget_uid().0 >= (1 << 31) || self.generation >= (1 << 31) { return None; }
        let id = node_id(widget, self.generation);
        let area = widget.area();
        let bounds = area.rect(self.cx);
        let name = path.last().map(String::as_str).unwrap_or("");
        let text = widget.text();
        let is_list = widget.borrow::<crate::controls::OdList>().is_some();
        let mut node = ax::Node::new(if is_list { ax::Role::List } else if self.list_child { ax::Role::ListItem } else { ax::Role::GenericContainer });
        node.set_author_id(id.0.to_string());
        if name == "od_stop_sheet" {
            node.set_role(ax::Role::Dialog);
            if let Some(title) = descendant_label(widget, "od_stop_title") { node.set_label(title); }
            node.set_live(ax::Live::Polite);
        }
        if matches!(name, "od_tabs" | "od_period") { node.set_role(ax::Role::TabList); }
        if name == "od_filters" || name == "picker_choices" { node.set_role(ax::Role::RadioGroup); }
        if matches!(name, "od_resources" | "od_unassigned") { node.set_role(ax::Role::List); }
        if path.iter().rev().nth(1).is_some_and(|p| matches!(p.as_str(), "od_resources" | "od_unassigned")) { node.set_role(ax::Role::ListItem); }
        let mut interactive = false;
        let mut leaf = false;
        let mut extra_nodes = Vec::new();
        let custom = self.adapters.iter().find_map(|adapter| (adapter.describe)(widget, self.cx).map(|control| (control, adapter.activate)));
        let custom_activate = custom.as_ref().map(|(_, activate)| *activate);
        if let Some((control, _)) = custom {
            node.set_role(if control.selected.is_some() { ax::Role::Tab } else { ax::Role::Button });
            node.set_label(control.label);
            if let Some(selected) = control.selected { node.set_selected(selected); }
            if control.disabled { node.set_disabled(); }
            else { node.add_action(ax::Action::Click); interactive = true; }
            leaf = true;
        } else if let Some(button) = widget.borrow::<Button>() {
            let period = path.iter().find_map(|part| match part.as_str() {
                "od_period_day" => Some(crate::dto::period::UsagePeriod::Day),
                "od_period_week" => Some(crate::dto::period::UsagePeriod::Week),
                "od_period_month" => Some(crate::dto::period::UsagePeriod::Month),
                _ => None,
            });
            node.set_role(if tab_for(path).is_some() || period.is_some() { ax::Role::Tab } else { ax::Role::Button });
            if name == "od_owner_menu" { node.set_expanded(self.owner_expanded); }
            if name == "od_usage_filters_toggle" { node.set_expanded(self.filters_expanded); }
            if name == "picker_toggle" { node.set_expanded(self.picker_expanded); }
            if let Some(period) = period { node.set_selected(period == self.selected_period); }
            let picker_choice = name == "picker_choice" && path.iter().any(|part| matches!(part.as_str(),
                "od_filter_account" | "od_filter_resource" | "od_filter_model" | "od_filter_requester"));
            if picker_choice || path.iter().any(|part| part == "od_filters") {
                if let Some(selected) = self.row_selected {
                    node.set_role(ax::Role::RadioButton);
                    node.set_toggled(if selected { ax::Toggled::True } else { ax::Toggled::False });
                }
            }
            node.set_label(if name == "od_resource_open" {
                self.row_name.as_ref().map(|resource| format!("{text} · {resource}")).unwrap_or_else(|| text.clone())
            } else { text.clone() });
            if let Some(tab) = tab_for(path) { node.set_selected(self.selected_tab == Some(tab)); }
            if button.enabled() {
                node.add_action(ax::Action::Click);
                interactive = true;
            } else { node.set_disabled(); }
            leaf = true;
        } else if let Some(input) = widget.borrow::<TextInput>() {
            node.set_role(if input.is_password() { ax::Role::PasswordInput }
                else if input.is_multiline() { ax::Role::MultilineTextInput } else { ax::Role::TextInput });
            let label = self.labels.get(&format!("{name}_label")).cloned()
                .unwrap_or_else(|| if input.empty_text().is_empty() { name.replace('_', " ") } else { input.empty_text().into() });
            node.set_label(label);
            node.set_placeholder(input.empty_text());
            let value = input.display_text();
            node.set_value(value.clone());
            if input.is_read_only() { node.set_read_only(); }
            else { node.add_action(ax::Action::SetValue); }
            interactive = true;
            // Password contents never enter the tree, including text runs.
            if !input.is_password() {
                let segments: Vec<&str> = value.graphemes(true).collect();
                if segments.iter().all(|g| g.len() <= u8::MAX as usize) {
                    let child_id = text_id(widget, self.generation);
                    let mut run = ax::Node::new(ax::Role::TextRun);
                    run.set_value(value.clone());
                    run.set_character_lengths(segments.iter().map(|g| g.len() as u8).collect::<Vec<_>>());
                    run.set_bounds(rect(bounds));
                    let selection = input.selection();
                    let index = |byte: usize| value.grapheme_indices(true).take_while(|(at, _)| *at < byte).count();
                    node.set_text_selection(ax::TextSelection {
                        anchor: ax::TextPosition { node: child_id, character_index: index(selection.anchor.index) },
                        focus: ax::TextPosition { node: child_id, character_index: index(selection.cursor.index) },
                    });
                    node.add_action(ax::Action::SetTextSelection);
                    node.set_children([child_id]);
                    extra_nodes.push((child_id, run));
                }
            }
            leaf = true;
        } else if let Some(dropdown) = widget.borrow::<DropDown>() {
            node.set_role(ax::Role::ComboBox);
            let chinese = self.locale == crate::i18n::Locale::ZhHans;
            node.set_label(self.labels.get(&format!("{name}_label")).cloned().unwrap_or_else(|| match name {
                "od_destination" => if chinese { "控制台页面" } else { "Dashboard page" }.to_owned(),
                "od_period_picker" => if chinese { "统计周期" } else { "Reporting period" }.to_owned(),
                _ => name.replace('_', " "),
            }));
            node.set_value(dropdown.selected_item_label());
            node.set_expanded(dropdown.accessibility_is_expanded());
            node.add_action(if dropdown.accessibility_is_expanded() { ax::Action::Collapse } else { ax::Action::Expand });
            node.add_action(ax::Action::Click);
            let mut options = Vec::new();
            let mut popup_bounds: Option<ax::Rect> = None;
            for (index, label, item) in dropdown.accessibility_items() {
                let bounds = item.area().rect(self.cx);
                if !valid_rect(bounds) { continue; }
                let item_id = node_id(&item, self.generation);
                let mut option = ax::Node::new(ax::Role::MenuListOption);
                option.set_author_id(item_id.0.to_string());
                option.set_label(label);
                option.set_selected(index == dropdown.selected_item_index());
                let bounds = rect(bounds);
                popup_bounds = Some(popup_bounds.map_or(bounds, |old| ax::Rect::new(old.x0.min(bounds.x0), old.y0.min(bounds.y0), old.x1.max(bounds.x1), old.y1.max(bounds.y1))));
                option.set_bounds(bounds);
                option.add_action(ax::Action::Click);
                self.targets.insert(item_id, Target { widget: widget.clone(), selection: Some(index), activate: None });
                extra_nodes.push((item_id, option));
                options.push(item_id);
            }
            if !options.is_empty() {
                let popup_id = text_id(widget, self.generation);
                let mut popup = ax::Node::new(ax::Role::MenuListPopup);
                popup.set_author_id(popup_id.0.to_string());
                popup.set_children(options);
                if let Some(bounds) = popup_bounds { popup.set_bounds(bounds); }
                extra_nodes.push((popup_id, popup));
                node.set_children([popup_id]);
                self.raised.insert(id);
            }
            interactive = true;
            leaf = true;
        } else if let Some(check) = widget.borrow::<CheckBox>() {
            node.set_role(ax::Role::CheckBox);
            node.set_label(text.clone());
            node.set_toggled(if check.active(self.cx) { ax::Toggled::True } else { ax::Toggled::False });
            node.add_action(ax::Action::Click);
            interactive = true;
            leaf = true;
        } else if widget.borrow::<Label>().is_some() && !text.is_empty() {
            let heading = name == "od_title" || name == "setup_title" || name.ends_with("_header") || name.ends_with("_title");
            node.set_role(if heading { ax::Role::Heading } else { ax::Role::Label });
            node.set_label(text.clone());
            if name == "od_sync" {
                if let Some(notice) = &self.notice_text { node.set_description(notice.clone()); node.set_live(ax::Live::Polite); }
            }
            if heading { node.set_level(2); }
            // Status labels are read as text. Only the explicit notice/command
            // message is live; the two-second freshness clock never interrupts.
            if matches!(name, "od_command_status" | "setup_status" | "od_date_error") || (name == "text" && path.iter().any(|p| p == "od_notice")) {
                node.set_live(ax::Live::Polite);
            }
            leaf = true;
        }
        if widget.disabled(self.cx) {
            node.set_disabled();
            node.remove_action(ax::Action::Click);
            node.remove_action(ax::Action::SetValue);
            node.remove_action(ax::Action::Expand);
            node.remove_action(ax::Action::Collapse);
            interactive = false;
        }
        if interactive {
            node.add_action(ax::Action::Focus);
            node.add_action(ax::Action::ScrollIntoView);
            self.targets.insert(id, Target { widget: widget.clone(), selection: None, activate: custom_activate });
        }
        if area == self.cx.key_focus() && interactive { self.focus = id; }
        if valid_rect(bounds) { node.set_bounds(rect(bounds)); }
        let mut children = Vec::new();
        if !leaf {
            widget.children(&mut |name, child| {
                path.push(name.to_string());
                let saved_name = self.row_name.clone();
                let saved_selected = self.row_selected;
                let saved_list = self.list_child;
                if is_list {
                    self.row_name = descendant_label(&child, "name");
                    self.row_selected = None;
                    child.children(&mut |id, child| { if id.to_string() == "sel" { self.row_selected = Some(child.visible()); } });
                }
                self.list_child = is_list;
                if let Some(id) = self.visit(&child, path) { children.push(id); }
                self.row_name = saved_name;
                self.row_selected = saved_selected;
                self.list_child = saved_list;
                path.pop();
            });
            if children.is_empty() { return None; }
            // AccessKit hit-tests children last-to-first. Makepad popup menus
            // render above the body even though their semantic parent is a
            // header dropdown. Raise that branch while it is open, preserving
            // the ComboBox -> Menu hierarchy and normal order once it closes.
            if children.iter().any(|child| self.raised.contains(child)) {
                children.sort_by_key(|child| self.raised.contains(child));
                self.raised.insert(id);
            }
            node.set_children(children);
        } else if !valid_rect(bounds) {
            self.targets.remove(&id);
            for (extra, _) in &extra_nodes { self.targets.remove(extra); }
            return None;
        }
        self.nodes.extend(extra_nodes);
        self.nodes.push((id, node));
        Some(id)
    }
}

fn descendant_label(widget: &WidgetRef, wanted: &str) -> Option<String> {
    let mut result = None;
    widget.children(&mut |id, child| {
        if result.is_some() { return; }
        if id.to_string() == wanted && child.borrow::<Label>().is_some() { result = Some(child.text()); }
        else { result = descendant_label(&child, wanted); }
    });
    result
}

fn labels(root: &WidgetRef) -> BTreeMap<String, String> {
    fn visit(widget: &WidgetRef, id: LiveId, seen: &mut HashSet<WidgetUid>, labels: &mut BTreeMap<String, String>) {
        if !widget.visible() || !seen.insert(widget.widget_uid()) || seen.len() > MAX_WIDGETS { return; }
        let name = id.to_string();
        if name.ends_with("_label") && widget.borrow::<Label>().is_some() { labels.insert(name, widget.text()); }
        widget.children(&mut |id, child| visit(&child, id, seen, labels));
    }
    let mut result = BTreeMap::new();
    visit(root, LiveId(0), &mut HashSet::new(), &mut result);
    result
}

fn build(root: &WidgetRef, cx: &Cx, window: WindowId, generation: u64, adapters: &[ControlAdapter]) -> (ax::TreeUpdate, BTreeMap<ax::NodeId, Target>) {
    let root_id = root_id(window);
    let mut builder = Builder { cx, nodes: Vec::new(), targets: BTreeMap::new(), seen: HashSet::new(), raised: HashSet::new(),
        labels: labels(root), owner_expanded: false, filters_expanded: false, picker_expanded: false, notice_text: None, selected_tab: None, selected_period: crate::dto::period::UsagePeriod::Day, locale: crate::i18n::Locale::En, focus: root_id, generation, adapters, row_name: None, row_selected: None, list_child: false };
    let child = builder.visit(root, &mut Vec::new());
    let mut node = ax::Node::new(ax::Role::Window);
    node.set_label(cx.windows[window].create_title.clone());
    // AccessKit uses physical pixels; Makepad areas use its effective logical
    // coordinate system. The native adapter converts physical to AppKit points.
    node.set_transform(ax::Affine::scale(cx.windows[window].effective_dpi_factor()));
    node.set_children(child.into_iter().collect::<Vec<_>>());
    builder.nodes.push((root_id, node));
    let mut tree = ax::TreeInfo::new(root_id);
    tree.toolkit_name = Some("Makepad / Hagency".into());
    (ax::TreeUpdate { nodes: builder.nodes, tree: Some(tree), tree_id: ax::TreeId::ROOT, focus: builder.focus }, builder.targets)
}

fn byte_at_grapheme(value: &str, index: usize) -> usize {
    value.grapheme_indices(true).nth(index).map(|(at, _)| at).unwrap_or(value.len())
}

fn apply(cx: &mut Cx, target: &Target, generation: u64, request: &ax::ActionRequest) {
    let widget = &target.widget;
    if let Some(index) = target.selection {
        if request.action == ax::Action::Click {
            if let Some(mut dropdown) = widget.borrow_mut::<DropDown>() {
                dropdown.accessibility_select(cx, index);
            }
        }
        return;
    }
    if request.action == ax::Action::Click {
        if let Some(activate) = target.activate {
            cx.set_key_focus(widget.area());
            activate(widget, cx);
            widget.redraw(cx);
            return;
        }
    }
    let uid = widget.widget_uid();
    match request.action {
        ax::Action::Focus | ax::Action::ScrollIntoView => {
            crate::keyboard::focus(cx, widget.area());
            if let Some(mut input) = widget.borrow_mut::<TextInput>() { input.take_key_focus(cx); }
        }
        ax::Action::Click => {
            // Activation may immediately open a dialog or focus an invalid
            // field. Do not enqueue a competing reveal for the old button;
            // explicit native Focus/ScrollIntoView handles navigation above.
            cx.set_key_focus(widget.area());
            if widget.borrow::<Button>().is_some_and(|b| b.enabled()) {
                cx.widget_action(uid, ButtonAction::Clicked(KeyModifiers::default()));
            } else if let Some(mut dropdown) = widget.borrow_mut::<DropDown>() {
                if dropdown.accessibility_is_expanded() { dropdown.set_closed(cx); }
                else { dropdown.set_active(cx); }
            } else if let Some(mut check) = widget.borrow_mut::<CheckBox>() {
                let active = !check.active(cx);
                check.set_active(cx, active, Animate::No);
                cx.widget_action(uid, CheckBoxAction::Change(active));
            }
        }
        ax::Action::Expand | ax::Action::Collapse => {
            if let Some(mut dropdown) = widget.borrow_mut::<DropDown>() {
                if request.action == ax::Action::Expand { dropdown.set_active(cx); }
                else { dropdown.set_closed(cx); }
            }
        }
        ax::Action::SetValue => {
            if let Some(ax::ActionData::Value(value)) = &request.data {
                if value.len() <= 65_536 {
                    if let Some(mut input) = widget.borrow_mut::<TextInput>() {
                        if !input.is_read_only() {
                            input.set_text(cx, value);
                            cx.widget_action(uid, TextInputAction::Changed(value.to_string()));
                        }
                    }
                }
            }
        }
        ax::Action::SetTextSelection => {
            if let Some(ax::ActionData::SetTextSelection(selection)) = &request.data {
                if selection.anchor.node == text_id(widget, generation) && selection.focus.node == text_id(widget, generation) {
                    if let Some(mut input) = widget.borrow_mut::<TextInput>() {
                        let text = input.text();
                        use makepad_draw::text::selection::{Cursor, Selection};
                        input.set_selection(cx, Selection {
                            anchor: Cursor { index: byte_at_grapheme(&text, selection.anchor.character_index), prefer_next_row: false },
                            cursor: Cursor { index: byte_at_grapheme(&text, selection.focus.character_index), prefer_next_row: false },
                        });
                        input.take_key_focus(cx);
                    }
                }
            }
        }
        _ => {}
    }
    widget.redraw(cx);
}

pub fn after_event(root: &WidgetRef, cx: &mut Cx, event: &Event) {
    let mut state = std::mem::take(cx.global::<State>());
    let windows = windows(root);
    if let Event::Actions(actions) = event {
        for event in actions.iter().filter_map(|a| a.downcast_ref::<AccessibilityAction>()) {
            if event.request.target_tree != ax::TreeId::ROOT { continue; }
            let key = (event.window_id.0, event.window_id.1);
            if let Some(current) = state.windows.get(&key) {
                if current.generation != event.generation { continue; }
                let Some((_, window)) = windows.iter().find(|(id, _)| *id == event.window_id) else { continue };
                if owner_scope(window) != current.scope { continue; }
                // Rebuild current visible targets so a hidden/disabled control
                // cannot receive an action queued before a tab/state change.
                let (tree, targets) = build(window, cx, event.window_id, current.generation, &state.adapters);
                if let Some(widget) = targets.get(&event.request.target_node) {
                    let supported = tree.nodes.iter().find(|(id, _)| *id == event.request.target_node)
                        .is_some_and(|(_, node)| node.supports_action(event.request.action));
                    if supported {
                        if widget.selection.is_none() && matches!(event.request.action, ax::Action::Click | ax::Action::Expand)
                            && widget.widget.borrow::<DropDown>().is_some() {
                            // Native activation doesn't synthesize the outside
                            // pointer event that normally closes another menu.
                            for other in targets.values() {
                                if other.widget.widget_uid() != widget.widget.widget_uid() {
                                    if let Some(mut dropdown) = other.widget.borrow_mut::<DropDown>() {
                                        if dropdown.accessibility_is_expanded() { dropdown.set_closed(cx); }
                                    }
                                }
                            }
                        }
                        apply(cx, widget, current.generation, &event.request);
                    }
                }
            }
        }
    }
    // Focus requested during layout is committed only after Draw returns.
    // Publish that committed focus even when reduced motion needs no redraw.
    if matches!(event, Event::Draw(_) | Event::KeyFocus(_)) {
        let live: Vec<_> = windows.iter().map(|(id, _)| (id.0, id.1)).collect();
        state.windows.retain(|id, _| live.contains(id));
        for (window_id, window) in windows {
            let key = (window_id.0, window_id.1);
            let scope = owner_scope(&window);
            let old = state.windows.get(&key);
            let generation = old.map_or(1, |old| if old.scope == scope { old.generation } else { old.generation.saturating_add(1) });
            let (tree, targets) = build(&window, cx, window_id, generation, &state.adapters);
            if old.is_none_or(|old| old.tree != tree || old.generation != generation) {
                cx.update_window_accessibility(WindowTreeUpdate { window_id, generation, tree: tree.clone() });
            }
            state.windows.insert(key, WindowState { generation, scope, tree, targets });
        }
    }
    if let Event::WindowClosed(event) = event {
        state.windows.remove(&(event.window_id.0, event.window_id.1));
    }
    *cx.global::<State>() = state;
}

pub fn diagnostics(cx: &mut Cx) -> serde_json::Value {
    let state = cx.global::<State>();
    serde_json::json!({"nativeAccessibility": true, "windows": state.windows.iter().map(|(id, state)| {
        serde_json::json!({"window": id.0, "generation": state.generation, "tree": state.tree,
            "actionTargets": state.targets.keys().map(|n| n.0).collect::<Vec<_>>()})
    }).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_selection_uses_graphemes_not_bytes_or_codepoints() {
        let text = "A你e\u{301}👩‍💻";
        assert_eq!((0..=4).map(|i| byte_at_grapheme(text, i)).collect::<Vec<_>>(), vec![0, 1, 4, 7, text.len()]);
        assert_eq!(byte_at_grapheme(text, 999), text.len());
    }
}

#[cfg(all(feature = "fixtures", target_os = "macos"))]
#[path = "native_probe.rs"]
mod native_probe;
#[cfg(all(feature = "fixtures", target_os = "macos"))]
pub fn native_fixture_probe(cx: &mut Cx, press_node_id: &str) -> serde_json::Value {
    let windows = cx.global::<State>().windows.keys().map(|&(a,b)| WindowId(a,b)).collect();
    native_probe::probe(windows, press_node_id)
}

pub(super) fn register_control_adapter(cx: &mut Cx, adapter: ControlAdapter) {
    let adapters = &mut cx.global::<State>().adapters;
    if let Some(old) = adapters.iter_mut().find(|a| a.name == adapter.name) { *old = adapter; }
    else { adapters.push(adapter); }
}
