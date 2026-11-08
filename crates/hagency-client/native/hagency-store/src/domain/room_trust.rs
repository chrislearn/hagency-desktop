//! Per-room trust state (task #80): the TS `bridge-matrix.js` `markRoomTrusted`
//! (`state.trustedManagedRooms[roomId]`) parity record. A room the service marked
//! trusted (e.g. after its inviter passed the trusted-inviter gate) is persisted
//! here so the classifier returns reason 'managed' for a room that is neither a
//! frozen target (`HostConfig.rooms`) nor operator-allowlisted.
use super::DomainRepository;
use crate::Error;
use rusqlite::{OptionalExtension, params};

/// One row of the `room_trust` table: a room the service marked trusted.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RoomTrustRecord {
    pub room_id: String,
    /// JSON document (group/owner/inviter), opaque to the store.
    pub meta: String,
    pub added_at: u64,
}

/// The projection reads at most this many rows per call; the table is bounded
/// by the rooms a single fleet can be in.
const ROOM_TRUST_LIMIT: usize = 512;

impl DomainRepository {
    /// Read every recorded trusted room (bounded). The matrix classifier merges
    /// these with the frozen targets and the operator allowlist.
    pub fn room_trust_records(&self) -> Result<Vec<RoomTrustRecord>, Error> {
        let mut statement = self.db.prepare(
            "SELECT room_id, meta, added_at FROM room_trust ORDER BY added_at DESC LIMIT ?1",
        )?;
        let limit = i64::try_from(ROOM_TRUST_LIMIT).map_err(|_| Error::State)?;
        let rows = statement.query_map([limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (room_id, meta, added_at) = row?;
            out.push(RoomTrustRecord {
                room_id,
                meta,
                added_at: u64::try_from(added_at).map_err(|_| Error::State)?,
            });
        }
        Ok(out)
    }

    /// TS `markRoomTrusted` parity: idempotently record a room as trusted
    /// (`state.trustedManagedRooms[roomId] = {...meta, addedAt}`; no-op if present).
    /// The caller's `now` is the wall clock. Returns true when the row was created.
    pub fn mark_room_trusted(
        &mut self,
        room_id: &str,
        meta: &str,
        now: u64,
    ) -> Result<bool, Error> {
        hagency_core::tasks::text(room_id, 512)?;
        let existing: Option<i64> = self
            .db
            .query_row(
                "SELECT 1 FROM room_trust WHERE room_id=?1",
                [room_id],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(false);
        }
        let added_at = i64::try_from(now).map_err(|_| Error::State)?;
        self.db.execute(
            "INSERT INTO room_trust (room_id, meta, added_at) VALUES (?1, ?2, ?3)",
            params![room_id, meta, added_at],
        )?;
        Ok(true)
    }

    /// Read the trust record for one room, or None (the unknown_room arm).
    pub fn room_trust_record(&self, room_id: &str) -> Result<Option<RoomTrustRecord>, Error> {
        let row: Option<(String, String, i64)> = self
            .db
            .query_row(
                "SELECT room_id, meta, added_at FROM room_trust WHERE room_id=?1",
                [room_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((room_id, meta, added_at)) => Ok(Some(RoomTrustRecord {
                room_id,
                meta,
                added_at: u64::try_from(added_at).map_err(|_| Error::State)?,
            })),
        }
    }
}
