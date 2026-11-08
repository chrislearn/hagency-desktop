//! Offline process-custody helper for local runtime regression tests.
fn main() {
    #[cfg(unix)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("guardian")) {
        if hagency_platform::run_guardian().is_err() {
            std::process::exit(1);
        }
        return;
    }
    std::process::exit(2);
}
