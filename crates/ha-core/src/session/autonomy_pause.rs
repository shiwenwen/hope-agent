use std::collections::HashSet;

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::SessionDB;

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

impl SessionDB {
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
                  WHERE state IN ('pending', 'injecting')
                 UNION
                 SELECT session_id FROM chat_stream_runs
                  WHERE status = 'running'
                    AND source IN ('desktop', 'http', 'channel', 'acp')
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

    /// Capture the exact autonomous controllers owned by a session and publish
    /// a durable pause fence before any in-process cancellation is attempted.
    ///
    /// Every Stop replaces the prior active receipt with a fresh generation.
    /// Captured controller ids are carried forward so repeatedly pressing Stop
    /// cannot make already-paused work impossible to resume, while a Continue
    /// bound to the older id can no longer consume the newer user decision.
    pub fn prepare_session_autonomy_pause(&self, session_id: &str) -> Result<SessionAutonomyPause> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction()?;

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
                             WHERE state IN ('pending', 'injecting')
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
                subagent_run_ids_json, created_at, resumed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![
                pause.id,
                pause.session_id,
                pause.goal_id,
                serde_json::to_string(&pause.workflow_run_ids)?,
                serde_json::to_string(&pause.subagent_run_ids)?,
                pause.created_at,
            ],
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

    /// Resolve durable Stop generations against process-local runtime
    /// snapshots. Pause rows are never deleted, so a fast Continue cannot hide
    /// an older generation from an injection or immutable sub-agent/workflow
    /// attempt that was admitted before that Stop.
    pub(crate) fn resolve_local_autonomy_stop_fences(
        &self,
        foreground_generations: &[(String, String, u64)],
        injection_generations: &[(String, u64)],
        subagent_run_ids: &[String],
        workflow_generations: &[(String, String, u64)],
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>, Vec<String>)> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
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
        for (run_id, session_id, admitted_epoch) in foreground_generations {
            let epoch = epoch_stmt
                .query_row(params![session_id], |row| row.get::<_, i64>(0))?
                .max(0) as u64;
            if epoch > *admitted_epoch {
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
        let tx = conn.transaction()?;
        let subagent_ids = tx
            .query_row(
                "SELECT subagent_run_ids_json
                   FROM session_autonomy_pauses
                  WHERE id = ?1 AND resumed_at IS NULL",
                params![pause_id],
                |row| parse_ids(row.get(0)?),
            )
            .optional()?;
        let Some(subagent_ids) = subagent_ids else {
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
                    AND state IN ('pending', 'injecting')",
                params![now, run_id],
            )?;
        }
        let changed = tx.execute(
            "UPDATE session_autonomy_pauses
                SET resumed_at = ?1,
                    resume_requested_at = ?1,
                    resume_replayed_at = NULL,
                    resume_replay_error = NULL
              WHERE id = ?2 AND resumed_at IS NULL",
            params![now, pause_id],
        )?;
        tx.commit()?;
        Ok(changed > 0)
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
    fn pending_parent_delivery_keeps_root_in_global_stop_enumeration() {
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
        db.release_subagent_result_delivery_claim(&run.run_id, "provider unavailable")
            .expect("release delivery claim");
        assert!(db
            .defer_subagent_result_delivery_retry(&run.run_id, "provider unavailable", 3, 30)
            .expect("defer delivery retry"));

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
        assert_eq!(delivery.0, "suppressed");
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
                )],
                &[],
                &[],
                &[],
            )
            .expect("resolve foreground Stop generation");
        assert_eq!(stale_foreground_runs, vec![run_id]);
    }
}
