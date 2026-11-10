-- ADR-188: rooms an agent joined by invitation, beyond the identity rooms it
-- was created with. One row per (engagement, room). The identity rooms stay
-- in the agent's host configuration and store binding; these are read from
-- here on every pass. `state`:
--   working           the agent reads and answers there;
--   encrypted_shared  joined, not working: encrypted with other people in it;
--   retired           left, removed or declined; kept for the record.
-- `notice_at` is when the one "can't work here" notice was posted.
CREATE TABLE joined_rooms (
 engagement_id TEXT NOT NULL,
 room_id TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('working','encrypted_shared','retired')),
 joined_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 notice_at INTEGER,
 PRIMARY KEY(engagement_id, room_id)
) STRICT;
