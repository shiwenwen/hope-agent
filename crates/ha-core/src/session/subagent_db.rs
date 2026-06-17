use anyhow::Result;
use rusqlite::params;

use super::db::SessionDB;

impl SessionDB {
    // ── Sub-Agent Run CRUD ──────────────────────────────────────

    /// Insert a new sub-agent run record.
    pub fn insert_subagent_run(&self, run: &crate::subagent::SubagentRun) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO subagent_runs (run_id, parent_session_id, parent_agent_id, child_agent_id,
                child_session_id, task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                label, attachment_count, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                run.run_id, run.parent_session_id, run.parent_agent_id,
                run.child_agent_id, run.child_session_id, run.task,
                run.status.as_str(), run.result, run.error, run.depth,
                run.model_used, run.started_at, run.finished_at, run.duration_ms.map(|d| d as i64),
                run.label, run.attachment_count, run.input_tokens.map(|v| v as i64), run.output_tokens.map(|v| v as i64),
            ],
        )?;
        Ok(())
    }

    /// Update a sub-agent run's status, result, error, model_used, and duration.
    pub fn update_subagent_status(
        &self,
        run_id: &str,
        status: crate::subagent::SubagentStatus,
        result: Option<&str>,
        error: Option<&str>,
        model_used: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE subagent_runs SET status = ?1, result = COALESCE(?2, result),
                error = COALESCE(?3, error), model_used = COALESCE(?4, model_used),
                duration_ms = COALESCE(?5, duration_ms)
             WHERE run_id = ?6",
                params![
                    status.as_str(),
                    result,
                    error,
                    model_used,
                    duration_ms.map(|d| d as i64),
                    run_id,
                ],
            )?;
        } // drop the SessionDB lock before the cross-DB projection sync below.
          // R6: this is the single status choke point, so mirroring here keeps the
          // `background_jobs` subagent projection in lockstep with the truth source
          // for ALL transition paths (run lifecycle + the three kill fallbacks).
          // Best-effort + no-op when the run was never projected (foreground /
          // internal / incognito) — and it NEVER writes run content back.
        let became_terminal = status.is_terminal();
        crate::async_jobs::JobManager::sync_subagent_projection(run_id, status);
        // R7.2: a terminal status may have freed a per-session concurrency slot —
        // wake the subagent scheduler to promote any parked (`Queued`) spawn.
        if became_terminal {
            crate::subagent::queue::wake_subagent_scheduler();
        }
        Ok(())
    }

    /// Guarded status transition: write `to` only when the row is currently
    /// `from`. Returns `Ok(true)` iff a row was updated. The R7.2 promoter uses
    /// this to flip `Queued → Spawning` atomically so it loses cleanly to a
    /// concurrent cancel (which stamps the row terminal): a no-op transition
    /// (`Ok(false)`) means the row already moved off `Queued`, so the promoter
    /// must NOT launch — otherwise a killed run would be resurrected into a
    /// running child. On a real transition it keeps the `background_jobs`
    /// projection in lockstep, exactly like [`update_subagent_status`].
    pub fn try_transition_subagent_status(
        &self,
        run_id: &str,
        from: crate::subagent::SubagentStatus,
        to: crate::subagent::SubagentStatus,
    ) -> Result<bool> {
        let changed = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE subagent_runs SET status = ?1 WHERE run_id = ?2 AND status = ?3",
                params![to.as_str(), run_id, from.as_str()],
            )?
        }; // drop the SessionDB lock before the cross-DB projection sync below.
        if changed > 0 {
            let became_terminal = to.is_terminal();
            crate::async_jobs::JobManager::sync_subagent_projection(run_id, to);
            if became_terminal {
                crate::subagent::queue::wake_subagent_scheduler();
            }
        }
        Ok(changed > 0)
    }

    /// Set the finished_at timestamp for a sub-agent run.
    pub fn set_subagent_finished_at(&self, run_id: &str, finished_at: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE subagent_runs SET finished_at = ?1 WHERE run_id = ?2",
            params![finished_at, run_id],
        )?;
        Ok(())
    }

    /// Get a single sub-agent run by ID.
    pub fn get_subagent_run(&self, run_id: &str) -> Result<Option<crate::subagent::SubagentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
                    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                    label, attachment_count, input_tokens, output_tokens
             FROM subagent_runs WHERE run_id = ?1"
        )?;
        let mut rows = stmt.query_map(params![run_id], Self::row_to_subagent_run)?;
        match rows.next() {
            Some(Ok(run)) => Ok(Some(run)),
            Some(Err(e)) => Err(anyhow::anyhow!("DB error: {}", e)),
            None => Ok(None),
        }
    }

    /// Batch variant of [`get_subagent_run`]. Returns a `HashMap` keyed by
    /// `run_id` so callers can look up by id without index coupling. Missing
    /// ids simply don't appear in the map.
    pub fn get_subagent_runs_batch(
        &self,
        run_ids: &[String],
    ) -> Result<std::collections::HashMap<String, crate::subagent::SubagentRun>> {
        if run_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let placeholders = crate::sql_in_placeholders(run_ids.len());
        let sql = format!(
            "SELECT run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
                    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                    label, attachment_count, input_tokens, output_tokens
             FROM subagent_runs WHERE run_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_dyn: Vec<&dyn rusqlite::ToSql> =
            run_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_dyn.as_slice(), Self::row_to_subagent_run)?;

        let mut out = std::collections::HashMap::with_capacity(run_ids.len());
        for row in rows {
            let run = row?;
            out.insert(run.run_id.clone(), run);
        }
        Ok(out)
    }

    /// List all sub-agent runs for a parent session, ordered by started_at DESC.
    pub fn list_subagent_runs(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<crate::subagent::SubagentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
                    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                    label, attachment_count, input_tokens, output_tokens
             FROM subagent_runs WHERE parent_session_id = ?1 ORDER BY started_at DESC"
        )?;
        let rows = stmt.query_map(params![parent_session_id], Self::row_to_subagent_run)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// List active (non-terminal) sub-agent runs for a parent session.
    pub fn list_active_subagent_runs(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<crate::subagent::SubagentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
                    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                    label, attachment_count, input_tokens, output_tokens
             FROM subagent_runs
             WHERE parent_session_id = ?1 AND status IN ('spawning', 'running')
             ORDER BY started_at DESC"
        )?;
        let rows = stmt.query_map(params![parent_session_id], Self::row_to_subagent_run)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// R8 follow-up: the active (`spawning`/`running`) sub-agent run whose CHILD
    /// session is `child_session_id`. An inner-tool approval event carries the
    /// child session that requested it; this maps that back to the run whose
    /// Background Job projection should reflect `AwaitingApproval`. Each active
    /// run owns a distinct child session, so the result is 0-or-1; terminal runs
    /// are excluded (their projection is already settled and must not reopen).
    pub fn find_active_run_by_child_session(
        &self,
        child_session_id: &str,
    ) -> Result<Option<crate::subagent::SubagentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
                    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                    label, attachment_count, input_tokens, output_tokens
             FROM subagent_runs
             WHERE child_session_id = ?1 AND status IN ('spawning', 'running')
             ORDER BY started_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![child_session_id], Self::row_to_subagent_run)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// List all active (non-terminal) sub-agent runs.
    pub fn list_all_active_subagent_runs(&self) -> Result<Vec<crate::subagent::SubagentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
                    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
                    label, attachment_count, input_tokens, output_tokens
             FROM subagent_runs
             WHERE status IN ('spawning', 'running')
             ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_subagent_run)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// Count active sub-agent runs for a parent session.
    pub fn count_active_subagent_runs(&self, parent_session_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM subagent_runs WHERE parent_session_id = ?1 AND status IN ('spawning', 'running')",
            params![parent_session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Mark all non-terminal sub-agent runs as error (orphan cleanup on startup).
    /// Includes `queued` (R7.2): a parked run's in-memory queue entry is lost on
    /// restart, so the row must settle (mirrors the tool-job `Queued→Interrupted`
    /// recovery) rather than linger forever as a phantom queued run.
    pub fn cleanup_orphan_subagent_runs(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let affected = conn.execute(
            "UPDATE subagent_runs SET status = 'error', error = 'Orphaned: app restarted before completion', finished_at = ?1
             WHERE status IN ('queued', 'spawning', 'running')",
            params![now],
        )?;
        Ok(affected)
    }

    pub(crate) fn row_to_subagent_run(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<crate::subagent::SubagentRun> {
        use crate::subagent::SubagentStatus;
        let duration_val: Option<i64> = row.get(13)?;
        let input_tokens_val: Option<i64> = row.get(16)?;
        let output_tokens_val: Option<i64> = row.get(17)?;
        Ok(crate::subagent::SubagentRun {
            run_id: row.get(0)?,
            parent_session_id: row.get(1)?,
            parent_agent_id: row.get(2)?,
            child_agent_id: row.get(3)?,
            child_session_id: row.get(4)?,
            task: row.get(5)?,
            status: SubagentStatus::from_str(&row.get::<_, String>(6)?),
            result: row.get(7)?,
            error: row.get(8)?,
            depth: row.get::<_, u32>(9)?,
            model_used: row.get(10)?,
            started_at: row.get(11)?,
            finished_at: row.get(12)?,
            duration_ms: duration_val.map(|v| v as u64),
            label: row.get(14)?,
            attachment_count: row.get::<_, u32>(15).unwrap_or(0),
            input_tokens: input_tokens_val.map(|v| v as u64),
            output_tokens: output_tokens_val.map(|v| v as u64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SessionDB;
    use crate::subagent::{SubagentRun, SubagentStatus};

    fn run(run_id: &str, child_session: &str, status: SubagentStatus) -> SubagentRun {
        SubagentRun {
            run_id: run_id.into(),
            parent_session_id: "parent".into(),
            parent_agent_id: "ha-main".into(),
            child_agent_id: "helper".into(),
            child_session_id: child_session.into(),
            task: "t".into(),
            status,
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
        }
    }

    #[test]
    fn find_active_run_by_child_session_matches_only_active_runs() {
        // R8 follow-up: maps an inner-tool approval's child session → the active
        // run whose projection should reflect AwaitingApproval. Terminal runs and
        // other sessions must not match (their projection is already settled).
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        db.insert_subagent_run(&run("run-A", "child-A", SubagentStatus::Running))
            .unwrap();
        db.insert_subagent_run(&run("run-S", "child-S", SubagentStatus::Spawning))
            .unwrap();
        db.insert_subagent_run(&run("run-done", "child-done", SubagentStatus::Completed))
            .unwrap();

        assert_eq!(
            db.find_active_run_by_child_session("child-A")
                .unwrap()
                .unwrap()
                .run_id,
            "run-A"
        );
        // Spawning counts as active (the run can already hit an inner approval).
        assert_eq!(
            db.find_active_run_by_child_session("child-S")
                .unwrap()
                .unwrap()
                .run_id,
            "run-S"
        );
        // Terminal run is excluded.
        assert!(db
            .find_active_run_by_child_session("child-done")
            .unwrap()
            .is_none());
        // Unknown child session (e.g. a foreground turn / R8 background exec whose
        // approval carries its parent session) → None.
        assert!(db
            .find_active_run_by_child_session("child-nope")
            .unwrap()
            .is_none());
    }

    #[test]
    fn try_transition_subagent_status_is_a_guarded_cas() {
        // R7.2 promote-vs-cancel core guarantee: `Queued → Spawning` is a CAS
        // that fires at most once and NEVER resurrects a row a concurrent cancel
        // already stamped terminal.
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        db.insert_subagent_run(&run("run-q", "child-q", SubagentStatus::Queued))
            .unwrap();

        // First Queued → Spawning succeeds and moves the row.
        assert!(db
            .try_transition_subagent_status(
                "run-q",
                SubagentStatus::Queued,
                SubagentStatus::Spawning
            )
            .unwrap());
        assert_eq!(
            db.get_subagent_run("run-q").unwrap().unwrap().status,
            SubagentStatus::Spawning
        );
        // A second promote attempt is a no-op (row no longer Queued) — the
        // promoter must not relaunch.
        assert!(!db
            .try_transition_subagent_status(
                "run-q",
                SubagentStatus::Queued,
                SubagentStatus::Spawning
            )
            .unwrap());

        // A concurrent cancel stamped the row terminal: Queued → Spawning must
        // NOT resurrect it (the bug this fix closes).
        db.insert_subagent_run(&run("run-k", "child-k", SubagentStatus::Killed))
            .unwrap();
        assert!(!db
            .try_transition_subagent_status(
                "run-k",
                SubagentStatus::Queued,
                SubagentStatus::Spawning
            )
            .unwrap());
        assert_eq!(
            db.get_subagent_run("run-k").unwrap().unwrap().status,
            SubagentStatus::Killed,
            "a killed run must stay killed — never resurrected into Spawning"
        );
    }
}
