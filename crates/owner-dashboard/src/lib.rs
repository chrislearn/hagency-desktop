//! Hagency owner hosting dashboard (ADR 0012, ADR-192).
//!
//! One Makepad widget, [`widget::OwnerDashboard`], mounted on the Hosting page
//! of Hagency Desktop's Agent mini-app. The host owns all IO: it feeds
//! snapshots, command results and its hosting setup state in, and performs
//! the semantic [`DashboardAction`]s the widget emits. [`transport`] provides
//! the nonblocking owner-daemon connector.
//!
//! Data contract: [`dto`] re-exports `hagency_metering::dashboard`.
pub use hagency_metering::dashboard as dto;

pub mod controls;
pub mod picker;
mod activity_text;
pub mod format;
pub mod date_range;
pub mod i18n;
pub mod keyboard;
pub mod accessibility;
pub mod model;
pub mod theme;
pub mod transport;
pub mod widget;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

pub use format::Clock;
pub use i18n::Locale;
pub use widget::{OwnerDashboard, OwnerDashboardRef, OwnerDashboardWidgetExt, OwnerDashboardWidgetRefExt};
pub use model::{
    CommandKind, CommandResult, CommandTicket, ConnectionStatus, DashboardAction, DashboardState, HostOptions,
    SetupNeed, SetupProgress, SetupView, Tab, ViewState,
};

/// Registers dashboard tokens, controls and the `OwnerDashboard` widget.
/// Call after `makepad_widgets::script_mod` and after the host installed its
/// theme/stylesheet; call again on theme reload so tokens follow the theme.
pub fn script_mod(vm: &mut makepad_widgets::ScriptVm) {
    theme::script_mod(vm);
    controls::script_mod(vm);
    picker::script_mod(vm);
    widget::script_mod(vm);
}

mod command_store;
mod private_files;
