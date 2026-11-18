//! Run the production daemon under JIT with disposable writer custody.
//! The driver supplies a private test root; no installed authority is opened.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if !hagency::daemon::cli::requested(&args) {
        eprintln!("SKIP: this JIT fixture requires explicit daemon arguments and a test root");
        return Ok(());
    }
    let root = std::path::PathBuf::from(
        std::env::var_os("HAGENCY_DAEMON_CRYPTO_ROOT").ok_or("explicit test root required")?,
    );
    if !root.is_absolute() {
        return Err("absolute test root required".into());
    }
    hagency_store::private::directory(&root)?;
    let authority = root.join("jit-writer-authority");
    hagency_store::private::directory(&authority)?;
    hagency::test_support::initialize_resource_authority_at(&authority)?;
    let status = hagency::daemon::cli::run_if_requested().ok_or("daemon arguments required")?;
    std::process::exit(status);
}
