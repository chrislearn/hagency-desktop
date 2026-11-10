-- Task #19 (TS parity: backend-v2.js:15330-15385, 15802-15833, 15960-15971;
-- lib/engagement-store.js:178-232, 396-478). Role offers (count / per-engagement
-- budget / rate caps, published), the project room whitelist, and per-resource
-- agent definitions. Seats already exist; seat DELETE is behaviour, not schema.
-- IF NOT EXISTS throughout: the schema-regression fixtures rewind user_version
-- below 12/13/17/22/24/33 and re-run the whole chain (approvals.rs:968-990,
-- schema_fixtures.rs:104-119), so every CREATE must replay idempotently — the
-- same convention as migrations 018/023/024/026-031/034.
CREATE TABLE IF NOT EXISTS role_offers (
    role TEXT PRIMARY KEY,
    count INTEGER,
    budget_cap_per_engagement INTEGER,
    rate_cap INTEGER,
    published INTEGER NOT NULL CHECK(published IN (0,1)),
    updated_at INTEGER NOT NULL,
    updated_by TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS room_whitelist (
    project_room_id TEXT PRIMARY KEY,
    display_name TEXT,
    added_at INTEGER NOT NULL,
    added_by TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS room_whitelist_added ON room_whitelist(added_at DESC);
CREATE TABLE IF NOT EXISTS agent_definitions (
    id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    role TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    created_at INTEGER NOT NULL,
    UNIQUE(resource_id, name)
) STRICT;
