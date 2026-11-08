//! Native owner management reuses the authenticated client service in process.
pub use crate::console::native::*;

/// Private process-custody entry. The guardian validates its inherited handles;
/// this does not launch a provider or accept owner commands from argv.
pub fn run_guardian_if_requested() -> Option<std::io::Result<()>> {
    #[cfg(unix)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("guardian")) {
        return Some(hagency_platform::run_guardian());
    }
    None
}
