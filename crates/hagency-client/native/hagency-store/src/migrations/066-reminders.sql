-- Agent-scheduled self-reminders (board #53, TS lib/delivery-queue.js:1725-1760).
--
-- A reminder is scheduled by an agent against its OWN running session and
-- later wakes that same session with the retained `[Self Time Reminder] …`
-- text. `id` mirrors the TS `reminderIdCounter` (an integer the DELETE route
-- addresses); `msg` is the agent's own free text; `created_at`/`fire_at` are
-- wall-clock milliseconds. `fired_at` is set exactly once when the due sweep
-- claims the row, so a restart never re-fires a delivered reminder and the
-- console can distinguish pending from fired.
CREATE TABLE reminders (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 engagement_id TEXT NOT NULL REFERENCES engagements(id),
 session_id TEXT NOT NULL REFERENCES runner_sessions(id),
 msg TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 fire_at INTEGER NOT NULL,
 fired_at INTEGER
) STRICT;
CREATE INDEX reminders_due ON reminders(fire_at) WHERE fired_at IS NULL;
