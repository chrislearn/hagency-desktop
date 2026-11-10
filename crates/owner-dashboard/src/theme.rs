//! Dashboard design tokens derived from the host's shared Makepad theme
//! (ADR 0009 roles in `mod.theme`). The dashboard owns no palette: it
//! follows Hagency's installed stylesheet through the same roles.
//!
//! Registered as `mod.widgets.OD_*` before the widget DSL. Re-running
//! [`crate::script_mod`] on a theme reload recomputes them; widgets then
//! re-sync their imperative state, so drafts, focus and scroll survive.
use makepad_widgets::*;

/// Host-controlled text enlargement on top of the theme's typography scale
/// (accessibility settings, fixtures). 1.0 = theme default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DashboardTextScale(pub f64);
impl Default for DashboardTextScale {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Scale actually applied to OD typography, including the host stylesheet.
#[derive(Clone, Copy, Debug)]
pub struct DashboardEffectiveTextScale(pub f64);
impl Default for DashboardEffectiveTextScale { fn default() -> Self { Self(1.0) } }
pub fn effective_text_scale(cx: &mut Cx) -> f64 { cx.global::<DashboardEffectiveTextScale>().0 }

pub fn set_text_scale(cx: &mut Cx, scale: f64) {
    cx.global::<DashboardTextScale>().0 = scale.clamp(0.75, 3.0);
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub page: Vec4f,
    pub surface: Vec4f,
    pub field: Vec4f,
    pub ink: Vec4f,
    pub muted: Vec4f,
    pub separator: Vec4f,
    pub accent: Vec4f,
    pub accent_fill: Vec4f,
    pub on_accent: Vec4f,
    pub accent_hover: Vec4f,
    pub accent_down: Vec4f,
    pub hover: Vec4f,
    pub pressed: Vec4f,
    pub selected: Vec4f,
    pub track: Vec4f,
    pub positive: (Vec4f, Vec4f),
    pub caution: (Vec4f, Vec4f),
    pub critical: (Vec4f, Vec4f),
    pub neutral: (Vec4f, Vec4f),
    pub progress: (Vec4f, Vec4f),
    pub radius: f64,
    pub text_scale: f64,
    pub control_height: f64,
    pub motion: f64,
}

fn rgb(value: u32) -> Vec4f {
    vec4((value >> 16 & 255) as f32 / 255., (value >> 8 & 255) as f32 / 255., (value & 255) as f32 / 255., 1.)
}
fn mix(a: Vec4f, b: Vec4f, t: f32) -> Vec4f {
    a * (1. - t) + b * t
}
pub fn luminance(c: Vec4f) -> f32 {
    let l = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * l(c.x) + 0.7152 * l(c.y) + 0.0722 * l(c.z)
}
pub fn contrast(a: Vec4f, b: Vec4f) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
/// Moves `fg` toward `toward` until it reaches `ratio` against every `bg`.
fn ensure(fg: Vec4f, bgs: &[Vec4f], toward: Vec4f, ratio: f32) -> Vec4f {
    let mut out = fg;
    for step in 0..=20 {
        out = mix(fg, toward, step as f32 / 20.);
        if bgs.iter().all(|bg| contrast(out, *bg) >= ratio) {
            break;
        }
    }
    out
}

/// Reads theme roles with safe fallbacks and derives the token set.
pub fn tokens_for_vm(vm: &mut ScriptVm) -> Tokens {
    let theme = vm.module(id!(theme));
    let color = |vm: &mut ScriptVm, name: &str| -> Option<Vec4f> {
        vm.bx.heap.value(theme, LiveId::from_str(name).into(), NoTrap).as_color().map(|rgba| {
            vec4(
                (rgba >> 24) as f32 / 255.,
                (rgba >> 16 & 255) as f32 / 255.,
                (rgba >> 8 & 255) as f32 / 255.,
                (rgba & 255) as f32 / 255.,
            )
        })
    };
    let number = |vm: &mut ScriptVm, name: &str| vm.bx.heap.value(theme, LiveId::from_str(name).into(), NoTrap).as_number();
    let page = color(vm, "color_bg_app").unwrap_or(rgb(0xf2f2f7));
    let dark = luminance(page) < 0.5;
    let surface = color(vm, "color_bg_container").unwrap_or(if dark { rgb(0x1c1c1e) } else { rgb(0xffffff) });
    let surface = if contrast(surface, page) < 1.04 {
        // Keep grouped cards distinguishable from the page.
        if dark { mix(page, rgb(0xffffff), 0.07) } else { rgb(0xffffff) }
    } else {
        surface
    };
    let ink = color(vm, "color_text").unwrap_or(if dark { rgb(0xf2f2f7) } else { rgb(0x1c1c1e) });
    let ink = ensure(ink, &[page, surface], if dark { rgb(0xffffff) } else { rgb(0) }, 7.0);
    let field = color(vm, "color_inset").unwrap_or(mix(surface, ink, 0.06));
    let muted = color(vm, "octo_color_content_secondary").unwrap_or(mix(ink, surface, 0.32));
    let muted = ensure(muted, &[page, surface, field], ink, 4.5);
    let border = color(vm, "color_bevel_outset_2").unwrap_or(mix(surface, ink, 0.18));
    let separator = mix(border, surface, 0.35);
    let accent = color(vm, "color_focus").unwrap_or(if dark { rgb(0x0a84ff) } else { rgb(0x0a64d6) });
    let on_accent = color(vm, "color_text_on_accent").unwrap_or(rgb(0xffffff));
    let away = if luminance(on_accent) > 0.5 { rgb(0) } else { rgb(0xffffff) };
    // Accent used as text on cards must stay readable.
    let accent_text = ensure(accent, &[surface, field], ink, 4.5);
    let accent_fill = ensure(accent, &[on_accent], away, 4.5);
    let status = |vm: &mut ScriptVm, role: &str, light: u32, dark_fg: u32| -> (Vec4f, Vec4f) {
        let fg = color(vm, &format!("octo_color_status_{role}_foreground")).unwrap_or(rgb(if dark { dark_fg } else { light }));
        let bg = color(vm, &format!("octo_color_status_{role}_background")).unwrap_or(mix(surface, fg, if dark { 0.20 } else { 0.12 }));
        (ensure(fg, &[surface, bg], ink, 4.5), bg)
    };
    let positive = status(vm, "success", 0x1d7a36, 0x4cd964);
    let caution = status(vm, "warning", 0x8a5300, 0xffb340);
    let critical = status(vm, "danger", 0xb42318, 0xff6961);
    let neutral = (muted, mix(surface, ink, 0.07));
    let progress = (accent_text, mix(surface, accent, if dark { 0.22 } else { 0.12 }));
    let radius = number(vm, "corner_radius").unwrap_or(6.0);
    let host_scale = number(vm, "octo_typography_scale").unwrap_or(1.0);
    let extra = vm.cx_mut().global::<DashboardTextScale>().0;
    let text_scale = (host_scale * extra).clamp(0.75, 3.0);
    Tokens {
        dark,
        page,
        surface,
        field,
        ink,
        muted,
        separator,
        accent: accent_text,
        accent_fill,
        on_accent,
        accent_hover: ensure(mix(accent_fill, away, 0.08), &[on_accent], away, 4.5),
        accent_down: ensure(mix(accent_fill, away, 0.16), &[on_accent], away, 4.5),
        hover: mix(field, ink, 0.06),
        pressed: mix(field, ink, 0.12),
        selected: mix(surface, accent, if dark { 0.24 } else { 0.12 }),
        track: mix(surface, ink, if dark { 0.18 } else { 0.10 }),
        positive,
        caution,
        critical,
        neutral,
        progress,
        // iOS-style grouped cards: rounder than controls, derived from the theme.
        radius: (radius * 2.0).clamp(8.0, 14.0),
        text_scale,
        control_height: number(vm, "octo_control_height").unwrap_or(44.0).max(44.0).max(24.0 * text_scale + 20.0),
        motion: number(vm, "octo_motion_ms").unwrap_or(150.0) / 1000.0,
    }
}

/// Publishes `mod.widgets.OD_*`. Called by [`crate::script_mod`].
pub fn script_mod(vm: &mut ScriptVm) {
    let t = tokens_for_vm(vm);
    let s = t.text_scale;
    vm.cx_mut().global::<DashboardEffectiveTextScale>().0 = s;
    script_eval!(vm, {
        mod.widgets.OD_DARK = #(t.dark)
        mod.widgets.OD_PAGE = #(t.page)
        mod.widgets.OD_SURFACE = #(t.surface)
        mod.widgets.OD_FIELD = #(t.field)
        mod.widgets.OD_INK = #(t.ink)
        mod.widgets.OD_MUTED = #(t.muted)
        mod.widgets.OD_SEPARATOR = #(t.separator)
        mod.widgets.OD_ACCENT = #(t.accent)
        mod.widgets.OD_ACCENT_FILL = #(t.accent_fill)
        mod.widgets.OD_ON_ACCENT = #(t.on_accent)
        mod.widgets.OD_ACCENT_HOVER = #(t.accent_hover)
        mod.widgets.OD_ACCENT_DOWN = #(t.accent_down)
        mod.widgets.OD_HOVER = #(t.hover)
        mod.widgets.OD_PRESSED = #(t.pressed)
        mod.widgets.OD_SELECTED = #(t.selected)
        mod.widgets.OD_TRACK = #(t.track)
        mod.widgets.OD_POS_FG = #(t.positive.0)
        mod.widgets.OD_POS_BG = #(t.positive.1)
        mod.widgets.OD_CAU_FG = #(t.caution.0)
        mod.widgets.OD_CAU_BG = #(t.caution.1)
        mod.widgets.OD_CRI_FG = #(t.critical.0)
        mod.widgets.OD_CRI_BG = #(t.critical.1)
        mod.widgets.OD_NEU_FG = #(t.neutral.0)
        mod.widgets.OD_NEU_BG = #(t.neutral.1)
        mod.widgets.OD_PRO_FG = #(t.progress.0)
        mod.widgets.OD_PRO_BG = #(t.progress.1)
        mod.widgets.OD_RADIUS = #(t.radius)
        mod.widgets.OD_RADIUS_SMALL = #(t.radius * 0.75)
        mod.widgets.OD_CONTROL = #(t.control_height)
        mod.widgets.OD_COMPACT_CONTROL = #((18.0 * s + 8.0).max(44.0))
        mod.widgets.OD_MOTION = #(t.motion)
        mod.widgets.OD_TEXT_SCALE = #(s)
        mod.widgets.OD_SIZE_LARGE = #(19.0 * s)
        mod.widgets.OD_SIZE_TITLE = #(14.0 * s)
        mod.widgets.OD_SIZE_HEAD = #(12.0 * s)
        mod.widgets.OD_SIZE_BODY = #(11.0 * s)
        mod.widgets.OD_SIZE_CAPTION = #(9.5 * s)
        mod.widgets.OD_SIZE_METRIC = #(17.0 * s)
        mod.widgets.OD_GLYPH = #(20.0 * s.sqrt())
    });
}
