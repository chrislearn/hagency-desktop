-- Task #13: one appservice registration per project side (ADR-016: the
-- side IS the homeserver). The board's number is 046; the schema-head
-- walk (domain.rs version + sequential migrations) places it at list
-- version 40, hence the file keeps 046 while the tuple carries 40.
-- `credential` is the LIVE credential document (camelCase keys, matching
-- the retained store's spelling); `pending`/`pending_at` hold a staged
-- reissue that never disturbs the live one until a verify promotes it.
-- Tokens are random 64-hex values, never derived; the file columns live
-- on the private state directory instead (registrations/<side>.yaml,
-- matrix.appservice_token), so no token bytes land in SQLite.
CREATE TABLE side_registrations (
    fleet_id TEXT PRIMARY KEY REFERENCES registrations(fleet_id),
    credential TEXT,
    issued_at INTEGER,
    pending TEXT,
    pending_at INTEGER
) STRICT;
