-- ADR-187 §C: the owner's cross-signing master key, pinned on first use. One
-- row per owner. `master_key` is the pinned anchor every agent enrollment and
-- approval card for that owner trusts. A later, different key the homeserver
-- reports never replaces it: it is recorded in `mismatch_key` for the
-- operator, and only an operator re-pin (`source='operator'`) changes the
-- anchor.
CREATE TABLE owner_anchors (
 owner_mxid TEXT PRIMARY KEY,
 master_key TEXT NOT NULL,
 source TEXT NOT NULL CHECK(source IN ('first_use','operator')),
 pinned_at INTEGER NOT NULL,
 mismatch_key TEXT,
 mismatch_at INTEGER
) STRICT;
