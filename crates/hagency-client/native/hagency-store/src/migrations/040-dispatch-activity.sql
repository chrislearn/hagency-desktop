-- Task #1: the activity notice state (TS router/src/activity.ts:1-30,
-- RUNNER_ACTIVITY_SCHEMA), under this store's names.
--
-- One row per dispatch. `phase` is the TS vocabulary exactly
-- (started|heartbeat|waiting|resumed|completed|interrupted|tool_start|
-- tool_end); `kind` is the tool kind while a tool phase is current.
-- `revision` counts DELIVERED-eligible bodies only — it is the number in
-- the notice id (`activity:<dispatch>:<revision>`), so a superseded
-- projection and its successor can never collide. `queued_at` is the
-- coalescing window's origin (TS activity.ts:56-62: 30 s heartbeat,
-- 5 s otherwise, immediate for lifecycle phases and the first tool).
-- `anchor` is the FIRST delivered activity event id — the event every
-- later revision edits in place (activity.ts:76-78, COALESCE semantics).
CREATE TABLE dispatch_activity (
    dispatch_id TEXT PRIMARY KEY REFERENCES runner_dispatches(id),
    phase TEXT NOT NULL CHECK(phase IN
        ('started','heartbeat','waiting','resumed','completed','interrupted','tool_start','tool_end')),
    kind TEXT CHECK(kind IS NULL OR kind IN ('command','files','search','delegate','tool')),
    tools INTEGER NOT NULL DEFAULT 0,
    finished INTEGER NOT NULL DEFAULT 0,
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    queued_at INTEGER NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    anchor TEXT
) STRICT;

-- Dedupe of runner tool events (TS runner_activity_events): a repeated
-- event key counts nothing; `tool_end` requires its `tool_start` row.
CREATE TABLE dispatch_activity_events (
    dispatch_id TEXT NOT NULL REFERENCES dispatch_activity(dispatch_id),
    event_key TEXT NOT NULL,
    PRIMARY KEY(dispatch_id, event_key)
) STRICT;
