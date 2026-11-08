//! Console change feed (#26): a bounded read of the tables the console's
//! live pages render — dispatches, tasks, alerts — reduced to one JSON
//! fingerprint per category. TS parity: the retained `/api/router/events`
//! sequence (router/src/store.ts:3811 `eventsAfter`: seq, watermark, gap)
//! is the contract the SSE route re-broadcasts; the fingerprint itself is
//! the native equivalent of the retained store's `snapshot()` (:3655)
//! freshness question: "did the page's rows change since I last looked".
//!
//! No new table: the feed is a pure read over existing rows, so a stopped
//! or absent stream can never wedge a writer. `stale` is true when the
//! caller's category version trails the current one — the SSE route then
//! emits that category and the page refetches its own bounded read, the
//! same division the retained dashboard used (event says "changed", page
//! reads its own data).
use crate::Error;
use rusqlite::Connection;
use serde_json::json;

/// One category's fingerprint: the row count and a digest of the rows'
/// changing columns, so a state transition with no count change still
/// reads as a change.
fn category(db: &Connection, counter: &str) -> Result<(i64, String), Error> {
    let (count, digest): (i64, String) =
        db.query_row(counter, [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok((count, digest))
}

impl super::DomainRepository {
    /// The console feed: `{agents: {version, count}, tasks: …, alerts: …}`
    /// plus a single `version` digest over all three, so a client that
    /// keeps only one cursor still learns that SOMETHING changed.
    pub fn console_feed(&self) -> Result<serde_json::Value, Error> {
        let dispatches = category(
            &self.db,
            "SELECT COUNT(*),COALESCE((SELECT group_concat(d.id||':'||d.state,'|') FROM \
             (SELECT id,state FROM runner_dispatches ORDER BY id LIMIT 200) d),'')",
        )?;
        let tasks = category(
            &self.db,
            "SELECT COUNT(*),COALESCE((SELECT group_concat(t.id||':'||json_extract(t.config,'$.status'),'|') FROM \
             (SELECT id,config FROM canonical_tasks ORDER BY id LIMIT 200) t),'')",
        )?;
        let alerts = category(
            &self.db,
            "SELECT COUNT(*),COALESCE((SELECT group_concat(dedupe_key||':'||CAST(last_seen_ms AS TEXT),'|') FROM \
             (SELECT dedupe_key,last_seen_ms FROM ceiling_alerts ORDER BY dedupe_key LIMIT 200) a),'')",
        )?;
        let agents =
            hagency_core::canonical::digest(&json!(["agents", dispatches.0, &dispatches.1,]))?;
        let tasksv = hagency_core::canonical::digest(&json!(["tasks", tasks.0, &tasks.1,]))?;
        let alertsv = hagency_core::canonical::digest(&json!(["alerts", alerts.0, &alerts.1,]))?;
        let version = hagency_core::canonical::digest(&json!([&agents, &tasksv, &alertsv,]))?;
        Ok(json!({
            "version": version,
            "agents": {"version": agents, "count": dispatches.0},
            "tasks": {"version": tasksv, "count": tasks.0},
            "alerts": {"version": alertsv, "count": alerts.0},
        }))
    }

    /// #59 named-event entities: the rows behind the TS `broadcastSSE`
    /// vocabulary, each carrying its own state word so a stream can diff
    /// two snapshots into named events with entity payloads. The task
    /// payload IS the row's stored `Task` document (canonical_tasks.config
    /// is the serialized entity, exactly what `task_updated` broadcast);
    /// the alert payload is the ceiling_alerts row (what `alert_created` /
    /// `alert_updated` / `alert_resolved` broadcast). Approvals, fences and
    /// admitted messages carry their identifying columns — the native
    /// counterparts of `approval_requested`'s `{request_id, agent}`,
    /// `agent_blocked`/`agent_recovered`'s `{agent, reason, blockedSince}`
    /// and `message`'s message row.
    pub fn console_entities(&self) -> Result<serde_json::Value, Error> {
        let mut tasks = Vec::new();
        {
            let mut stmt = self.db.prepare(
                // The table has no updated_at column; the serialized Task
                // document in config carries it (core Task struct).
                "SELECT id,config,json_extract(config,'$.updated_at') \
                 FROM canonical_tasks ORDER BY id LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<u64>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, config, updated_at) in rows {
                let mut value: serde_json::Value =
                    serde_json::from_str(&config).unwrap_or(serde_json::Value::Null);
                if let Some(object) = value.as_object_mut() {
                    object.insert("id".into(), serde_json::json!(id));
                }
                tasks.push(json!({
                    "key": id,
                    "state": value.get("status").cloned()
                        .unwrap_or(serde_json::Value::Null),
                    "updated_at": updated_at.unwrap_or_default(),
                    "entity": value,
                }));
            }
        }
        let mut alerts = Vec::new();
        {
            let mut stmt = self.db.prepare(
                "SELECT dedupe_key,resource_id,summary,detail,runbook,impact,recovery_condition,\
                 occurrences,first_seen_ms,last_seen_ms,resolved_at_ms \
                 FROM ceiling_alerts ORDER BY dedupe_key LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<u64>>(9)?,
                        row.get::<_, Option<u64>>(10)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (dedupe_key, resource_id, summary, last_seen, resolved) in rows {
                alerts.push(json!({
                    "key": dedupe_key,
                    "state": if resolved.is_some() { "resolved" } else { "open" },
                    "updated_at": last_seen.unwrap_or_default(),
                    "entity": {
                        "dedupe_key": dedupe_key,
                        "resource_id": resource_id,
                        "summary": summary,
                        "resolved_at_ms": resolved,
                    },
                }));
            }
        }
        let mut approvals = Vec::new();
        {
            let mut stmt = self.db.prepare(
                "SELECT o.id,o.state FROM owner_approvals o \
                 ORDER BY o.id LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, state) in rows {
                approvals.push(json!({
                    "key": id,
                    "state": state,
                    // The table carries no update timestamp; the diff keys
                    // on (id, state), so a state transition still reads.
                    "updated_at": 0,
                    "entity": {"request_id": id, "status": state},
                }));
            }
        }
        let mut fences = Vec::new();
        {
            let mut stmt = self.db.prepare(
                "SELECT engagement_id,reason,created_at,cleared_at FROM agent_fences \
                 ORDER BY engagement_id,id LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, Option<u64>>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (engagement, reason, created_at, cleared_at) in rows {
                fences.push(json!({
                    "key": engagement,
                    "state": if cleared_at.is_some() { "recovered" } else { "blocked" },
                    "updated_at": cleared_at.unwrap_or(created_at),
                    "entity": {
                        "agent": engagement,
                        "reason": reason,
                        "blocked_since": created_at,
                        "recovered_at": cleared_at,
                    },
                }));
            }
        }
        let mut messages = Vec::new();
        {
            let mut stmt = self.db.prepare(
                "SELECT sequence,source_key FROM admitted_messages ORDER BY sequence LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (sequence, source_key) in rows {
                messages.push(json!({
                    "key": sequence,
                    "state": "admitted",
                    "updated_at": sequence,
                    "entity": {"sequence": sequence, "source_key": source_key},
                }));
            }
        }
        let mut graphs = Vec::new();
        {
            // #59 task-graph events (lib/task-graph.js:285-390): the native
            // entities are migration 010's task_graphs and graph_nodes. The
            // key is (graph, node) so a node transition diffs like a task
            // status change; the graph's own state row keys the
            // graph-level events.
            let mut stmt = self.db.prepare(
                "SELECT graph_id,node_id,state,completed_epoch FROM graph_nodes \
                 ORDER BY graph_id,node_id LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<u64>>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (graph_id, node_id, state, completed_epoch) in rows {
                graphs.push(json!({
                    "key": format!("{graph_id}/{node_id}"),
                    "state": state,
                    "updated_at": completed_epoch.unwrap_or_default(),
                    "entity": {
                        "graph_id": graph_id,
                        "node_id": node_id,
                        "status": state,
                        "completed_at": completed_epoch,
                    },
                }));
            }
        }
        let mut graph_heads = Vec::new();
        {
            let mut stmt = self
                .db
                .prepare("SELECT id,state,created_at FROM task_graphs ORDER BY id LIMIT 500")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, state, created_at) in rows {
                graph_heads.push(json!({
                    "key": id,
                    "state": state,
                    "updated_at": created_at,
                    "entity": {
                        "graph_id": id,
                        "status": state,
                        "created_at": created_at,
                    },
                }));
            }
        }
        let version = hagency_core::canonical::digest(&json!([
            &tasks,
            &alerts,
            &approvals,
            &fences,
            &messages,
            &graphs,
            &graph_heads,
        ]))?;
        Ok(json!({
            "version": version,
            "tasks": tasks,
            "alerts": alerts,
            "approvals": approvals,
            "fences": fences,
            "messages": messages,
            "graphs": graphs,
            "graph_heads": graph_heads,
        }))
    }
}
