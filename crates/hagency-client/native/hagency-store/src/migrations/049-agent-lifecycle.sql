-- Task #21 (TS parity backend-v2.js:12577-12708 stop / :12712-12775 start):
-- the durable operator stop fence the retained product called `manualDown`
-- plus `offlineReason` ('operator-stop-requested' while fencing,
-- 'operator-stopped' once the stop has completed). One row per engagement.
-- `stopped_at IS NOT NULL AND started_at IS NULL` means stopped: the
-- dispatch selector excludes the engagement exactly as the retained claim
-- path skipped a manual-down agent. Start records `started_at` (the retained
-- route refuses while unconfirmed dispatch cleanups remain — the console
-- maps that to 409 agent_lifecycle_busy). Board slot is 049; this lane's
-- head is 39, so the file lands as 040 and integration renumbers (the
-- 032/033 landing-order precedent in domain.rs).
CREATE TABLE agent_lifecycle(
  engagement_id TEXT PRIMARY KEY REFERENCES engagements(id),
  stopped_at INTEGER NOT NULL,
  reason TEXT NOT NULL CHECK(reason IN ('operator-stop-requested','operator-stopped')),
  operator TEXT,
  started_at INTEGER,
  CHECK(started_at IS NULL OR started_at >= stopped_at)
) STRICT;
CREATE INDEX stopped_agents ON agent_lifecycle(stopped_at) WHERE started_at IS NULL;
