//! ADR-187 §C: owner anchors, trusted on first use (amends ADR-102 for
//! imported fleets).
//!
//! The first master key Hagency observes for an owner is pinned. Every later
//! observation is compared with the pin: the same key is fine; a different
//! key is recorded as a mismatch for the operator and refused, never adopted.
//! Only an operator re-pin changes the anchor.
use crate::Error;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerAnchor {
    pub owner_mxid: String,
    pub master_key: String,
    pub source: String,
    pub pinned_at: u64,
    /// A different key the homeserver reported after the pin, awaiting the
    /// operator. Never trusted.
    pub mismatch_key: Option<String>,
    pub mismatch_at: Option<u64>,
}

/// An unpadded base64 Ed25519 public key, as Matrix key queries carry it.
fn valid_key(key: &str) -> bool {
    key.len() == 43
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}
fn valid_owner(owner: &str) -> bool {
    owner.len() <= 255
        && owner.starts_with('@')
        && owner.contains(':')
        && !owner.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub(super) fn get(db: &Connection, owner: &str) -> Result<Option<OwnerAnchor>, Error> {
    Ok(db
        .query_row(
            "SELECT owner_mxid,master_key,source,pinned_at,mismatch_key,mismatch_at FROM owner_anchors WHERE owner_mxid=?1",
            [owner],
            |r| {
                Ok(OwnerAnchor {
                    owner_mxid: r.get(0)?,
                    master_key: r.get(1)?,
                    source: r.get(2)?,
                    pinned_at: r.get(3)?,
                    mismatch_key: r.get(4)?,
                    mismatch_at: r.get(5)?,
                })
            },
        )
        .optional()?)
}

pub(super) fn list(db: &Connection) -> Result<Vec<OwnerAnchor>, Error> {
    let mut statement =
        db.prepare("SELECT owner_mxid FROM owner_anchors ORDER BY owner_mxid LIMIT 1000")?;
    let owners: Vec<String> = statement
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    owners
        .iter()
        .map(|owner| get(db, owner).map(|a| a.expect("listed row exists")))
        .collect()
}

/// The key the homeserver reports now, against the pin: pin it if it is the
/// first, accept it if it matches, otherwise record the mismatch and refuse.
pub(super) fn observe(
    db: &Connection,
    owner: &str,
    key: &str,
    now: u64,
) -> Result<OwnerAnchor, Error> {
    if !valid_owner(owner) || !valid_key(key) {
        return Err(hagency_core::InvalidInput("owner anchor").into());
    }
    match get(db, owner)? {
        None => {
            db.execute(
                "INSERT INTO owner_anchors(owner_mxid,master_key,source,pinned_at) VALUES(?1,?2,'first_use',?3)",
                params![owner, key, now],
            )?;
        }
        Some(pinned) if pinned.master_key == key => {}
        Some(_) => {
            db.execute(
                "UPDATE owner_anchors SET mismatch_key=?2,mismatch_at=?3 WHERE owner_mxid=?1 AND (mismatch_key IS NOT ?2)",
                params![owner, key, now],
            )?;
            return Err(Error::Conflict);
        }
    }
    get(db, owner)?.ok_or(Error::NotFound)
}

/// The operator re-pins an owner's anchor (after the owner reset their
/// cross-signing): the new key replaces the pin and clears the mismatch.
pub(super) fn repin(
    db: &Connection,
    owner: &str,
    key: &str,
    now: u64,
) -> Result<OwnerAnchor, Error> {
    if !valid_owner(owner) || !valid_key(key) {
        return Err(hagency_core::InvalidInput("owner anchor").into());
    }
    db.execute(
        "INSERT INTO owner_anchors(owner_mxid,master_key,source,pinned_at) VALUES(?1,?2,'operator',?3) \
         ON CONFLICT(owner_mxid) DO UPDATE SET master_key=?2,source='operator',pinned_at=?3,mismatch_key=NULL,mismatch_at=NULL",
        params![owner, key, now],
    )?;
    get(db, owner)?.ok_or(Error::NotFound)
}
