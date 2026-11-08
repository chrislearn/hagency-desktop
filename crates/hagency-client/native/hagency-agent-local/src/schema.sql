CREATE TABLE profile_identity (singleton INTEGER PRIMARY KEY CHECK(singleton=1),digest TEXT NOT NULL CHECK(length(digest)=64)) STRICT;
CREATE TRIGGER permanent_profile BEFORE UPDATE ON profile_identity BEGIN SELECT RAISE(ABORT,'immutable profile'); END;
CREATE TRIGGER retain_profile BEFORE DELETE ON profile_identity BEGIN SELECT RAISE(ABORT,'retain profile'); END;
CREATE TABLE identity (owner TEXT NOT NULL PRIMARY KEY) STRICT;
CREATE TRIGGER permanent_owner BEFORE UPDATE ON identity BEGIN SELECT RAISE(ABORT,'immutable owner'); END;
CREATE TRIGGER retain_owner BEFORE DELETE ON identity BEGIN SELECT RAISE(ABORT,'retain owner'); END;
CREATE TABLE bindings (id TEXT PRIMARY KEY, agent TEXT NOT NULL, room TEXT NOT NULL) STRICT;
CREATE TRIGGER immutable_binding BEFORE UPDATE ON bindings BEGIN SELECT RAISE(ABORT,'immutable binding'); END;
CREATE TRIGGER retain_binding BEFORE DELETE ON bindings BEGIN SELECT RAISE(ABORT,'retain binding'); END;
CREATE TABLE policies (scope TEXT PRIMARY KEY, revision INTEGER NOT NULL, config TEXT NOT NULL) STRICT;
CREATE TABLE accounts (scope TEXT NOT NULL, window TEXT NOT NULL, spent INTEGER NOT NULL, held INTEGER NOT NULL, PRIMARY KEY(scope,window)) STRICT;
CREATE TABLE calls (id TEXT PRIMARY KEY, binding TEXT NOT NULL REFERENCES bindings(id), scope TEXT NOT NULL, digest TEXT NOT NULL, reserved INTEGER NOT NULL, snapshots TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','unknown','settled')), usage TEXT) STRICT;
CREATE TABLE approvals (digest TEXT PRIMARY KEY, binding TEXT NOT NULL, proposal TEXT NOT NULL, expires INTEGER NOT NULL, consumed INTEGER NOT NULL DEFAULT 0) STRICT;
CREATE TABLE contexts (scope TEXT PRIMARY KEY, model_session TEXT NOT NULL) STRICT;

CREATE TABLE model_profiles (agent TEXT PRIMARY KEY, model TEXT NOT NULL, credential_ref TEXT NOT NULL, workspace_root TEXT NOT NULL, reasoning_effort TEXT NOT NULL DEFAULT '') STRICT;
CREATE TABLE dispatches (id TEXT PRIMARY KEY, scope TEXT NOT NULL, policies TEXT NOT NULL) STRICT;
CREATE TABLE codex_usage (scope TEXT PRIMARY KEY, counters TEXT NOT NULL) STRICT;

CREATE TABLE local_inbox (
 id TEXT PRIMARY KEY, binding TEXT NOT NULL REFERENCES bindings(id), event_id TEXT NOT NULL,
 immutable_digest TEXT NOT NULL, envelope TEXT NOT NULL, state TEXT NOT NULL
 CHECK(state IN ('received','acknowledged','prepared','running','reply_ready','replied','rejected','failed','unknown')),
 execution_id TEXT UNIQUE, reply TEXT, reply_digest TEXT, received_at INTEGER NOT NULL,
 UNIQUE(binding,event_id)
) STRICT;
CREATE TRIGGER retain_inbox_scope BEFORE UPDATE ON local_inbox
 WHEN NEW.id!=OLD.id OR NEW.binding!=OLD.binding OR NEW.event_id!=OLD.event_id OR NEW.immutable_digest!=OLD.immutable_digest
 OR (OLD.execution_id IS NOT NULL AND NEW.execution_id IS NOT OLD.execution_id)
 OR (OLD.state IN ('replied','rejected','failed','unknown') AND NEW.state!=OLD.state)
 OR (OLD.reply_digest IS NOT NULL AND NEW.reply_digest IS NOT OLD.reply_digest)
 BEGIN SELECT RAISE(ABORT,'immutable dispatch identity or terminal result'); END;

CREATE TABLE inbox_limits (singleton INTEGER PRIMARY KEY CHECK(singleton=1),max_records INTEGER NOT NULL,max_bytes INTEGER NOT NULL) STRICT;
INSERT INTO inbox_limits VALUES (1,10000,16777216);
