use std::collections::HashSet;

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::SessionDB;

/// Stable diagnostic used when an entry admission predates a durable Stop.
/// Callers that collapse `anyhow` into a transport string still need to map
/// this exact safety rejection to cancellation rather than execution failure.
pub const FOREGROUND_STOP_FENCE_ERROR: &str = "foreground request crossed a durable Stop fence";

/// Durable Stop generation captured when a foreground request first enters a
/// transport. The session hash keeps the snapshot `Copy` while preventing a
/// draft/channel remap from applying another session's lineage generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundStopAdmission {
    session_hash: Option<[u8; 32]>,
    lineage_epoch: u64,
    global_stop_epoch: u64,
    global_stop_receipt_count: u64,
}

impl ForegroundStopAdmission {
    fn baseline_for(self, session_id: &str) -> (u64, u64, u64) {
        let matches_session = self
            .session_hash
            .is_some_and(|hash| hash == *blake3::hash(session_id.as_bytes()).as_bytes());
        (
            if matches_session {
                self.lineage_epoch
            } else {
                0
            },
            self.global_stop_epoch,
            if matches_session {
                self.global_stop_receipt_count
            } else {
                0
            },
        )
    }

    pub(crate) fn resolved_for(self, session_id: &str) -> (u64, u64, u64) {
        self.baseline_for(session_id)
    }
}

pub(super) const SESSION_LINEAGE_PAUSE_EXISTS_SQL: &str =
    "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
         SELECT id, parent_session_id FROM sessions WHERE id = ?1
         UNION
         SELECT parent.id, parent.parent_session_id
           FROM sessions parent
           JOIN session_lineage child ON parent.id = child.parent_session_id
     )
     SELECT EXISTS(
         SELECT 1
           FROM session_autonomy_pauses pause
           JOIN session_lineage lineage ON lineage.id = pause.session_id
          WHERE pause.resumed_at IS NULL
     )";

/// Durable receipt for one user-initiated session Stop.
///
/// The receipt is written before controllers are paused. It therefore remains
/// an authoritative restart fence even if the process exits between the Stop
/// request and the individual Goal / Workflow state transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAutonomyPause {
    pub id: String,
    pub session_id: String,
    pub goal_id: Option<String>,
    pub workflow_run_ids: Vec<String>,
    pub subagent_run_ids: Vec<String>,
    pub created_at: String,
    pub resumed_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAutonomyResumeOutcome {
    pub resumed: bool,
    pub pause_id: Option<String>,
    pub goal_id: Option<String>,
    pub workflow_run_ids: Vec<String>,
    pub subagent_run_ids: Vec<String>,
}

fn parse_ids(raw: String) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            raw.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn row_to_pause(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionAutonomyPause> {
    Ok(SessionAutonomyPause {
        id: row.get(0)?,
        session_id: row.get(1)?,
        goal_id: row.get(2)?,
        workflow_run_ids: parse_ids(row.get(3)?)?,
        subagent_run_ids: parse_ids(row.get(4)?)?,
        created_at: row.get(5)?,
        resumed_at: row.get(6)?,
    })
}

pub(super) fn session_autonomy_lineage_pause_epoch_with_conn(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<u64> {
    conn.query_row(
        "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
             SELECT id, parent_session_id FROM sessions WHERE id = ?1
             UNION
             SELECT parent.id, parent.parent_session_id
               FROM sessions parent
               JOIN session_lineage child ON parent.id = child.parent_session_id
         )
         SELECT COUNT(*)
           FROM session_autonomy_pauses pause
           JOIN session_lineage lineage ON lineage.id = pause.session_id",
        params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value.max(0) as u64)
    .map_err(Into::into)
}

pub(super) fn lineage_attributed_global_stop_receipt_count_with_conn(
    conn: &rusqlite::Connection,
    session_id: &str,
    global_stop_epoch: u64,
) -> Result<u64> {
    let global_stop_epoch = i64::try_from(global_stop_epoch)
        .map_err(|_| anyhow!("global Stop epoch exceeds SQLite INTEGER range"))?;
    conn.query_row(
        "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
             SELECT id, parent_session_id FROM sessions WHERE id = ?1
             UNION
             SELECT parent.id, parent.parent_session_id
               FROM sessions parent
               JOIN session_lineage child ON parent.id = child.parent_session_id
         )
         SELECT COUNT(*)
           FROM session_autonomy_pauses pause
           JOIN session_lineage lineage ON lineage.id = pause.session_id
          WHERE pause.global_stop_epoch IS NOT NULL
            AND pause.global_stop_epoch <= ?2",
        params![session_id, global_stop_epoch],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value.max(0) as u64)
    .map_err(Into::into)
}

pub(super) fn global_stop_epoch_with_conn(conn: &rusqlite::Connection) -> Result<u64> {
    conn.query_row(
        "SELECT epoch FROM runtime_control_epochs WHERE key = 'global_stop'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value.max(0) as u64)
    .map_err(Into::into)
}

pub(super) fn foreground_stop_admission_with_conn(
    conn: &rusqlite::Connection,
    session_id: Option<&str>,
) -> Result<ForegroundStopAdmission> {
    let global_stop_epoch = global_stop_epoch_with_conn(conn)?;
    let Some(session_id) = session_id else {
        return Ok(ForegroundStopAdmission {
            session_hash: None,
            lineage_epoch: 0,
            global_stop_epoch,
            global_stop_receipt_count: 0,
        });
    };
    Ok(ForegroundStopAdmission {
        session_hash: Some(*blake3::hash(session_id.as_bytes()).as_bytes()),
        lineage_epoch: session_autonomy_lineage_pause_epoch_with_conn(conn, session_id)?,
        global_stop_epoch,
        global_stop_receipt_count: lineage_attributed_global_stop_receipt_count_with_conn(
            conn,
            session_id,
            global_stop_epoch,
        )?,
    })
}

pub(super) fn foreground_stop_admission_is_current_with_conn(
    conn: &rusqlite::Connection,
    session_id: &str,
    admission: ForegroundStopAdmission,
) -> Result<bool> {
    let (admitted_lineage, admitted_global, admitted_global_receipts) =
        admission.baseline_for(session_id);
    let current_global = global_stop_epoch_with_conn(conn)?;
    if current_global > admitted_global {
        return Ok(false);
    }
    let current_lineage = session_autonomy_lineage_pause_epoch_with_conn(conn, session_id)?;
    let current_global_receipts =
        lineage_attributed_global_stop_receipt_count_with_conn(conn, session_id, admitted_global)?;
    Ok(current_lineage.saturating_sub(admitted_lineage)
        <= current_global_receipts.saturating_sub(admitted_global_receipts))
}

fn list_session_ids_with_active_autonomy_with_conn(
    conn: &rusqlite::Connection,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE active_sessions(id) AS (
             SELECT session_id FROM goals
              WHERE state IN ('active', 'evaluating')
             UNION
             SELECT session_id FROM workflow_runs
              WHERE state IN ('draft', 'running', 'recovering')
             UNION
             SELECT parent_session_id FROM subagent_runs
              WHERE status IN ('queued', 'spawning', 'running')
             UNION
             SELECT parent_session_id FROM subagent_result_deliveries
              WHERE state IN ('pending', 'injecting', 'injecting_no_replay')
             UNION
             SELECT session_id FROM chat_stream_runs
              WHERE status = 'running'
                AND source IN ('desktop', 'http', 'channel', 'acp')
             UNION
             SELECT turn.session_id FROM chat_turns turn
               JOIN sessions session ON session.id = turn.session_id
              WHERE turn.status IN ('running', 'cancelling')
                AND session.incognito = 0
             UNION
             SELECT direct.session_id FROM direct_turn_admissions direct
               JOIN sessions session ON session.id = direct.session_id
              WHERE session.incognito = 0
             UNION
             SELECT session_id FROM queued_turn_user_messages
              WHERE source IN ('channel', 'scheduled')
                AND status IN ('queued', 'fallback_after_reply', 'waiting_tool_boundary',
                               'inserting', 'dispatching')
         ), session_lineage(active_id, id, parent_session_id) AS (
             SELECT active.id, session.id, session.parent_session_id
               FROM active_sessions active
               JOIN sessions session ON session.id = active.id
             UNION
             SELECT lineage.active_id, parent.id, parent.parent_session_id
               FROM session_lineage lineage
               JOIN sessions parent ON parent.id = lineage.parent_session_id
         )
         SELECT DISTINCT id FROM session_lineage
          WHERE parent_session_id IS NULL
          ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

impl SessionDB {
    pub fn foreground_stop_admission(
        &self,
        session_id: Option<&str>,
    ) -> Result<ForegroundStopAdmission> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let admission = foreground_stop_admission_with_conn(&tx, session_id)?;
        tx.commit()?;
        Ok(admission)
    }

    pub fn foreground_stop_admission_is_current(
        &self,
        session_id: &str,
        admission: ForegroundStopAdmission,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;
        let current = foreground_stop_admission_is_current_with_conn(&tx, session_id, admission)?;
        tx.commit()?;
        Ok(current)
    }

    /// Resolve any hidden descendant conversation to its top-level, visible
    /// session. Global Stop uses this before publishing receipts so Continue
    /// never has to discover and consume an invisible child-only fence.
    pub fn resolve_session_root_id(&self, session_id: &str) -> Result<String> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
                 SELECT id, parent_session_id FROM sessions WHERE id = ?1
                 UNION
                 SELECT parent.id, parent.parent_session_id
                   FROM sessions parent
                   JOIN session_lineage child ON parent.id = child.parent_session_id
             )
             SELECT id FROM session_lineage
              WHERE parent_session_id IS NULL
              LIMIT 1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))
    }

    /// Root plus every hidden sub-agent session descended from it.
    pub fn list_session_autonomy_tree_ids(&self, root_session_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let mut stmt = conn.prepare(
            "WITH RECURSIVE session_tree(id) AS (
                 SELECT id FROM sessions WHERE id = ?1
                 UNION
                 SELECT child.id
                   FROM sessions child
                   JOIN session_tree parent ON child.parent_session_id = parent.id
             )
             SELECT id FROM session_tree ORDER BY id",
        )?;
        let rows = stmt.query_map(params![root_session_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Sessions that currently own autonomous controllers or a durable
    /// foreground stream and therefore need a receipt for global Stop.
    pub fn list_session_ids_with_active_autonomy(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        list_session_ids_with_active_autonomy_with_conn(&conn)
    }

    /// Atomically publish a cross-process global Stop generation and snapshot
    /// every durable controller/foreground session admitted before it.
    pub fn begin_global_stop_enumeration(&self) -> Result<(u64, Vec<String>)> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE runtime_control_epochs
                SET epoch = epoch + 1, updated_at = ?1
              WHERE key = 'global_stop'",
            params![now],
        )?;
        let epoch = global_stop_epoch_with_conn(&tx)?;
        let mut sessions = list_session_ids_with_active_autonomy_with_conn(&tx)?;
        let epoch_i64 = i64::try_from(epoch)
            .map_err(|_| anyhow!("global Stop epoch exceeds SQLite INTEGER range"))?;

        // Rotate every already-active receipt inside the same write ordering
        // point as the new generation. A Continue carrying the old id must not
        // release Scheduled rows quarantined by this newer Global Stop while
        // the detached owner is still publishing per-session receipts. Copying
        // the captured controller ids into a new-generation receipt preserves
        // targeted Stop semantics and also leaves an exact resumable receipt if
        // the process exits before the eager convergence pass reaches it.
        let active_pauses = {
            let mut stmt = tx.prepare(
                "SELECT id, session_id, goal_id, workflow_run_ids_json,
                        subagent_run_ids_json
                   FROM session_autonomy_pauses
                  WHERE resumed_at IS NULL
                  ORDER BY session_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut current_receipt_sessions = HashSet::new();
        for (old_id, session_id, goal_id, workflow_ids, subagent_ids) in active_pauses {
            let changed = tx.execute(
                "UPDATE session_autonomy_pauses
                    SET resumed_at = ?1,
                        resume_replay_error = 'superseded_by_newer_stop'
                  WHERE id = ?2 AND resumed_at IS NULL",
                params![now, old_id],
            )?;
            if changed != 1 {
                return Err(anyhow!(
                    "active session pause changed during Global Stop publication"
                ));
            }
            tx.execute(
                "INSERT INTO session_autonomy_pauses (
                    id, session_id, goal_id, workflow_run_ids_json,
                    subagent_run_ids_json, created_at, resumed_at, global_stop_epoch
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
                params![
                    format!("pause_{}", uuid::Uuid::new_v4().simple()),
                    session_id,
                    goal_id,
                    workflow_ids,
                    subagent_ids,
                    now,
                    epoch_i64,
                ],
            )?;
            current_receipt_sessions.insert(session_id.clone());
            sessions.push(session_id);
        }

        // Continue publishes a durable Primary replay handoff after consuming
        // its pause. A newer Global Stop must supersede that handoff in this
        // same transaction: otherwise another process can replay the already-
        // consumed receipt before the detached Global Stop owner recreates the
        // session fence. Preserve the newest captured snapshot as the current
        // epoch receipt; all older pending handoffs are terminally superseded.
        let pending_resumes = {
            let mut stmt = tx.prepare(
                "SELECT id, session_id, goal_id, workflow_run_ids_json,
                        subagent_run_ids_json
                   FROM session_autonomy_pauses
                  WHERE resume_requested_at IS NOT NULL
                    AND resume_replayed_at IS NULL
                  ORDER BY session_id, resume_requested_at DESC, id DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (old_id, session_id, goal_id, workflow_ids, subagent_ids) in pending_resumes {
            let changed = tx.execute(
                "UPDATE session_autonomy_pauses
                    SET resume_replayed_at = ?1,
                        resume_replay_error = 'superseded_by_newer_stop'
                  WHERE id = ?2
                    AND resume_requested_at IS NOT NULL
                    AND resume_replayed_at IS NULL",
                params![now, old_id],
            )?;
            if changed != 1 {
                return Err(anyhow!(
                    "pending session Continue changed during Global Stop publication"
                ));
            }
            if current_receipt_sessions.insert(session_id.clone()) {
                tx.execute(
                    "INSERT INTO session_autonomy_pauses (
                        id, session_id, goal_id, workflow_run_ids_json,
                        subagent_run_ids_json, created_at, resumed_at, global_stop_epoch
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
                    params![
                        format!("pause_{}", uuid::Uuid::new_v4().simple()),
                        session_id,
                        goal_id,
                        workflow_ids,
                        subagent_ids,
                        now,
                        epoch_i64,
                    ],
                )?;
            }
            sessions.push(session_id);
        }
        sessions.sort();
        sessions.dedup();
        // The generation publication and quarantine of every pre-existing
        // backend-managed row are one SQLite ordering point. Primary Channel
        // and Scheduled pumps can therefore never re-snapshot old work as a
        // post-Stop admission while per-session receipts are still being
        // published asynchronously.
        tx.execute(
            "UPDATE queued_turn_user_messages
                SET mode = 'queue', status = 'held_after_stop', turn_id = NULL,
                    updated_at = ?1
              WHERE source IN ('channel', 'scheduled')
                AND status IN ('queued', 'fallback_after_reply', 'waiting_tool_boundary',
                               'inserting', 'dispatching')",
            params![now],
        )?;
        tx.commit()?;
        Ok((epoch, sessions))
    }

    pub fn global_stop_epoch(&self) -> Result<u64> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        global_stop_epoch_with_conn(&conn)
    }

    /// Capture the exact autonomous controllers owned by a session and publish
    /// a durable pause fence before any in-process cancellation is attempted.
    ///
    /// Every Stop replaces the prior active receipt with a fresh generation.
    /// Captured controller ids are carried forward so repeatedly pressing Stop
    /// cannot make already-paused work impossible to resume, while a Continue
    /// bound to the older id can no longer consume the newer user decision.
    pub fn prepare_session_autonomy_pause(&self, session_id: &str) -> Result<SessionAutonomyPause> {
        self.prepare_session_autonomy_pause_inner(session_id, None)
    }

    pub(crate) fn prepare_session_autonomy_pause_for_global(
        &self,
        session_id: &str,
        global_stop_epoch: u64,
    ) -> Result<SessionAutonomyPause> {
        self.prepare_session_autonomy_pause_inner(session_id, Some(global_stop_epoch))
    }

    fn prepare_session_autonomy_pause_inner(
        &self,
        session_id: &str,
        global_stop_epoch: Option<u64>,
    ) -> Result<SessionAutonomyPause> {
        let global_stop_epoch = global_stop_epoch
            .map(i64::try_from)
            .transpose()
            .map_err(|_| anyhow!("global Stop epoch exceeds SQLite INTEGER range"))?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        // One published global generation is idempotent across process-local
        // watchers. Concurrent owners may discover the same volatile session,
        // but they must converge on one pause id so a Continue cannot become
        // stale merely because another process repeated the same receipt.
        if let Some(global_stop_epoch) = global_stop_epoch {
            if let Some(existing) = tx
                .query_row(
                    "SELECT id, session_id, goal_id, workflow_run_ids_json,
                            subagent_run_ids_json, created_at, resumed_at
                       FROM session_autonomy_pauses
                      WHERE session_id = ?1
                        AND global_stop_epoch = ?2
                        AND resumed_at IS NULL
                      ORDER BY created_at DESC LIMIT 1",
                    params![session_id, global_stop_epoch],
                    row_to_pause,
                )
                .optional()?
            {
                return Ok(existing);
            }
        }

        let previous = tx
            .query_row(
                "SELECT id, session_id, goal_id, workflow_run_ids_json,
                        subagent_run_ids_json, created_at, resumed_at
                   FROM session_autonomy_pauses
                  WHERE session_id = ?1 AND resumed_at IS NULL
                  ORDER BY created_at DESC LIMIT 1",
                params![session_id],
                row_to_pause,
            )
            .optional()?;

        let session_exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )? != 0;
        if !session_exists {
            return Err(anyhow!("session not found: {session_id}"));
        }

        let goal_id = tx
            .query_row(
                "SELECT id FROM goals
                  WHERE session_id = ?1
                    AND state IN ('active', 'evaluating')
                  ORDER BY updated_at DESC LIMIT 1",
                params![session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .or_else(|| previous.as_ref().and_then(|pause| pause.goal_id.clone()));

        let mut workflow_run_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE session_tree(id) AS (
                     SELECT id FROM sessions WHERE id = ?1
                     UNION
                     SELECT child.id
                       FROM sessions child
                       JOIN session_tree parent ON child.parent_session_id = parent.id
                 )
                 SELECT id FROM workflow_runs
                  WHERE session_id IN (SELECT id FROM session_tree)
                    AND state IN ('draft', 'running', 'recovering')
                  ORDER BY updated_at ASC",
            )?;
            let rows = stmt
                .query_map(params![session_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if let Some(previous) = previous.as_ref() {
            let mut seen = workflow_run_ids.iter().cloned().collect::<HashSet<_>>();
            workflow_run_ids.extend(
                previous
                    .workflow_run_ids
                    .iter()
                    .filter(|id| seen.insert((*id).clone()))
                    .cloned(),
            );
        }

        let mut subagent_run_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE session_tree(id) AS (
                     SELECT id FROM sessions WHERE id = ?1
                     UNION
                     SELECT child.id
                       FROM sessions child
                       JOIN session_tree parent ON child.parent_session_id = parent.id
                 )
                 SELECT run_id FROM subagent_runs
                  WHERE parent_session_id IN (SELECT id FROM session_tree)
                    AND (
                        status IN ('queued', 'spawning', 'running')
                        OR run_id IN (
                            SELECT run_id FROM subagent_result_deliveries
                             WHERE state IN ('pending', 'injecting', 'injecting_no_replay')
                        )
                    )
                  ORDER BY started_at ASC",
            )?;
            let rows = stmt
                .query_map(params![session_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if let Some(previous) = previous.as_ref() {
            let mut seen = subagent_run_ids.iter().cloned().collect::<HashSet<_>>();
            subagent_run_ids.extend(
                previous
                    .subagent_run_ids
                    .iter()
                    .filter(|id| seen.insert((*id).clone()))
                    .cloned(),
            );
        }

        let now = chrono::Utc::now().to_rfc3339();
        // A newer Stop supersedes any durable resume handoff that the Primary
        // has not consumed yet. Source-level pause fences still cover an
        // already-running replay, while this update prevents a stale request
        // from waking the session after the next Continue.
        tx.execute(
            "UPDATE session_autonomy_pauses
                SET resume_replayed_at = ?1,
                    resume_replay_error = 'superseded_by_newer_stop'
              WHERE session_id = ?2
                AND resume_requested_at IS NOT NULL
                AND resume_replayed_at IS NULL",
            params![now, session_id],
        )?;
        let pause = SessionAutonomyPause {
            id: format!("pause_{}", uuid::Uuid::new_v4().simple()),
            session_id: session_id.to_string(),
            goal_id,
            workflow_run_ids,
            subagent_run_ids,
            created_at: now.clone(),
            resumed_at: None,
        };
        if let Some(previous) = previous.as_ref() {
            tx.execute(
                "UPDATE session_autonomy_pauses
                    SET resumed_at = ?1
                  WHERE id = ?2 AND resumed_at IS NULL",
                params![now, previous.id],
            )?;
        }
        tx.execute(
            "INSERT INTO session_autonomy_pauses (
                id, session_id, goal_id, workflow_run_ids_json,
                subagent_run_ids_json, created_at, resumed_at, global_stop_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
            params![
                pause.id,
                pause.session_id,
                pause.goal_id,
                serde_json::to_string(&pause.workflow_run_ids)?,
                serde_json::to_string(&pause.subagent_run_ids)?,
                pause.created_at,
                global_stop_epoch,
            ],
        )?;
        tx.execute(
            "UPDATE queued_turn_user_messages
                SET mode = 'queue', status = 'held_after_stop', turn_id = NULL,
                    updated_at = ?1
              WHERE session_id = ?2 AND source = 'channel'
                AND status IN ('queued', 'fallback_after_reply', 'waiting_tool_boundary',
                               'inserting', 'dispatching')",
            params![now, session_id],
        )?;
        tx.commit()?;
        Ok(pause)
    }

    pub fn active_session_autonomy_pause(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionAutonomyPause>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "SELECT id, session_id, goal_id, workflow_run_ids_json,
                    subagent_run_ids_json, created_at, resumed_at
               FROM session_autonomy_pauses
              WHERE session_id = ?1 AND resumed_at IS NULL
              ORDER BY created_at DESC LIMIT 1",
            params![session_id],
            row_to_pause,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Resolve the active Stop receipt inherited by this session. Hidden
    /// sub-agent conversations inherit their visible root's receipt.
    pub fn active_session_or_ancestor_autonomy_pause(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionAutonomyPause>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
                 SELECT id, parent_session_id FROM sessions WHERE id = ?1
                 UNION
                 SELECT parent.id, parent.parent_session_id
                   FROM sessions parent
                   JOIN session_lineage child ON parent.id = child.parent_session_id
             )
             SELECT pause.id, pause.session_id, pause.goal_id,
                    pause.workflow_run_ids_json, pause.subagent_run_ids_json,
                    pause.created_at, pause.resumed_at
               FROM session_autonomy_pauses pause
               JOIN session_lineage lineage ON lineage.id = pause.session_id
              WHERE pause.resumed_at IS NULL
              ORDER BY pause.created_at DESC LIMIT 1",
            params![session_id],
            row_to_pause,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Whether this session lineage has an active or already-consumed receipt
    /// attributed to one exact global Stop generation. Process-local volatile
    /// controllers use this to follow a Stop/Continue handled by another
    /// process without publishing their runtime identity in shared storage.
    pub(crate) fn session_lineage_global_stop_receipt_state(
        &self,
        session_id: &str,
        global_stop_epoch: u64,
    ) -> Result<(bool, bool)> {
        let global_stop_epoch = i64::try_from(global_stop_epoch)
            .map_err(|_| anyhow!("global Stop epoch exceeds SQLite INTEGER range"))?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
                 SELECT id, parent_session_id FROM sessions WHERE id = ?1
                 UNION
                 SELECT parent.id, parent.parent_session_id
                   FROM sessions parent
                   JOIN session_lineage child ON parent.id = child.parent_session_id
             )
             SELECT EXISTS(
                        SELECT 1
                          FROM session_autonomy_pauses pause
                          JOIN session_lineage lineage ON lineage.id = pause.session_id
                         WHERE pause.global_stop_epoch = ?2
                           AND pause.resumed_at IS NULL
                    ),
                    EXISTS(
                        SELECT 1
                          FROM session_autonomy_pauses pause
                          JOIN session_lineage lineage ON lineage.id = pause.session_id
                         WHERE pause.global_stop_epoch = ?2
                           AND pause.resumed_at IS NOT NULL
                    )",
            params![session_id, global_stop_epoch],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
        )
        .map_err(Into::into)
    }

    pub fn is_session_autonomy_paused(&self, session_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM session_autonomy_pauses
                 WHERE session_id = ?1 AND resumed_at IS NULL
             )",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(Into::into)
    }

    /// Effective Stop fence for a session. Nested sub-agent sessions inherit
    /// the pause of their root conversation, so a late grandchild spawn cannot
    /// cross a Stop merely because its immediate parent has a different id.
    pub fn is_session_or_ancestor_autonomy_paused(&self, session_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            SESSION_LINEAGE_PAUSE_EXISTS_SQL,
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(Into::into)
    }

    /// Monotonic generation for invalidating autonomous work admitted before
    /// a Stop. Unlike the active flag, this never decreases on Continue, so an
    /// injector that waited across Stop/Continue cannot accidentally proceed.
    pub fn session_autonomy_pause_epoch(&self, session_id: &str) -> Result<u64> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value.max(0) as u64)
        .map_err(Into::into)
    }

    /// Monotonic Stop generation inherited from this session's full ancestry.
    pub fn session_autonomy_lineage_pause_epoch(&self, session_id: &str) -> Result<u64> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        session_autonomy_lineage_pause_epoch_with_conn(&conn, session_id)
    }

    /// Stop admission snapshot used by foreground turns. The third component
    /// attributes lineage receipts that belong to this or any earlier global
    /// Stop generation. Receipts that land late can then be distinguished
    /// from a newer global or session-scoped Stop.
    pub fn session_autonomy_stop_admission(&self, session_id: &str) -> Result<(u64, u64, u64)> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let lineage_epoch = session_autonomy_lineage_pause_epoch_with_conn(&conn, session_id)?;
        let global_stop_epoch = global_stop_epoch_with_conn(&conn)?;
        let global_receipt_count = lineage_attributed_global_stop_receipt_count_with_conn(
            &conn,
            session_id,
            global_stop_epoch,
        )?;
        Ok((lineage_epoch, global_stop_epoch, global_receipt_count))
    }

    pub fn session_lineage_attributed_global_stop_receipt_count(
        &self,
        session_id: &str,
        global_stop_epoch: u64,
    ) -> Result<u64> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        lineage_attributed_global_stop_receipt_count_with_conn(&conn, session_id, global_stop_epoch)
    }

    /// Resolve durable Stop generations against process-local runtime
    /// snapshots. Pause rows are never deleted, so a fast Continue cannot hide
    /// an older generation from an injection or immutable sub-agent/workflow
    /// attempt that was admitted before that Stop.
    pub(crate) fn resolve_local_autonomy_stop_fences(
        &self,
        foreground_generations: &[(String, String, u64, u64, u64)],
        injection_generations: &[(String, u64)],
        subagent_run_ids: &[String],
        workflow_generations: &[(String, String, u64)],
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>, Vec<String>)> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let global_stop_epoch = global_stop_epoch_with_conn(&conn)?;
        let mut epoch_stmt = conn.prepare(
            "WITH RECURSIVE session_lineage(id, parent_session_id) AS (
                 SELECT id, parent_session_id FROM sessions WHERE id = ?1
                 UNION
                 SELECT parent.id, parent.parent_session_id
                   FROM sessions parent
                   JOIN session_lineage child ON parent.id = child.parent_session_id
             )
             SELECT COUNT(*)
               FROM session_autonomy_pauses pause
               JOIN session_lineage lineage ON lineage.id = pause.session_id",
        )?;
        let mut stale_injections = Vec::new();
        for (session_id, admitted_epoch) in injection_generations {
            let epoch = epoch_stmt
                .query_row(params![session_id], |row| row.get::<_, i64>(0))?
                .max(0) as u64;
            if epoch > *admitted_epoch {
                stale_injections.push(session_id.clone());
            }
        }

        let mut stale_foreground_runs = Vec::new();
        for (
            run_id,
            session_id,
            admitted_epoch,
            admitted_global_stop_epoch,
            admitted_global_receipt_count,
        ) in foreground_generations
        {
            let epoch = epoch_stmt
                .query_row(params![session_id], |row| row.get::<_, i64>(0))?
                .max(0) as u64;
            let global_receipt_count = lineage_attributed_global_stop_receipt_count_with_conn(
                &conn,
                session_id,
                *admitted_global_stop_epoch,
            )?;
            let added_lineage_receipts = epoch.saturating_sub(*admitted_epoch);
            let added_attributed_global_receipts =
                global_receipt_count.saturating_sub(*admitted_global_receipt_count);
            if global_stop_epoch > *admitted_global_stop_epoch
                || added_lineage_receipts > added_attributed_global_receipts
            {
                stale_foreground_runs.push(run_id.clone());
            }
        }

        let mut subagent_stmt = conn.prepare(
            "SELECT EXISTS(
                 SELECT 1
                   FROM session_autonomy_pauses pause,
                        json_each(pause.subagent_run_ids_json) captured
                  WHERE captured.value = ?1
             )",
        )?;
        let mut captured_subagents = Vec::new();
        for run_id in subagent_run_ids {
            if subagent_stmt.query_row(params![run_id], |row| row.get::<_, i64>(0))? != 0 {
                captured_subagents.push(run_id.clone());
            }
        }

        let mut stale_workflows = Vec::new();
        for (run_id, session_id, admitted_epoch) in workflow_generations {
            let epoch = epoch_stmt
                .query_row(params![session_id], |row| row.get::<_, i64>(0))?
                .max(0) as u64;
            if epoch > *admitted_epoch {
                stale_workflows.push(run_id.clone());
            }
        }

        Ok((
            stale_injections,
            captured_subagents,
            stale_workflows,
            stale_foreground_runs,
        ))
    }

    /// Consume the active receipt last and atomically publish a durable replay
    /// request for the Primary process. A crash before this CAS leaves the
    /// restart fence active and Continue safely retryable; a Secondary can
    /// report success only after the Primary handoff is durable.
    pub fn finish_session_autonomy_resume(&self, pause_id: &str) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let pause = tx
            .query_row(
                "SELECT session_id, subagent_run_ids_json
                   FROM session_autonomy_pauses
                  WHERE id = ?1 AND resumed_at IS NULL",
                params![pause_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        parse_ids(row.get::<_, String>(1)?)?,
                    ))
                },
            )
            .optional()?;
        let Some((session_id, subagent_ids)) = pause else {
            tx.commit()?;
            return Ok(false);
        };

        // Every captured child belongs to this Stop generation. The explicit
        // Continue turn consumes its eventual terminal state through the exact
        // run ids and runtime-recovery reminder. If an old injector was already
        // claimed, leave a suppression request for its generation-fence
        // rollback to settle instead of launching a duplicate parent turn
        // alongside Continue.
        let now = chrono::Utc::now().to_rfc3339();
        for run_id in subagent_ids {
            tx.execute(
                "UPDATE subagent_result_deliveries
                    SET state = CASE WHEN state = 'pending' THEN 'suppressed' ELSE state END,
                        suppress_reason = 'session_continue_uses_runtime_recovery',
                        delivered_at = CASE WHEN state = 'pending' THEN ?1 ELSE delivered_at END,
                        last_error = CASE WHEN state = 'pending' THEN NULL ELSE last_error END
                  WHERE run_id = ?2
                    AND state IN ('pending', 'injecting', 'injecting_no_replay')",
                params![now, run_id],
            )?;
        }
        let changed = tx.execute(
            "UPDATE session_autonomy_pauses
                SET resumed_at = ?1,
                    resume_requested_at = ?1,
                    resume_global_stop_epoch = (
                        SELECT epoch FROM runtime_control_epochs WHERE key = 'global_stop'
                    ),
                    resume_replayed_at = NULL,
                    resume_replay_error = NULL
              WHERE id = ?2 AND resumed_at IS NULL",
            params![now, pause_id],
        )?;
        if changed == 1 {
            // Global Stop quarantines Scheduled rows in the same transaction
            // that publishes its epoch. Release them only after this exact,
            // still-active pause generation wins the Continue CAS. A stale
            // Continue either observes `resumed_at` above or loses this writer
            // ordering to a newer Stop, whose transaction holds the rows again.
            tx.execute(
                "UPDATE queued_turn_user_messages
                    SET status = 'queued', updated_at = ?1
                  WHERE session_id = ?2 AND source = 'scheduled'
                    AND status = 'held_after_stop'",
                params![now, session_id],
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    pub(crate) fn session_autonomy_resume_global_stop_epoch(&self, pause_id: &str) -> Result<u64> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row(
            "SELECT COALESCE(resume_global_stop_epoch, 0)
               FROM session_autonomy_pauses
              WHERE id = ?1 AND resume_requested_at IS NOT NULL",
            params![pause_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value.max(0) as u64)
        .map_err(Into::into)
    }

    /// Pending Primary-owned replay requests published by Continue. There is
    /// exactly one Primary process, and its replay loop is single-flight; the
    /// source-specific wakeup/workflow/delivery claims remain the idempotency
    /// boundary if the process crashes after dispatch but before the ack.
    pub(crate) fn list_pending_session_autonomy_resume_replays(
        &self,
        limit: usize,
    ) -> Result<Vec<SessionAutonomyPause>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT pause.id, pause.session_id, pause.goal_id,
                    pause.workflow_run_ids_json, pause.subagent_run_ids_json,
                    pause.created_at, pause.resumed_at
               FROM session_autonomy_pauses pause
              WHERE pause.resume_requested_at IS NOT NULL
                AND pause.resume_replayed_at IS NULL
                AND NOT EXISTS (
                    SELECT 1 FROM session_autonomy_pauses active
                     WHERE active.session_id = pause.session_id
                       AND active.resumed_at IS NULL
                )
              ORDER BY pause.resume_requested_at ASC, pause.id ASC
              LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.max(1) as i64], row_to_pause)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn finish_session_autonomy_resume_replay(&self, pause_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let changed = conn.execute(
            "UPDATE session_autonomy_pauses
                SET resume_replayed_at = ?1,
                    resume_replay_error = NULL
              WHERE id = ?2
                AND resume_requested_at IS NOT NULL
                AND resume_replayed_at IS NULL
                AND NOT EXISTS (
                    SELECT 1 FROM session_autonomy_pauses active
                     WHERE active.session_id = session_autonomy_pauses.session_id
                       AND active.resumed_at IS NULL
                )",
            params![chrono::Utc::now().to_rfc3339(), pause_id],
        )?;
        Ok(changed > 0)
    }

    pub(crate) fn record_session_autonomy_resume_replay_error(
        &self,
        pause_id: &str,
        error: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE session_autonomy_pauses
                SET resume_replay_error = ?1
              WHERE id = ?2 AND resume_replayed_at IS NULL",
            params![error, pause_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_stop_creates_a_new_generation_and_epoch_never_rewinds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db =
            SessionDB::open_ephemeral_for_test(&dir.path().join("pause.db")).expect("session db");
        let session = db.create_session("ha-main").expect("session");
        let child = db
            .create_session_with_parent("helper", Some(&session.id))
            .expect("child session");
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Finish the interrupted work".to_string(),
                completion_criteria: "Verified result exists".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("goal");
        db.transition_goal(
            &goal.goal.id,
            crate::goal::GoalState::Evaluating,
            Some("test"),
        )
        .expect("evaluating goal");
        let workflow = db
            .create_workflow_run(crate::workflow::CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "test.pause".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main() {}".to_string(),
                budget: serde_json::json!({}),
                parent_run_id: None,
                origin: None,
                goal_id: Some(goal.goal.id.clone()),
                goal_criterion_id: None,
                worktree_id: None,
            })
            .expect("workflow");

        assert_eq!(
            db.list_session_ids_with_active_autonomy().unwrap(),
            vec![session.id.clone()]
        );

        let first = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("first receipt");
        assert_eq!(
            db.pause_goal(&goal.goal.id).expect("pause goal").goal.state,
            crate::goal::GoalState::Paused
        );
        assert_eq!(
            db.pause_workflow_run(&workflow.id)
                .expect("pause workflow")
                .state,
            crate::workflow::WorkflowRunState::Paused
        );
        let duplicate = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("duplicate receipt");
        assert_ne!(duplicate.id, first.id);
        assert_eq!(duplicate.goal_id.as_deref(), Some(goal.goal.id.as_str()));
        assert_eq!(duplicate.workflow_run_ids, vec![workflow.id.clone()]);
        assert_eq!(db.session_autonomy_pause_epoch(&session.id).unwrap(), 2);
        assert!(!db.finish_session_autonomy_resume(&first.id).unwrap());
        assert_eq!(
            db.active_session_or_ancestor_autonomy_pause(&child.id)
                .unwrap()
                .expect("inherited pause")
                .id,
            duplicate.id
        );
        assert_eq!(db.resolve_session_root_id(&child.id).unwrap(), session.id);
        assert!(db.finish_session_autonomy_resume(&duplicate.id).unwrap());
        assert!(!db.is_session_autonomy_paused(&session.id).unwrap());
        let (_, _, stale_workflows, _) = db
            .resolve_local_autonomy_stop_fences(
                &[],
                &[],
                &[],
                &[(workflow.id.clone(), session.id.clone(), 0)],
            )
            .expect("resolve workflow Stop generation after fast Continue");
        assert_eq!(stale_workflows, vec![workflow.id.clone()]);
        assert_eq!(
            db.list_pending_session_autonomy_resume_replays(10)
                .unwrap()
                .into_iter()
                .map(|pause| pause.id)
                .collect::<Vec<_>>(),
            vec![duplicate.id.clone()]
        );

        let second = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("second receipt");
        assert_ne!(second.id, duplicate.id);
        assert_eq!(db.session_autonomy_pause_epoch(&session.id).unwrap(), 3);
        assert!(db
            .list_pending_session_autonomy_resume_replays(10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn no_replay_parent_delivery_keeps_root_in_global_stop_enumeration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("delivery-pause.db"))
            .expect("session db");
        let root = db.create_session("ha-main").expect("root session");
        let child = db
            .create_session_with_parent("helper", Some(&root.id))
            .expect("child session");
        let run = crate::subagent::SubagentRun {
            run_id: "run-pending-delivery".into(),
            thread_id: child.id.clone(),
            parent_session_id: root.id.clone(),
            parent_agent_id: "ha-main".into(),
            child_agent_id: "helper".into(),
            child_session_id: child.id,
            task: "return a durable result".into(),
            status: crate::subagent::SubagentStatus::Running,
            result: None,
            error: None,
            depth: 1,
            model_used: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: None,
            duration_ms: None,
            label: None,
            attachment_count: 0,
            input_tokens: None,
            output_tokens: None,
            continuation_of_run_id: None,
            trigger_kind: "spawn".into(),
            terminal_reason: None,
            runner_owner: None,
            lease_epoch: 1,
            last_heartbeat_at: None,
            delivery_kind: crate::subagent::SubagentDeliveryKind::Parent,
            launch_spec_json: None,
            owner_kind: crate::subagent::SubagentOwnerKind::ParentSession,
            owner_id: root.id.clone(),
        };
        db.insert_subagent_run(&run).expect("insert subagent run");
        db.update_subagent_status(
            &run.run_id,
            crate::subagent::SubagentStatus::Completed,
            Some("durable child result"),
            None,
            None,
            Some(1),
        )
        .expect("complete subagent run");
        assert!(db
            .claim_subagent_result_delivery(&run.run_id)
            .expect("claim delivery"));
        db.arm_subagent_result_delivery_no_replay(&run.run_id)
            .expect("arm no-replay delivery");

        assert_eq!(
            db.list_session_ids_with_active_autonomy().unwrap(),
            vec![root.id.clone()]
        );
        let pause = db
            .prepare_session_autonomy_pause(&root.id)
            .expect("pause receipt");
        assert_eq!(pause.subagent_run_ids, vec![run.run_id.clone()]);
        assert!(db.finish_session_autonomy_resume(&pause.id).unwrap());
        let (stale_injections, captured_subagents, _, _) = db
            .resolve_local_autonomy_stop_fences(
                &[],
                &[(root.id.clone(), 0)],
                std::slice::from_ref(&run.run_id),
                &[],
            )
            .expect("resolved Stop survives fast Continue");
        assert_eq!(stale_injections, vec![root.id.clone()]);
        assert_eq!(captured_subagents, vec![run.run_id.clone()]);

        let conn = db.conn.lock().expect("session db lock");
        let delivery = conn
            .query_row(
                "SELECT state, suppress_reason
                   FROM subagent_result_deliveries
                  WHERE run_id = ?1",
                params![run.run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .expect("delivery row");
        assert_eq!(delivery.0, "injecting_no_replay");
        assert_eq!(
            delivery.1.as_deref(),
            Some("session_continue_uses_runtime_recovery")
        );
    }

    #[test]
    fn running_remote_foreground_stream_keeps_root_in_global_stop_enumeration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("foreground-stop.db"))
            .expect("session db");
        let root = db.create_session("ha-main").expect("root session");
        let child = db
            .create_session_with_parent("helper", Some(&root.id))
            .expect("child session");
        let run_id = "remote-acp-stream".to_string();
        let registration = db
            .create_stream_run(&crate::session::CreateStreamRun {
                run_id: run_id.clone(),
                session_id: child.id.clone(),
                source: "acp".to_string(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .expect("foreground stream admission");

        assert_eq!(registration.admitted_stop_epoch, 0);
        assert_eq!(
            db.list_session_ids_with_active_autonomy().unwrap(),
            vec![root.id.clone()]
        );
        let pause = db
            .prepare_session_autonomy_pause(&root.id)
            .expect("global Stop receipt");
        assert!(db.finish_session_autonomy_resume(&pause.id).unwrap());
        let (_, _, _, stale_foreground_runs) = db
            .resolve_local_autonomy_stop_fences(
                &[(
                    run_id.clone(),
                    child.id.clone(),
                    registration.admitted_stop_epoch,
                    registration.admitted_global_stop_epoch,
                    registration.admitted_global_stop_receipt_count,
                )],
                &[],
                &[],
                &[],
            )
            .expect("resolve foreground Stop generation");
        assert_eq!(stale_foreground_runs, vec![run_id]);
    }

    #[test]
    fn attributed_global_stop_receipts_landing_after_admission_do_not_cancel_new_turn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("global-stop-race.db"))
            .expect("session db");
        let session = db.create_session("ha-main").expect("session");
        let (older_global_stop_epoch, _) = db
            .begin_global_stop_enumeration()
            .expect("publish older global Stop generation");
        let (global_stop_epoch, _) = db
            .begin_global_stop_enumeration()
            .expect("publish current global Stop generation");
        let registration = db
            .create_stream_run(&crate::session::CreateStreamRun {
                run_id: "new-foreground-after-global-stop".to_string(),
                session_id: session.id.clone(),
                source: "http".to_string(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .expect("foreground stream admission");
        db.prepare_session_autonomy_pause_for_global(&session.id, older_global_stop_epoch)
            .expect("late older-generation global receipt");
        db.prepare_session_autonomy_pause_for_global(&session.id, global_stop_epoch)
            .expect("same-generation global receipt");

        let foreground = vec![(
            registration.run_id.clone(),
            session.id.clone(),
            registration.admitted_stop_epoch,
            registration.admitted_global_stop_epoch,
            registration.admitted_global_stop_receipt_count,
        )];
        let (_, _, _, stale_foreground_runs) = db
            .resolve_local_autonomy_stop_fences(&foreground, &[], &[], &[])
            .expect("resolve attributed global receipts");
        assert!(stale_foreground_runs.is_empty());

        db.prepare_session_autonomy_pause(&session.id)
            .expect("newer targeted Stop receipt");
        let (_, _, _, stale_foreground_runs) = db
            .resolve_local_autonomy_stop_fences(&foreground, &[], &[], &[])
            .expect("resolve newer targeted Stop");
        assert_eq!(
            stale_foreground_runs,
            vec![registration.run_id],
            "a targeted Stop after admission must still win"
        );
    }

    #[test]
    fn foreground_admission_rejects_new_stop_but_accepts_its_late_global_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("foreground-admission.db"))
            .expect("session db");
        let session = db.create_session("ha-main").expect("session");

        let before_target = db
            .foreground_stop_admission(Some(&session.id))
            .expect("capture targeted admission");
        let target = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("targeted Stop");
        assert!(db.finish_session_autonomy_resume(&target.id).unwrap());
        assert!(!db
            .foreground_stop_admission_is_current(&session.id, before_target)
            .unwrap());

        let (global_epoch, _) = db
            .begin_global_stop_enumeration()
            .expect("publish global generation");
        let after_global_epoch = db
            .foreground_stop_admission(Some(&session.id))
            .expect("capture after global generation");
        db.prepare_session_autonomy_pause_for_global(&session.id, global_epoch)
            .expect("late same-generation receipt");
        assert!(db
            .foreground_stop_admission_is_current(&session.id, after_global_epoch)
            .unwrap());
    }

    #[test]
    fn one_global_stop_generation_has_one_active_pause_id_per_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("global-stop-idempotent.db"))
            .expect("session db");
        let session = db.create_session("ha-main").expect("session");
        let (global_stop_epoch, _) = db
            .begin_global_stop_enumeration()
            .expect("publish global Stop generation");

        let first = db
            .prepare_session_autonomy_pause_for_global(&session.id, global_stop_epoch)
            .expect("first owner publishes receipt");
        let repeated = db
            .prepare_session_autonomy_pause_for_global(&session.id, global_stop_epoch)
            .expect("second owner converges on receipt");

        assert_eq!(repeated.id, first.id);
        assert_eq!(db.session_autonomy_pause_epoch(&session.id).unwrap(), 1);
        assert!(db.finish_session_autonomy_resume(&first.id).unwrap());
        assert_eq!(
            db.session_autonomy_resume_global_stop_epoch(&first.id)
                .unwrap(),
            global_stop_epoch
        );
    }

    #[test]
    fn global_stop_holds_scheduled_rows_until_the_exact_pause_is_continued() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("scheduled-stop.db"))
            .expect("session db");
        let session = db.create_session("ha-main").expect("session");
        let admission = db
            .foreground_stop_admission(Some(&session.id))
            .expect("scheduled admission");
        db.enqueue_scheduled_turn_message(
            crate::session::NewScheduledTurnMessage {
                request_id: "scheduled-before-stop".to_string(),
                session_id: session.id.clone(),
                source_ref: "101".to_string(),
                message: "run after Continue".to_string(),
            },
            admission,
        )
        .expect("enqueue scheduled row");
        let cancellable_admission = db
            .foreground_stop_admission(Some(&session.id))
            .expect("second scheduled admission");
        db.enqueue_scheduled_turn_message(
            crate::session::NewScheduledTurnMessage {
                request_id: "scheduled-cancelled-while-held".to_string(),
                session_id: session.id.clone(),
                source_ref: "102".to_string(),
                message: "cancel me".to_string(),
            },
            cancellable_admission,
        )
        .expect("enqueue cancellable scheduled row");

        let (global_epoch, enumerated) = db
            .begin_global_stop_enumeration()
            .expect("publish global Stop and hold queues");
        assert_eq!(enumerated, vec![session.id.clone()]);
        assert_eq!(
            db.get_scheduled_turn_message("101")
                .unwrap()
                .expect("held scheduled row")
                .status,
            crate::session::QueuedTurnMessageStatus::HeldAfterStop
        );
        assert!(db
            .claim_scheduled_turn_message_for_dispatch(
                "scheduled-before-stop",
                "101",
                "turn-before-receipt",
            )
            .unwrap()
            .is_none());
        assert!(db
            .cancel_scheduled_turn_message("scheduled-cancelled-while-held", "102")
            .expect("cancel held scheduled row"));

        let global_pause = db
            .prepare_session_autonomy_pause_for_global(&session.id, global_epoch)
            .expect("global pause receipt");
        let newer_pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("newer targeted pause receipt");
        assert!(!db
            .finish_session_autonomy_resume(&global_pause.id)
            .expect("stale Continue"));
        assert_eq!(
            db.get_scheduled_turn_message("101")
                .unwrap()
                .expect("still-held scheduled row")
                .status,
            crate::session::QueuedTurnMessageStatus::HeldAfterStop
        );

        assert!(db
            .finish_session_autonomy_resume(&newer_pause.id)
            .expect("exact Continue"));
        assert_eq!(
            db.get_scheduled_turn_message("101")
                .unwrap()
                .expect("resumed scheduled row")
                .status,
            crate::session::QueuedTurnMessageStatus::Queued
        );
        assert!(db
            .claim_scheduled_turn_message_for_dispatch(
                "scheduled-before-stop",
                "101",
                "turn-after-continue",
            )
            .expect("claim resumed scheduled row")
            .is_some());
    }

    #[test]
    fn global_stop_atomically_supersedes_an_old_pause_before_releasing_scheduled_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("global-stop-old-continue.db");
        let stop_owner = SessionDB::open_ephemeral_for_test(&path).expect("Stop owner DB");
        let continue_owner = SessionDB::open_ephemeral_for_test(&path).expect("Continue owner DB");
        let session = stop_owner.create_session("ha-main").expect("session");
        let admission = stop_owner
            .foreground_stop_admission(Some(&session.id))
            .expect("scheduled admission");
        stop_owner
            .enqueue_scheduled_turn_message(
                crate::session::NewScheduledTurnMessage {
                    request_id: "scheduled-before-new-global-stop".to_string(),
                    session_id: session.id.clone(),
                    source_ref: "201".to_string(),
                    message: "remain held for the new generation".to_string(),
                },
                admission,
            )
            .expect("enqueue scheduled row");
        let old_pause = stop_owner
            .prepare_session_autonomy_pause(&session.id)
            .expect("old targeted pause");

        let (global_epoch, enumerated) = stop_owner
            .begin_global_stop_enumeration()
            .expect("publish Global Stop");
        assert_eq!(enumerated, vec![session.id.clone()]);

        assert!(!continue_owner
            .finish_session_autonomy_resume(&old_pause.id)
            .expect("old cross-process Continue loses"));
        assert_eq!(
            continue_owner
                .get_scheduled_turn_message("201")
                .unwrap()
                .expect("held scheduled row")
                .status,
            crate::session::QueuedTurnMessageStatus::HeldAfterStop
        );

        let new_pause = continue_owner
            .prepare_session_autonomy_pause_for_global(&session.id, global_epoch)
            .expect("new exact Global Stop receipt");
        assert_ne!(new_pause.id, old_pause.id);
        assert!(continue_owner
            .finish_session_autonomy_resume(&new_pause.id)
            .expect("new cross-process Continue wins"));
        assert_eq!(
            continue_owner
                .get_scheduled_turn_message("201")
                .unwrap()
                .expect("resumed scheduled row")
                .status,
            crate::session::QueuedTurnMessageStatus::Queued
        );
    }

    #[test]
    fn global_stop_supersedes_a_pending_continue_handoff_across_connections() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("global-stop-pending-continue.db");
        let stop_owner = SessionDB::open_ephemeral_for_test(&path).expect("Stop owner DB");
        let continue_owner = SessionDB::open_ephemeral_for_test(&path).expect("Continue owner DB");
        let session = stop_owner.create_session("ha-main").expect("session");
        let created_pause = stop_owner
            .prepare_session_autonomy_pause(&session.id)
            .expect("old pause");
        stop_owner
            .with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE session_autonomy_pauses
                        SET goal_id = 'goal-snapshot',
                            workflow_run_ids_json = '[\"workflow-snapshot\"]',
                            subagent_run_ids_json = '[\"subagent-snapshot\"]'
                      WHERE id = ?1",
                    params![created_pause.id],
                )?;
                Ok(())
            })
            .expect("seed captured controller snapshot");
        let old_pause = stop_owner
            .active_session_autonomy_pause(&session.id)
            .expect("load old pause")
            .expect("old pause remains active");
        assert!(continue_owner
            .finish_session_autonomy_resume(&old_pause.id)
            .expect("publish Continue handoff"));
        assert_eq!(
            stop_owner
                .list_pending_session_autonomy_resume_replays(10)
                .expect("pending handoff")
                .iter()
                .map(|pause| pause.id.as_str())
                .collect::<Vec<_>>(),
            vec![old_pause.id.as_str()]
        );

        let (global_epoch, enumerated) = stop_owner
            .begin_global_stop_enumeration()
            .expect("publish newer Global Stop");
        assert_eq!(enumerated, vec![session.id.clone()]);
        assert!(continue_owner
            .list_pending_session_autonomy_resume_replays(10)
            .expect("old replay suppressed")
            .is_empty());
        assert!(!continue_owner
            .finish_session_autonomy_resume_replay(&old_pause.id)
            .expect("old replay cannot acknowledge"));

        let current_pause = continue_owner
            .prepare_session_autonomy_pause_for_global(&session.id, global_epoch)
            .expect("current Global Stop receipt");
        assert_ne!(current_pause.id, old_pause.id);
        assert_eq!(current_pause.goal_id, old_pause.goal_id);
        assert_eq!(current_pause.workflow_run_ids, old_pause.workflow_run_ids);
        assert_eq!(current_pause.subagent_run_ids, old_pause.subagent_run_ids);
        assert!(continue_owner
            .finish_session_autonomy_resume(&current_pause.id)
            .expect("Continue current receipt"));
        assert_eq!(
            stop_owner
                .list_pending_session_autonomy_resume_replays(10)
                .expect("current replay handoff")
                .iter()
                .map(|pause| pause.id.as_str())
                .collect::<Vec<_>>(),
            vec![current_pause.id.as_str()]
        );
    }
}
