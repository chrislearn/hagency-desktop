-- Task #12: pending invitations to agents (TS bridge-matrix.js
-- state.pendingInvites, 2391-2455, + backend-v2.js:10813-10855 routes).
-- One row per (room, agent). 'declined' is REMEMBERED so the invite poll
-- cannot resurrect it; 'accepted' records that the join has happened.
-- `inviter` is NULL when the invite state named no sender — surfaced,
-- never guessed, because the inviter IS the owner (ADR-002) and inventing
-- one would forge ownership. `mode` is the TS binding's direct/group
-- (from is_direct); `since_ts` is the admission boundary the TS
-- direct-rooms POST records.
-- `join_pending` splits 'accepted' into DECIDED vs JOINED: the console
-- records the decision, the invite poll performs the join and clears the
-- flag — so a refused join is retried next round (ADR-183: never
-- terminal) instead of being lost after one attempt.
-- `leave_pending` is the decline's mirror: leaving is best-effort in TS
-- (`bridge-matrix.js:9135-9147`), so the record outlives the wire act and
-- the poll retries the leave until the homeserver answers.
CREATE TABLE pending_invites (
    room_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    inviter TEXT,
    project_server TEXT NOT NULL,
    mode TEXT NOT NULL CHECK(mode IN ('direct','group')),
    since_ts INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','accepted','declined')),
    join_pending INTEGER NOT NULL DEFAULT 0,
    leave_pending INTEGER NOT NULL DEFAULT 0,
    seen_at INTEGER NOT NULL,
    decided_at INTEGER,
    decided_by TEXT,
    PRIMARY KEY(room_id,agent)
) STRICT;
CREATE INDEX pending_invite_decisions ON pending_invites(state, seen_at);
