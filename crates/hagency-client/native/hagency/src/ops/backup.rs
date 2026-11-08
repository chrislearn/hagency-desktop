//! `hagency backup` and `hagency restore`.
//!
//! The mechanism lives in the store (`hagency_store::backup`) because that is
//! where the SQLite online backup belongs; this module is the operator-facing
//! framing — the terminal verbs, their preconditions, and the receipt they
//! print.

use hagency_store::{Error, backup, private};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn now_ms() -> Result<u64, Error> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::State)?
        .as_millis() as u64)
}

/// Capture an initialized state directory into a new private directory.
///
/// A precondition of an initialized state (the operator token every other
/// offline verb requires) keeps `backup` from writing a snapshot of an empty
/// or half-provisioned directory. It is deliberately the ONLY precondition:
/// the running service holds the store's own lock, and the snapshot is taken
/// through it, not around it, so this runs while `serve` is up.
pub fn snapshot(state: &Path, out: &Path) -> Result<backup::Manifest, Error> {
    private::read_secret(&state.join("operator.token"))?;
    backup::snapshot(state, out, now_ms()?)
}

/// Restore a snapshot into an empty state directory. No operator-token
/// precondition: the snapshot being restored is what carries the token, so
/// requiring one first would make a first restore impossible.
pub fn restore(state: &Path, from: &Path) -> Result<backup::Manifest, Error> {
    backup::restore(state, from)
}
