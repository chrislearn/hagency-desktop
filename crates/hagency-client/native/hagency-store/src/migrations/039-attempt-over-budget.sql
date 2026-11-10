-- ADR-183 decision D (amending ADR-181): the operation budget elapsing while
-- the turn is still running is a phase the attempt visits and records —
-- `over_budget`, with the budget and the elapsed time in `detail` — and never
-- a stop. The 037 CHECK enumerated the fourteen words, and SQLite cannot
-- widen a CHECK in place, so the table is rebuilt with fifteen. No table
-- references runner_attempt_events; the rows are copied unchanged.
-- Recovery fixtures rewind user_version below 37 only alongside dropping the
-- event table, exactly as before (037's rule).
CREATE TABLE runner_attempt_events_039(dispatch_id TEXT NOT NULL REFERENCES runner_dispatches(id), fence INTEGER NOT NULL, seq INTEGER NOT NULL, at_ms INTEGER NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('claimed','spawn_started','spawn_done','initialized','turn_started','approval_requested','approval_decided','parked','resumed','stop_requested','stop_reported','settled','failed','lost','over_budget')), detail TEXT NOT NULL CHECK(json_valid(detail)), PRIMARY KEY(dispatch_id,fence,seq)) STRICT;
INSERT INTO runner_attempt_events_039(dispatch_id,fence,seq,at_ms,phase,detail) SELECT dispatch_id,fence,seq,at_ms,phase,detail FROM runner_attempt_events;
DROP TABLE runner_attempt_events;
ALTER TABLE runner_attempt_events_039 RENAME TO runner_attempt_events;
