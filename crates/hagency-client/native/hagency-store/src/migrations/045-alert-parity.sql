-- Alerts like the retained store (task #24): restore the full retained
-- state machine and record shape on the ceiling-alerts rows
-- (`lib/alert-store.js`), which the native port had narrowed to four states
-- without `assigned`, without notes history, and without the retained
-- identity/severity/source/actionability fields (ADR-124's subset is
-- SUPERSEDED by this migration wherever the two disagree — the outer-loop
-- review's task #24 restores TS parity 1:1).
--
-- One table stays: the ceiling sweep keeps writing the rows it always
-- wrote; this migration ADDS the retained fields around them. NOT
-- idempotent: recovery fixtures rewind user_version and replay the whole
-- chain, and an ADD COLUMN replay fails loudly on the duplicate column,
-- exactly like migration 025.

-- The retained FIFTH state (`lib/alert-store.js:4`): an assigned alert
-- carries its assignee (normalized to 128 chars like the retained
-- `normalizeText`, `alert-store.js:106,430`). SQLite cannot widen a CHECK
-- in place, so the status constraint moves onto the alert identity: new
-- writes pass through the store's one map, and the column default stays
-- 'open'. `status` itself was added by 025 with a 4-state CHECK, so the
-- old CHECK is retired by rewriting the table (the only way SQLite
-- allows), preserving every column 025 defined.
CREATE TABLE ceiling_alerts_new (
  dedupe_key TEXT PRIMARY KEY,
  resource_id TEXT NOT NULL,
  summary TEXT NOT NULL,
  detail TEXT NOT NULL CHECK(length(detail) <= 4096),
  runbook TEXT NOT NULL,
  impact TEXT NOT NULL,
  recovery_condition TEXT NOT NULL,
  occurrences INTEGER NOT NULL CHECK(occurrences >= 1),
  first_seen_ms INTEGER NOT NULL,
  last_seen_ms INTEGER NOT NULL,
  resolved_at_ms INTEGER,
  resolved_by TEXT,
  -- The retained five states (alert-store.js:4): resolved is terminal.
  status TEXT NOT NULL DEFAULT 'open'
    CHECK(status IN ('open','acknowledged','assigned','resolved','suppressed')),
  note TEXT CHECK(note IS NULL OR length(note) <= 2048),
  transitioned_at_ms INTEGER,
  transitioned_by TEXT CHECK(transitioned_by IS NULL OR length(transitioned_by) <= 128),
  -- Retained identity fields (alert-store.js:300-328): the alert KIND
  -- (`alertType`) and where it came from. Ceiling rows file
  -- 'agent_ceiling_overrun' / 'backend' exactly like the retained sweep
  -- (backend-v2.js:9424-9427).
  alert_type TEXT NOT NULL DEFAULT 'agent_ceiling_overrun',
  severity TEXT NOT NULL DEFAULT 'warning'
    CHECK(severity IN ('info','warning','critical')),
  source TEXT NOT NULL DEFAULT 'backend'
    CHECK(source IN ('backend','bridge','supervisor','system')),
  source_agent TEXT CHECK(source_agent IS NULL OR length(source_agent) <= 128),
  -- The retained assignee (alert-store.js:430): set by a transition into
  -- `assigned` or by PATCH; never an authority grant — display state only.
  assignee TEXT CHECK(assignee IS NULL OR length(assignee) <= 128),
  -- Retained suppression window (alert-store.js:433-434,
  -- ALERT_SUPPRESS_DEFAULT_MS): an absolute millisecond deadline; the
  -- sweep's dedupe-repeat reopens the row once it passes
  -- (alert-store.js:245-248). NULL releases on operator transition only.
  suppress_until_ms INTEGER,
  -- Retained linked task (alert-store.js:477-478), normalized to 128.
  linked_task_id TEXT CHECK(linked_task_id IS NULL OR length(linked_task_id) <= 128),
  -- Retained actionability bookkeeping (alert-store.js:121-137): when a
  -- warning/critical lacks its actionable fields the retained store files
  -- it as info and records the request here.
  original_severity TEXT CHECK(original_severity IS NULL OR original_severity IN ('info','warning','critical')),
  missing_actionable_fields TEXT NOT NULL DEFAULT '[]',
  -- Retained tags (alert-store.js:317): JSON array of ≤20 strings ≤64 chars.
  tags TEXT NOT NULL DEFAULT '[]',
  -- The retained `owner` (alert-store.js:317,470): the ceiling sweep files
  -- 'hagency-operator' (`backend-v2.js:9438`); PATCH may change it.
  owner TEXT CHECK(owner IS NULL OR length(owner) <= 128)
) STRICT;
INSERT INTO ceiling_alerts_new(
  dedupe_key, resource_id, summary, detail, runbook, impact,
  recovery_condition, occurrences, first_seen_ms, last_seen_ms,
  resolved_at_ms, resolved_by, status, note, transitioned_at_ms,
  transitioned_by
)
SELECT
  dedupe_key, resource_id, summary, detail, runbook, impact,
  recovery_condition, occurrences, first_seen_ms, last_seen_ms,
  resolved_at_ms, resolved_by, status, note, transitioned_at_ms,
  transitioned_by
FROM ceiling_alerts;
DROP TABLE ceiling_alerts;
ALTER TABLE ceiling_alerts_new RENAME TO ceiling_alerts;

-- The retained NOTES history (alert-store.js:447-462): one row per note,
-- author bounded at 128 like the retained normalizeText, text at 2048
-- exactly like the retained bound, ts the statement time. The 025 `note`
-- column stays as the LATEST note (what the console transition renders);
-- this table is the full history the retained GET /api/alerts/:id exposes.
CREATE TABLE ceiling_alert_notes (
  dedupe_key TEXT NOT NULL REFERENCES ceiling_alerts(dedupe_key) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  author TEXT NOT NULL CHECK(length(author) <= 128),
  text TEXT NOT NULL CHECK(length(text) <= 2048),
  ts_ms INTEGER NOT NULL,
  PRIMARY KEY (dedupe_key, seq)
) STRICT;
CREATE INDEX ceiling_alert_notes_key ON ceiling_alert_notes(dedupe_key, seq);
