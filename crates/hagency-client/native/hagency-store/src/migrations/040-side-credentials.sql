-- Project-side credential and lifecycle state (board #14; TS parity:
-- lib/project-side-store.js). One row per server name — the id IS the
-- server name (ADR-016 decision 1) — holding the credential family the
-- ADR-132 read-only projection had to declare unavailable. The tokens
-- live ONLY here, in `credential`/`pending_credential` JSON, and every
-- read of this table goes through the allow-list projection in
-- domain/side_lifecycle.rs (the same rule as TS `publicSide`): no API
-- handler serializes these columns verbatim.
CREATE TABLE side_records (
    server_name TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    api_base_url TEXT,
    credential TEXT CHECK(credential IS NULL OR json_valid(credential)),
    pending_credential TEXT CHECK(pending_credential IS NULL OR json_valid(pending_credential)),
    pending_issued_at INTEGER,
    representative TEXT CHECK(representative IS NULL OR json_valid(representative)),
    access_state TEXT NOT NULL DEFAULT 'unverified',
    access_detail TEXT,
    access_checked_at INTEGER,
    access_issued_at INTEGER,
    allocated_tokens INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
-- A project under a side: a name and a room, not a second credential
-- (「项目方一个,但每个项目可以单独指定房间」). Archived, never deleted —
-- 「删除会有合规问题」. room_id NULL means "no room bound yet", a state the
-- console renders rather than an error.
CREATE TABLE side_projects (
    server_name TEXT NOT NULL REFERENCES side_records(server_name),
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    room_id TEXT,
    note TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    archived_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(server_name,id)
) STRICT;
