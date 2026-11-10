-- #27: workspace dirty-release receipt and the per-agent execution policy.
--
-- REPLAY POSTURE (the 032/033 rule): no ADD COLUMN in this store replays over
-- an already-upgraded table, and the recovery fixtures rewind `user_version`
-- and replay the chain from there. So this migration adds NO columns to an
-- existing table: both facts get their own new table behind CREATE TABLE IF
-- NOT EXISTS, which replays as a no-op.
--
-- Dirty release (TS parity: router/src/store.ts:3297 clearWorkspaceDirty). The
-- dirty flag itself already exists (003-task-dispatch.sql
-- workspace_resources.dirty) and blocks dispatch enqueue; this table is the
-- release RECEIPT TS writes on clear (`inspected_at`, store.ts:3312). The
-- quarantine refusal stays in the store method — a dispatch whose outcome is
-- unknown must be inspected through the stopped-dispatch inspection flow,
-- never released by this route.
--
-- TS also carries a `dirty_generation` precondition token, but no native
-- reader consumes one in this slice: native's release guard is the unresolved
-- `outcome_unknown` dispatch itself (`unresolved_dispatches`), not a token. A
-- column with no reader would be dead schema, so none is added.
CREATE TABLE IF NOT EXISTS workspace_dirty_releases (
 resource_id TEXT PRIMARY KEY REFERENCES workspace_resources(id),
 inspected_at INTEGER NOT NULL
) STRICT;

-- Per-agent execution policy (TS parity: backend-v2.js:10865-10885 GET/PUT
-- /api/agents/:name/execution-policy). TS keeps it on the agent record as
-- `agent.executionPolicy`; native's agent is the engagement, so the policy is
-- keyed by engagement. A missing row IS the TS default `{ yolo: false }`
-- (backend-v2.js:10867 reads `agent.executionPolicy?.yolo === true`), never a
-- stored zero that a reader has to distinguish from "unset".
CREATE TABLE IF NOT EXISTS agent_execution_policies (
 engagement_id TEXT PRIMARY KEY REFERENCES engagements(id),
 yolo INTEGER NOT NULL DEFAULT 0 CHECK(yolo IN (0,1)),
 updated_at INTEGER NOT NULL
) STRICT;
