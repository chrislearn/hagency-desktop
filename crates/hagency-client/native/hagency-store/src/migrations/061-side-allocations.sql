-- The operator's per-side token allocation (「真配额」, backend-v2.js:9541):
-- one nullable allocation per registered side. NULL is UNALLOCATED, which is
-- not unlimited (lib/project-side-store.js:443-452): a side with no
-- allocation refuses new work rather than drawing on whatever is left. Zero
-- is a legitimate value — the side is allocated nothing — and is NOT null.
-- The figure lives outside the registration config JSON so it can be set and
-- read without re-writing or parsing the whole registration; the fleet_id is
-- the side id (ADR-016: the id IS the server name).
-- Replay posture (024/034): recovery fixtures rewind user_version below this
-- step over a database the current build already opened, so the CREATE is
-- idempotent; the verify probe still refuses a wrongly-shaped table.
CREATE TABLE IF NOT EXISTS side_allocations (
    fleet_id TEXT PRIMARY KEY REFERENCES registrations(fleet_id),
    allocated_tokens INTEGER CHECK(allocated_tokens IS NULL OR allocated_tokens >= 0),
    updated_at INTEGER NOT NULL
) STRICT;
