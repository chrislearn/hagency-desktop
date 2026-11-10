fn main() {
    std::process::exit(hagency::daemon::cli::run_if_requested().unwrap_or(2));
}
