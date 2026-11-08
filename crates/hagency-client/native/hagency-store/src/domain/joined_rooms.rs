//! ADR-188: rooms an agent joined by invitation.
//!
//! The rooms an agent was created with (its DM and its project room) stay in
//! its host configuration and store binding. A room it joins later is recorded
//! here, and the agent's driver reads these rows on every pass, so a restart
//! loses nothing and no store binding changes.
use crate::Error;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinedRoomState {
    /// The agent reads and answers there.
    Working,
    /// Joined, not working: encrypted with other people in it (ADR-188 §3).
    EncryptedShared,
    /// Left, removed or declined.
    Retired,
}
impl JoinedRoomState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::EncryptedShared => "encrypted_shared",
            Self::Retired => "retired",
        }
    }
    fn parse(value: &str) -> Result<Self, Error> {
        Ok(match value {
            "working" => Self::Working,
            "encrypted_shared" => Self::EncryptedShared,
            "retired" => Self::Retired,
            _ => return Err(hagency_core::InvalidInput("joined room state").into()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinedRoom {
    pub engagement_id: String,
    pub room_id: String,
    pub state: JoinedRoomState,
    pub joined_at: u64,
    pub updated_at: u64,
    /// When the one "can't work here" notice was posted, if it was.
    pub notice_at: Option<u64>,
}

/// An engagement may hold at most this many live joined rooms: the agent's
/// claim profile carries every working room, and it is bounded.
pub const MAX_JOINED_ROOMS: usize = 12;

fn valid_room(room: &str) -> bool {
    room.len() <= 255
        && room.starts_with('!')
        && room.contains(':')
        && !room.chars().any(|c| c.is_control() || c.is_whitespace())
}
fn valid_engagement(engagement: &str) -> bool {
    !engagement.is_empty() && engagement.len() <= 128
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String, u64, u64, Option<u64>)> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    ))
}
fn decode(
    (engagement_id, room_id, state, joined_at, updated_at, notice_at): (
        String,
        String,
        String,
        u64,
        u64,
        Option<u64>,
    ),
) -> Result<JoinedRoom, Error> {
    Ok(JoinedRoom {
        engagement_id,
        room_id,
        state: JoinedRoomState::parse(&state)?,
        joined_at,
        updated_at,
        notice_at,
    })
}

pub(super) fn get(
    db: &Connection,
    engagement: &str,
    room: &str,
) -> Result<Option<JoinedRoom>, Error> {
    db.query_row(
        "SELECT engagement_id,room_id,state,joined_at,updated_at,notice_at FROM joined_rooms WHERE engagement_id=?1 AND room_id=?2",
        params![engagement, room],
        row,
    )
    .optional()?
    .map(decode)
    .transpose()
}

/// The engagement's joined rooms that are not retired, oldest first.
pub(super) fn live(db: &Connection, engagement: &str) -> Result<Vec<JoinedRoom>, Error> {
    let mut statement = db.prepare(
        "SELECT engagement_id,room_id,state,joined_at,updated_at,notice_at FROM joined_rooms WHERE engagement_id=?1 AND state<>'retired' ORDER BY joined_at,room_id LIMIT 64",
    )?;
    let rows = statement
        .query_map([engagement], row)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(decode).collect()
}

/// A join the agent made: record the room as working. Joining a room again
/// after it was retired makes it working again, with the notice cleared.
pub(super) fn record(
    db: &Connection,
    engagement: &str,
    room: &str,
    now: u64,
) -> Result<JoinedRoom, Error> {
    if !valid_engagement(engagement) || !valid_room(room) {
        return Err(hagency_core::InvalidInput("joined room").into());
    }
    let existing = get(db, engagement, room)?;
    if existing
        .as_ref()
        .is_none_or(|r| r.state == JoinedRoomState::Retired)
        && live(db, engagement)?.len() >= MAX_JOINED_ROOMS
    {
        return Err(Error::Capacity);
    }
    match existing {
        Some(r) if r.state != JoinedRoomState::Retired => return Ok(r),
        Some(_) => {
            db.execute(
                "UPDATE joined_rooms SET state='working',joined_at=?3,updated_at=?3,notice_at=NULL WHERE engagement_id=?1 AND room_id=?2",
                params![engagement, room, now],
            )?;
        }
        None => {
            db.execute(
                "INSERT INTO joined_rooms(engagement_id,room_id,state,joined_at,updated_at) VALUES(?1,?2,'working',?3,?3)",
                params![engagement, room, now],
            )?;
        }
    }
    get(db, engagement, room)?.ok_or(Error::Conflict)
}

/// Move a recorded room to `state`. Unknown rooms are refused; a retired room
/// stays retired (a new join records it again).
pub(super) fn set_state(
    db: &Connection,
    engagement: &str,
    room: &str,
    state: JoinedRoomState,
    now: u64,
) -> Result<JoinedRoom, Error> {
    let current = get(db, engagement, room)?.ok_or(Error::RunnerAuthority)?;
    if current.state == JoinedRoomState::Retired || current.state == state {
        return Ok(current);
    }
    db.execute(
        "UPDATE joined_rooms SET state=?3,updated_at=?4 WHERE engagement_id=?1 AND room_id=?2",
        params![engagement, room, state.as_str(), now],
    )?;
    get(db, engagement, room)?.ok_or(Error::Conflict)
}

/// ADR-188 §3, reminder: claim a repeat of the notice when it was last posted
/// at or before `not_before`. True only for the caller that moves it.
pub(super) fn claim_renotice(
    db: &Connection,
    engagement: &str,
    room: &str,
    now: u64,
    not_before: u64,
) -> Result<bool, Error> {
    Ok(db.execute(
        "UPDATE joined_rooms SET notice_at=?3 WHERE engagement_id=?1 AND room_id=?2 AND state='encrypted_shared' AND notice_at IS NOT NULL AND notice_at<=?4",
        params![engagement, room, now, not_before],
    )? == 1)
}

/// Claim the one "can't work here" notice: true only for the first caller
/// since the room was (re)joined.
pub(super) fn claim_notice(
    db: &Connection,
    engagement: &str,
    room: &str,
    now: u64,
) -> Result<bool, Error> {
    Ok(db.execute(
        "UPDATE joined_rooms SET notice_at=?3 WHERE engagement_id=?1 AND room_id=?2 AND notice_at IS NULL AND state='encrypted_shared'",
        params![engagement, room, now],
    )? == 1)
}
