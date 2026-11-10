-- Task #80: per-room trust state (bridge-matrix.js `markRoomTrusted` parity,
-- ADR-054 frozen-target framing). The board's number is 073; the schema-head
-- walk (domain.rs version + sequential migrations) places it at the next list
-- version after 41, hence the file keeps 073 while the tuple carries 42.
--
-- TS `state.trustedManagedRooms[roomId]` records a room the bridge marked
-- trusted (e.g. after its inviter passed the trusted-inviter gate). Native
-- carries the same record so the classifier can return reason 'managed' for a
-- room that is neither a frozen target nor operator-allowlisted. `meta` is a
-- JSON document (group/owner/inviter), `added_at` the wall-clock stamp.
CREATE TABLE room_trust (
    room_id TEXT PRIMARY KEY,
    meta TEXT NOT NULL,
    added_at INTEGER NOT NULL
) STRICT;
