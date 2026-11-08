//! `hagency rotate` — mint a replacement for a locally-held credential.
//!
//! Parity source of truth: the audit's row 26 (`docs/parity/operator-cli-ops-2026-09-24.md:83`).
//! TS rotates the operator token by editing `API_TOKEN` in `.env` and
//! restarting; the audit records that Rust has none and that "losing the token
//! means a new state directory and re-registering everything". The token is
//! minted locally (`main.rs:220-224`), so this is a purely local act — no
//! homeserver, no rebuild.
//!
//! What is NOT here, on purpose. Row 27 (Matrix/appservice/Palpo credentials)
//! rotates by a STAGED reissue whose promotion requires `verify` against the
//! live homeserver (`backend-v2.js:10054-10124`); those bytes are server-issued
//! and hand-placed (`bootstrap/config.rs:589,646-648,676,769`,
//! `bootstrap/palpo.rs:47`), so a local rewrite would desynchronise the
//! homeserver's own copy. Row 28 (agent tokens) is CHANGED-ON-PURPOSE per
//! ADR-041: native keeps no persistent agent token to rotate — each dispatch
//! gets a fresh runner capability — and a managed account's Codex namespace is
//! re-minted by `account retire` + `account prepare`, which already needs no
//! rebuild.

use hagency_store::{Error, private};
use std::path::Path;

#[derive(clap::Subcommand)]
pub enum Command {
    /// Mint a new operator token over the existing one.
    ///
    /// A running service keeps the previous token until it restarts: `serve`
    /// reads the file once at startup and holds its digest (`lib.rs:51-63`),
    /// exactly as TS's "edit `API_TOKEN` and restart" does. The new token is
    /// not echoed — it is read from the file, like every other caller.
    OperatorToken,
}

/// The rotation receipt. `fingerprint` is a digest of the new token, so an
/// operator can confirm the rotation took effect without the token itself
/// entering a log, a shell history or a scrollback.
#[derive(serde::Serialize)]
pub struct Receipt {
    pub ok: bool,
    pub rotated: &'static str,
    pub fingerprint: String,
    /// When the running service begins using the new token.
    pub applies_to: &'static str,
}

fn digest(token: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(token)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn run(state: &Path, command: Command) -> Result<Receipt, Error> {
    // Require an initialized private state; never manufacture a replacement
    // token where there is no state to authorise.
    private::read_secret(&state.join("operator.token"))?;
    match command {
        Command::OperatorToken => {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|_| Error::Unavailable)?;
            let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            // Atomic replace: a crash leaves the old token or the new one,
            // never a truncated credential that no caller would accept.
            private::replace(&state.join("operator.token"), token.as_bytes())?;
            Ok(Receipt {
                ok: true,
                rotated: "operator_token",
                fingerprint: digest(token.as_bytes()),
                applies_to: "restart",
            })
        }
    }
}
