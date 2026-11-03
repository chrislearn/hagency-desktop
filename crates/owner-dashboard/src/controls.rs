//! Small production controls used by the dashboard:
//!
//! - [`OdGlyph`]: a status glyph drawn as an SDF shape in a tinted circle.
//!   Status always pairs it with text, so color is never the only signal.
//! - [`OdMeter`]: a quota bar with a distinct striped "not reported" state.
//! - [`OdList`]: a keyed list. Rows are created from templates once per
//!   stable ID and reused across refreshes, so a telemetry update never
//!   remounts rows, resets their state or moves unaffected bounds.
//!
//! Tone/glyph/fraction live in `#[rust]` fields and are pushed into the
//! shader at draw time, so a theme reapply cannot reset them.
use crate::model::{Glyph, Tone};
use makepad_widgets::*;
use std::collections::HashMap;

script_mod! {
    use mod.prelude.widgets_internal.*
    use mod.widgets.*

    set_type_default() do #(DrawOdGlyph::script_shader(vm)){
        ..mod.draw.DrawQuad
        pixel: fn(){
            let sz = self.rect_size
            let sdf = Sdf2d.viewport(self.pos * sz)
            let r = min(sz.x, sz.y) * 0.5
            let cx = sz.x * 0.5
            let cy = sz.y * 0.5
            sdf.circle(cx, cy, r)
            sdf.fill(self.bg)
            let s = r * 0.52
            let w = max(1.4, r * 0.17)
            if self.glyph < 0.5 {
                sdf.move_to(cx - s * 0.85, cy + s * 0.05)
                sdf.line_to(cx - s * 0.25, cy + s * 0.65)
                sdf.line_to(cx + s * 0.9, cy - s * 0.6)
                sdf.stroke(self.fg, w)
            } else if self.glyph < 1.5 {
                sdf.rect(cx - s * 0.65, cy - s * 0.7, s * 0.45, s * 1.4)
                sdf.fill(self.fg)
                sdf.rect(cx + s * 0.2, cy - s * 0.7, s * 0.45, s * 1.4)
                sdf.fill(self.fg)
            } else if self.glyph < 2.5 {
                sdf.box(cx - s * 0.62, cy - s * 0.62, s * 1.24, s * 1.24, s * 0.18)
                sdf.fill(self.fg)
            } else if self.glyph < 3.5 {
                sdf.box(cx - w * 0.5, cy - s * 0.85, w, s * 1.15, w * 0.5)
                sdf.fill(self.fg)
                sdf.circle(cx, cy + s * 0.68, w * 0.62)
                sdf.fill(self.fg)
            } else if self.glyph < 4.5 {
                sdf.move_to(cx - s * 0.6, cy - s * 0.6)
                sdf.line_to(cx + s * 0.6, cy + s * 0.6)
                sdf.stroke(self.fg, w)
                sdf.move_to(cx + s * 0.6, cy - s * 0.6)
                sdf.line_to(cx - s * 0.6, cy + s * 0.6)
                sdf.stroke(self.fg, w)
            } else if self.glyph < 5.5 {
                sdf.circle(cx, cy, s * 0.95)
                sdf.stroke(self.fg, w * 0.85)
                sdf.move_to(cx, cy - s * 0.55)
                sdf.line_to(cx, cy)
                sdf.line_to(cx + s * 0.4, cy + s * 0.3)
                sdf.stroke(self.fg, w * 0.85)
            } else if self.glyph < 6.5 {
                sdf.circle(cx, cy - s * 0.62, w * 0.62)
                sdf.fill(self.fg)
                sdf.box(cx - w * 0.5, cy - s * 0.2, w, s * 1.05, w * 0.5)
                sdf.fill(self.fg)
            } else if self.glyph < 7.5 {
                sdf.move_to(cx - s * 0.45, cy - s * 0.35)
                sdf.line_to(cx - s * 0.2, cy - s * 0.72)
                sdf.line_to(cx + s * 0.3, cy - s * 0.72)
                sdf.line_to(cx + s * 0.5, cy - s * 0.35)
                sdf.line_to(cx, cy + s * 0.05)
                sdf.line_to(cx, cy + s * 0.25)
                sdf.stroke(self.fg, w * 0.9)
                sdf.circle(cx, cy + s * 0.68, w * 0.6)
                sdf.fill(self.fg)
            } else if self.glyph < 8.5 {
                sdf.circle(cx, cy, s * 0.85)
                sdf.stroke(self.fg, w * 0.85)
                sdf.move_to(cx - s * 0.75, cy - s * 0.75)
                sdf.line_to(cx + s * 0.75, cy + s * 0.75)
                sdf.stroke(self.fg, w * 0.85)
            } else {
                sdf.move_to(cx - s * 0.8, cy - s * 0.45)
                sdf.line_to(cx + s * 0.8, cy - s * 0.45)
                sdf.stroke(self.fg, w * 0.8)
                sdf.move_to(cx - s * 0.8, cy + s * 0.45)
                sdf.line_to(cx + s * 0.8, cy + s * 0.45)
                sdf.stroke(self.fg, w * 0.8)
                sdf.circle(cx - s * 0.3, cy - s * 0.45, w * 0.95)
                sdf.fill(self.fg)
                sdf.circle(cx + s * 0.35, cy + s * 0.45, w * 0.95)
                sdf.fill(self.fg)
            }
            return sdf.result
        }
    }

    set_type_default() do #(DrawOdMeter::script_shader(vm)){
        ..mod.draw.DrawQuad
        pixel: fn(){
            let sz = self.rect_size
            let p = self.pos * sz
            let sdf = Sdf2d.viewport(p)
            let r = sz.y * 0.5
            sdf.box(0., 0., sz.x, sz.y, r)
            sdf.fill(self.track)
            if self.unknown > 0.5 {
                if fract((p.x - p.y) / 7.0) < 0.5 {
                    sdf.box(0., 0., sz.x, sz.y, r)
                    sdf.fill(vec4(self.fg.rgb, 0.22))
                }
                return sdf.result
            }
            if self.fraction > 0.0 {
                let w = max(sz.y, sz.x * self.fraction)
                sdf.box(0., 0., w, sz.y, r)
                sdf.fill(self.fg)
            }
            return sdf.result
        }
    }

    mod.widgets.OdGlyphBase = #(OdGlyph::register_widget(vm))
    mod.widgets.OdGlyph = set_type_default() do mod.widgets.OdGlyphBase{
        width: OD_GLYPH
        height: OD_GLYPH
        pos_fg: OD_POS_FG pos_bg: OD_POS_BG
        cau_fg: OD_CAU_FG cau_bg: OD_CAU_BG
        cri_fg: OD_CRI_FG cri_bg: OD_CRI_BG
        neu_fg: OD_NEU_FG neu_bg: OD_NEU_BG
        pro_fg: OD_PRO_FG pro_bg: OD_PRO_BG
        acc_fg: OD_ON_ACCENT acc_bg: OD_ACCENT
    }

    mod.widgets.OdMeterBase = #(OdMeter::register_widget(vm))
    mod.widgets.OdMeter = set_type_default() do mod.widgets.OdMeterBase{
        width: Fill
        height: 8
        track: OD_TRACK
        accent: OD_ACCENT
        caution: OD_CAU_FG
        critical: OD_CRI_FG
        neutral: OD_MUTED
    }

    mod.widgets.OdListBase = #(OdList::register_widget(vm))
    mod.widgets.OdList = set_type_default() do mod.widgets.OdListBase{
        width: Fill
        height: Fit
        flow: Down
    }
}

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawOdGlyph {
    #[deref]
    draw_super: DrawQuad,
    #[live]
    pub glyph: f32,
    #[live]
    pub fg: Vec4f,
    #[live]
    pub bg: Vec4f,
}

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawOdMeter {
    #[deref]
    draw_super: DrawQuad,
    #[live]
    pub fraction: f32,
    #[live]
    pub unknown: f32,
    #[live]
    pub fg: Vec4f,
    #[live]
    pub track: Vec4f,
}

pub fn glyph_id(glyph: Glyph) -> f32 {
    match glyph {
        Glyph::Check => 0.0,
        Glyph::Pause => 1.0,
        Glyph::Stop => 2.0,
        Glyph::Warning => 3.0,
        Glyph::Cross => 4.0,
        Glyph::Clock => 5.0,
        Glyph::Info => 6.0,
        Glyph::Question => 7.0,
        Glyph::Offline => 8.0,
        Glyph::Setup => 9.0,
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct OdGlyph {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[walk]
    walk: Walk,
    #[live(true)]
    #[visible]
    visible: bool,
    #[redraw]
    #[live]
    draw_glyph: DrawOdGlyph,
    #[live]
    pos_fg: Vec4f,
    #[live]
    pos_bg: Vec4f,
    #[live]
    cau_fg: Vec4f,
    #[live]
    cau_bg: Vec4f,
    #[live]
    cri_fg: Vec4f,
    #[live]
    cri_bg: Vec4f,
    #[live]
    neu_fg: Vec4f,
    #[live]
    neu_bg: Vec4f,
    #[live]
    pro_fg: Vec4f,
    #[live]
    pro_bg: Vec4f,
    #[live]
    acc_fg: Vec4f,
    #[live]
    acc_bg: Vec4f,
    #[rust(Glyph::Info)]
    glyph: Glyph,
    #[rust(Tone::Neutral)]
    tone: Tone,
}

impl Widget for OdGlyph {
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}

    fn draw_walk(&mut self, cx: &mut Cx2d, _scope: &mut Scope, walk: Walk) -> DrawStep {
        if !self.visible {
            return DrawStep::done();
        }
        let rect = cx.walk_turtle(walk);
        let (fg, bg) = match self.tone {
            Tone::Positive => (self.pos_fg, self.pos_bg),
            Tone::Caution => (self.cau_fg, self.cau_bg),
            Tone::Critical => (self.cri_fg, self.cri_bg),
            Tone::Neutral => (self.neu_fg, self.neu_bg),
            Tone::Progress => (self.pro_fg, self.pro_bg),
            Tone::Accent => (self.acc_fg, self.acc_bg),
        };
        self.draw_glyph.fg = fg;
        self.draw_glyph.bg = bg;
        self.draw_glyph.glyph = glyph_id(self.glyph);
        self.draw_glyph.draw_abs(cx, rect);
        DrawStep::done()
    }
}

impl OdGlyphRef {
    pub fn set(&self, cx: &mut Cx, tone: Tone, glyph: Glyph) {
        if let Some(mut inner) = self.borrow_mut() {
            if inner.tone != tone || inner.glyph != glyph {
                inner.tone = tone;
                inner.glyph = glyph;
                inner.redraw(cx);
            }
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct OdMeter {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[walk]
    walk: Walk,
    #[live(true)]
    #[visible]
    visible: bool,
    #[redraw]
    #[live]
    draw_meter: DrawOdMeter,
    #[live]
    track: Vec4f,
    #[live]
    accent: Vec4f,
    #[live]
    caution: Vec4f,
    #[live]
    critical: Vec4f,
    #[live]
    neutral: Vec4f,
    #[rust]
    fraction: Option<f32>,
    #[rust(Tone::Neutral)]
    tone: Tone,
}

impl Widget for OdMeter {
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}

    fn draw_walk(&mut self, cx: &mut Cx2d, _scope: &mut Scope, walk: Walk) -> DrawStep {
        if !self.visible {
            return DrawStep::done();
        }
        let rect = cx.walk_turtle(walk);
        self.draw_meter.track = self.track;
        self.draw_meter.fg = match self.tone {
            Tone::Critical => self.critical,
            Tone::Caution => self.caution,
            Tone::Neutral => self.neutral,
            _ => self.accent,
        };
        self.draw_meter.unknown = if self.fraction.is_none() { 1.0 } else { 0.0 };
        self.draw_meter.fraction = self.fraction.unwrap_or(0.0);
        self.draw_meter.draw_abs(cx, rect);
        DrawStep::done()
    }
}

impl OdMeterRef {
    pub fn set(&self, cx: &mut Cx, fraction: Option<f32>, tone: Tone) {
        if let Some(mut inner) = self.borrow_mut() {
            if inner.fraction != fraction || inner.tone != tone {
                inner.fraction = fraction;
                inner.tone = tone;
                inner.redraw(cx);
            }
        }
    }
}

struct ListItem {
    key: String,
    template: LiveId,
    widget: WidgetRef,
}

/// Keyed list of template rows. [`OdListRef::sync`] keeps one widget per
/// (key, template), in the given order; unknown keys are dropped.
#[derive(Script, WidgetRegister, WidgetRef, WidgetSet)]
pub struct OdList {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[walk]
    walk: Walk,
    #[layout]
    layout: Layout,
    #[live(true)]
    visible: bool,
    #[rust]
    area: Area,
    #[rust]
    templates: HashMap<LiveId, ScriptObjectRef>,
    #[rust]
    items: Vec<ListItem>,
}

impl ScriptHook for OdList {
    fn on_before_apply(&mut self, _vm: &mut ScriptVm, apply: &Apply, _scope: &mut Scope, _value: ScriptValue) {
        if apply.is_reload() {
            self.templates.clear();
        }
    }

    fn on_after_apply(&mut self, vm: &mut ScriptVm, apply: &Apply, scope: &mut Scope, value: ScriptValue) {
        if !apply.is_eval() {
            if let Some(obj) = value.as_object() {
                vm.vec_with(obj, |vm, vec| {
                    for kv in vec {
                        if let Some(id) = kv.key.as_id() {
                            if let Some(template) = kv.value.as_object() {
                                self.templates.insert(id, vm.bx.heap.new_object_ref(template));
                            }
                        }
                    }
                });
            }
            // A theme/style reapply restyles existing rows in place.
            if !apply.is_new() {
                for item in &mut self.items {
                    if let Some(template) = self.templates.get(&item.template) {
                        let value: ScriptValue = template.as_object().into();
                        item.widget.script_apply(vm, apply, scope, value);
                    }
                }
            }
        }
    }
}

impl WidgetNode for OdList {
    fn widget_uid(&self) -> WidgetUid { self.uid }
    fn walk(&mut self, _cx: &mut Cx) -> Walk { self.walk }
    fn area(&self) -> Area { self.area }
    fn redraw(&mut self, cx: &mut Cx) { self.area.redraw(cx); }
    fn visible(&self) -> bool { self.visible }
    fn set_visible(&mut self, cx: &mut Cx, value: bool) { self.visible = value; self.redraw(cx); }
    fn children(&self, visit: &mut dyn FnMut(LiveId, WidgetRef)) {
        for item in &self.items { visit(LiveId::from_str(&item.key), item.widget.clone()); }
    }
    fn find_widgets_from_point(&self, cx: &Cx, point: DVec2, found: &mut dyn FnMut(&WidgetRef)) {
        if self.visible { for item in &self.items { item.widget.find_widgets_from_point(cx, point, found); } }
    }
}

impl Widget for OdList {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if !self.visible && event.requires_visibility() {
            return;
        }
        for item in &self.items {
            item.widget.handle_event(cx, event, scope);
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        if !self.visible {
            return DrawStep::done();
        }
        cx.begin_turtle(walk, self.layout);
        for item in &self.items {
            item.widget.draw_all(cx, scope);
        }
        cx.end_turtle_with_area(&mut self.area);
        DrawStep::done()
    }
}

impl OdList {
    fn sync(&mut self, cx: &mut Cx, wanted: &[(String, LiveId)]) -> Vec<WidgetRef> {
        let mut old: HashMap<(String, LiveId), WidgetRef> =
            self.items.drain(..).map(|i| ((i.key, i.template), i.widget)).collect();
        let mut out = Vec::with_capacity(wanted.len());
        for (key, template) in wanted {
            let widget = match old.remove(&(key.clone(), *template)) {
                Some(widget) => widget,
                None => {
                    let Some(template_ref) = self.templates.get(template) else {
                        warning!("OdList template not found: {template}");
                        continue;
                    };
                    let value: ScriptValue = template_ref.as_object().into();
                    let widget = cx.with_vm(|vm| WidgetRef::script_from_value(vm, value));
                    cx.widget_tree_insert_child(self.uid, LiveId::from_str(key), widget.clone());
                    widget
                }
            };
            out.push(widget.clone());
            self.items.push(ListItem { key: key.clone(), template: *template, widget });
        }
        self.area.redraw(cx);
        out
    }
}

impl OdListRef {
    /// Ensure rows for `wanted` in order and return them (same order).
    pub fn sync(&self, cx: &mut Cx, wanted: &[(String, LiveId)]) -> Vec<WidgetRef> {
        self.borrow_mut().map(|mut list| list.sync(cx, wanted)).unwrap_or_default()
    }

    /// Current rows with their keys, for routing child actions.
    pub fn rows(&self) -> Vec<(String, WidgetRef)> {
        self.borrow().map(|l| l.items.iter().map(|i| (i.key.clone(), i.widget.clone())).collect()).unwrap_or_default()
    }
}
