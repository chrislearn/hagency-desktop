-- Host-authored answers to a `!` command line.
--
-- A `!` line is not agent input (`verified_ingress::is_bot_command`) and it has
-- no task and no owner approval: the command layer renders the answer, and the
-- agent that received the line says it in the room as an `m.notice`. That makes
-- this custody row a sibling of `final_replies`/`task_notices`, not an instance
-- of either: `task_notices.task_id` is a NOT NULL reference to a real task, and
-- `final_replies` requires a completed runner dispatch. Reusing either would
-- mean minting a task or an approval per command, which is a lie about what
-- happened.
CREATE TABLE command_notices (
 id TEXT PRIMARY KEY,
 session_id TEXT NOT NULL REFERENCES runner_sessions(id),
 transaction_id TEXT NOT NULL UNIQUE,
 digest TEXT NOT NULL,body TEXT NOT NULL,html TEXT,
 route TEXT NOT NULL CHECK(json_valid(route)),
 source_event_id TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','claimed','sending','uncertain','delivered','cancelled')),
 cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
 fence INTEGER NOT NULL DEFAULT 0 CHECK(fence>=0),
 claim_hash TEXT,claim_until INTEGER,
 event_id TEXT,observation TEXT,
 created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,
 UNIQUE(session_id,source_event_id)
) STRICT;
CREATE INDEX command_notice_ready ON command_notices(state,id);
CREATE TABLE command_notice_inspections (
 notice_id TEXT NOT NULL REFERENCES command_notices(id),fence INTEGER NOT NULL,digest TEXT NOT NULL,
 observation TEXT NOT NULL CHECK(json_valid(observation)),
 PRIMARY KEY(notice_id,fence)
) STRICT;
-- Live only while its session still has a current route AND that route is the
-- one the answer was frozen against. A room generation that moved retires the
-- answer rather than delivering it into a room the agent may have left.
CREATE VIEW current_command_notices AS
SELECT n.id FROM command_notices n
JOIN current_matrix_routes route ON route.session_id=n.session_id
JOIN matrix_session_routes r ON r.session_id=n.session_id AND r.config=n.route;
