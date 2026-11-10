-- Board #49: operator message board, delivery-event log, agent tombstones and
-- the avatar request queue. Four sibling tables, one migration.
--
-- `operator_messages` is the retained backend's `messages.json` board
-- (`backend-v2.js:1721`): the operator/agent message record the retained
-- `GET /api/messages/:id` serves and `POST /api/messages/:id/suppress` writes.
-- It is NOT `admitted_messages` (004): that is authenticated Matrix ingress,
-- keyed by source event and projected into runner sessions. This table is the
-- operator board, keyed by its own id, and no ingress authority flows from it.
--
-- `delivery_events` replaces the retained `message-delivery-events.jsonl` append
-- log (`backend-v2.js:1543,1602`) with rows: `readDeliveryEvents` filters by
-- message id and agent, newest first, bounded. `id` AUTOINCREMENT is the
-- append order the jsonl file gave for free.
--
-- `agent_tombstones` replaces `deleted_agents.json` (`backend-v2.js:1718`): the
-- name a force-delete removed, so `POST /api/agents/:name/undelete` has the one
-- thing it can act on. No row means the route's 404 `no tombstone found`.
--
-- `avatar_requests` replaces the retained route's SSE hand-off
-- (`backend-v2.js:16370`): the route answers `queued: true` and the bridge does
-- the upload later, so a durable queue is that hand-off made restart-safe. The
-- base64 payload is NOT stored (the retained route holds it only in the SSE
-- frame); only the request and its flags are, bounded.
CREATE TABLE operator_messages (
  id TEXT PRIMARY KEY,
  sender TEXT NOT NULL,
  recipient TEXT,
  kind TEXT NOT NULL,
  priority TEXT NOT NULL DEFAULT 'normal' CHECK(priority IN ('normal','high','urgent')),
  summary TEXT NOT NULL,
  full TEXT NOT NULL DEFAULT '',
  mentions TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(mentions)),
  attachments TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(attachments)),
  created_at INTEGER NOT NULL,
  reply_to TEXT,
  group_id TEXT,
  source TEXT NOT NULL DEFAULT 'api',
  source_room TEXT,
  source_event_id TEXT,
  sender_mxid TEXT,
  room_recipients TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(room_recipients)),
  default_recipient TEXT,
  schema_kind TEXT,
  schema_version INTEGER,
  schema_payload TEXT CHECK(schema_payload IS NULL OR json_valid(schema_payload)),
  suppressed TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(suppressed))
) STRICT;
CREATE TABLE delivery_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  agent TEXT NOT NULL,
  message_id TEXT,
  kind TEXT NOT NULL,
  source TEXT,
  reason TEXT,
  context TEXT CHECK(context IS NULL OR json_valid(context)),
  created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX delivery_events_agent ON delivery_events(agent,id);
CREATE INDEX delivery_events_message ON delivery_events(message_id,id);
CREATE TABLE agent_tombstones (
  name TEXT PRIMARY KEY,
  deleted_at INTEGER NOT NULL,
  reason TEXT NOT NULL
) STRICT;
CREATE TABLE avatar_requests (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  agent TEXT NOT NULL,
  regenerate INTEGER NOT NULL CHECK(regenerate IN (0,1)),
  custom INTEGER NOT NULL CHECK(custom IN (0,1)),
  mime TEXT,
  requested_at INTEGER NOT NULL
) STRICT;
CREATE INDEX avatar_requests_agent ON avatar_requests(agent,id);
