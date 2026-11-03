//! Shared window semantics and action routing for both native dashboard hosts.
//!
//! Call `after_event` after the host has delivered an event to its entire UI.
//! This composes the dashboard with surrounding host controls under one window
//! root. The native feature requires the tracked Makepad platform adapter patch.
use makepad_widgets::*;

#[cfg(feature = "native-accessibility")]
#[path = "accessibility/native.rs"]
mod native;
#[cfg(feature = "native-accessibility")]
pub use native::{after_event, diagnostics};

#[cfg(not(feature = "native-accessibility"))]
pub fn after_event(_root: &WidgetRef, _cx: &mut Cx, _event: &Event) {}
#[cfg(not(feature = "native-accessibility"))]
pub fn diagnostics(_cx: &mut Cx) -> serde_json::Value {
    serde_json::json!({"nativeAccessibility": false})
}

#[cfg(all(feature = "native-accessibility", feature = "fixtures", target_os = "macos"))]
pub use native::native_fixture_probe;

/// Custom host controls compose into the same per-window native tree.
#[derive(Clone)]
pub struct Control {
    pub label: String,
    pub selected: Option<bool>,
    pub disabled: bool,
}
#[derive(Clone, Copy)]
pub struct ControlAdapter {
    pub name: &'static str,
    pub describe: fn(&WidgetRef, &Cx) -> Option<Control>,
    pub activate: fn(&WidgetRef, &mut Cx),
}
pub fn register_control_adapter(cx: &mut Cx, adapter: ControlAdapter) {
    #[cfg(feature = "native-accessibility")]
    native::register_control_adapter(cx, adapter);
    #[cfg(not(feature = "native-accessibility"))]
    let _ = (cx, adapter);
}
