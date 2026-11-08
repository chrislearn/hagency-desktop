-- ADR-186 §B: a used-up allocation pauses the agent. One row per hold; the
-- open hold (lifted_at NULL) keeps the claim from dispatching the
-- engagement's queued work, and nothing else. It is never an engagement
-- state: the engagement stays active, its running turn finishes, and queued
-- work stays queued until a top-up (§C) lifts the hold. `dispatch_id` names
-- the turn whose observation crossed the line (the pause notice's thread);
-- it is a plain id because execution retention may prune the dispatch.
-- No board number was assigned; the file takes its list version, 58.
CREATE TABLE quota_holds (
 id INTEGER PRIMARY KEY,
 engagement_id TEXT NOT NULL REFERENCES engagements(id),
 dispatch_id TEXT,
 spend INTEGER NOT NULL CHECK(spend>=0),
 allocation INTEGER NOT NULL CHECK(allocation>0),
 began_at INTEGER NOT NULL,
 lifted_at INTEGER,
 lifted_allocation INTEGER CHECK(lifted_allocation IS NULL OR lifted_allocation>0)
) STRICT;
CREATE UNIQUE INDEX open_quota_holds ON quota_holds(engagement_id) WHERE lifted_at IS NULL;
