-- /thread session directives (task #73, TS parity: router/src/store.ts:81-82).
-- The session carries the operator's model/mode override; the next dispatch of
-- the thread reads it into the launch descriptor (host.rs). NULL = no override.
ALTER TABLE runner_sessions ADD COLUMN model_override TEXT;
ALTER TABLE runner_sessions ADD COLUMN mode_override TEXT CHECK(mode_override IN ('plan','auto') OR mode_override IS NULL);
