-- Operator task management (TS parity: lib/task-store.js, backend-v2.js:13194-13332).
--
-- The RETAINED operator task store is a plain in-memory map persisted to
-- tasks.json: one flat record per task with an embedded comments array, no
-- session and no dispatch. That is what these two tables hold. It is NOT
-- `canonical_tasks` (migration 003): a canonical task belongs to a runner
-- session and is mutated through a dispatch capability, while an operator
-- task is created, edited and deleted by the operator with no agent in the
-- loop at all (`POST /api/tasks` carries no session; `PATCH /api/tasks/:id`
-- cannot change a status — only `/transition` can).
--
-- Columns and CHECKs are the retained vocabulary: the five statuses and the
-- transition legality live in the store module (TRANSITIONS), the four
-- priorities and three granularities are the retained sets. `labels` is the
-- retained array, held as JSON text the way every other list column here is.
-- `parent_id` deliberately carries NO foreign key: the retained
-- `createTask`/`updateTask` only require the parent to EXIST at write time
-- (`lib/task-store.js:111-113`), and `deleteTask` removes one row without
-- touching its children — a REFERENCES clause would refuse a delete the
-- retained route performs.
CREATE TABLE operator_tasks (
 id TEXT PRIMARY KEY,
 title TEXT NOT NULL,
 description TEXT NOT NULL DEFAULT '',
 status TEXT NOT NULL CHECK(status IN ('created','accepted','in_progress','blocked','done')),
 priority TEXT NOT NULL CHECK(priority IN ('p0','p1','p2','p3')),
 granularity TEXT NOT NULL CHECK(granularity IN ('epic','task','subtask')),
 assignee TEXT,
 created_by TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 started_at INTEGER,
 completed_at INTEGER,
 heartbeat_at INTEGER,
 waiting_reason TEXT,
 waiting_until TEXT,
 parent_id TEXT,
 labels TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(labels))
) STRICT;
CREATE INDEX operator_tasks_assignee ON operator_tasks(assignee,created_at,id);
CREATE INDEX operator_tasks_status ON operator_tasks(status,created_at,id);
CREATE TABLE operator_task_comments (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 task_id TEXT NOT NULL REFERENCES operator_tasks(id),
 author TEXT NOT NULL,
 body TEXT NOT NULL,
 created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX operator_task_comments_task ON operator_task_comments(task_id,sequence);