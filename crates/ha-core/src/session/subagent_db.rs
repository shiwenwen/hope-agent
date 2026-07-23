use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::db::SessionDB;

const SUBAGENT_RUN_COLUMNS: &str =
    "run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id,
    task, status, result, error, depth, model_used, started_at, finished_at, duration_ms,
    label, attachment_count, input_tokens, output_tokens, continuation_of_run_id, trigger_kind,
    terminal_reason, runner_owner, lease_epoch, last_heartbeat_at, delivery_kind, launch_spec_json,
    owner_kind, owner_id";

fn insert_run_row(
    tx: &rusqlite::Transaction<'_>,
    run: &crate::subagent::SubagentRun,
    lease_epoch: u64,
) -> Result<()> {
    let owner_id = if run.owner_id.is_empty()
        && run.owner_kind == crate::subagent::SubagentOwnerKind::ParentSession
    {
        run.parent_session_id.as_str()
    } else {
        run.owner_id.as_str()
    };
    tx.execute(
        "INSERT INTO subagent_runs (run_id, parent_session_id, parent_agent_id, child_agent_id,
            child_session_id, task, status, result, error, depth, model_used, started_at, finished_at,
            duration_ms, label, attachment_count, input_tokens, output_tokens,
            continuation_of_run_id, trigger_kind, terminal_reason, runner_owner, lease_epoch,
            last_heartbeat_at, delivery_kind, launch_spec_json, owner_kind, owner_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)",
        params![
            run.run_id,
            run.parent_session_id,
            run.parent_agent_id,
            run.child_agent_id,
            run.child_session_id,
            run.task,
            run.status.as_str(),
            run.result,
            run.error,
            run.depth,
            run.model_used,
            run.started_at,
            run.finished_at,
            run.duration_ms.map(|d| d as i64),
            run.label,
            run.attachment_count,
            run.input_tokens.map(|v| v as i64),
            run.output_tokens.map(|v| v as i64),
            run.continuation_of_run_id,
            run.trigger_kind,
            run.terminal_reason.map(|reason| reason.as_str()),
            run.runner_owner,
            lease_epoch as i64,
            run.last_heartbeat_at,
            run.delivery_kind.as_str(),
            run.launch_spec_json,
            run.owner_kind.as_str(),
            owner_id,
        ],
    )?;
    Ok(())
}

impl SessionDB {
    // ── Sub-Agent Run CRUD ──────────────────────────────────────

    /// Insert a new sub-agent run record.
    pub fn insert_subagent_run(&self, run: &crate::subagent::SubagentRun) -> Result<()> {
        if !run.thread_id.is_empty() && run.thread_id != run.child_session_id {
            return Err(anyhow::anyhow!(
                "Sub-agent thread_id must equal child_session_id"
            ));
        }
        if run.owner_id.is_empty()
            && run.owner_kind != crate::subagent::SubagentOwnerKind::ParentSession
        {
            return Err(anyhow::anyhow!("Sub-agent thread owner_id cannot be empty"));
        }
        if run.continuation_of_run_id.is_some() {
            return Err(anyhow::anyhow!(
                "Continuation runs must use insert_resumed_subagent_run"
            ));
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let lease_epoch = run.lease_epoch.max(1);
        let thread_id = run.child_session_id.as_str();
        let owner_id = if run.owner_id.is_empty() {
            run.parent_session_id.as_str()
        } else {
            run.owner_id.as_str()
        };
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO subagent_threads (
                thread_id, parent_session_id, parent_agent_id, child_agent_id, depth,
                owner_kind, owner_id, lifecycle_state, current_run_id, lease_epoch,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, ?9, ?10, ?10)",
            params![
                thread_id,
                run.parent_session_id,
                run.parent_agent_id,
                run.child_agent_id,
                run.depth,
                run.owner_kind.as_str(),
                owner_id,
                run.run_id,
                lease_epoch as i64,
                run.started_at,
            ],
        )?;
        if inserted == 0 {
            return Err(anyhow::anyhow!(
                "Sub-agent thread '{}' already exists; use a serialized continuation",
                thread_id
            ));
        }
        insert_run_row(&tx, run, lease_epoch)?;
        if run.owner_kind == crate::subagent::SubagentOwnerKind::Workflow {
            let projected = tx.execute(
                "INSERT INTO workflow_agent_attempts (
                    workflow_run_id, thread_id, run_id, source_op_id,
                    continuation_of_run_id, role, control_mode, resolution_state,
                    created_at
                 )
                 SELECT ?1, ?2, ?3, wo.id, NULL, 'initial', 'control',
                        CASE WHEN ?4 = 'completed' THEN 'resolved' ELSE 'pending' END,
                        ?5
                   FROM workflow_ops wo
                  WHERE wo.run_id = ?1
                    AND wo.child_handle = ?3
                    AND wo.op_type = 'spawnAgent'
                  LIMIT 1",
                params![
                    run.owner_id,
                    run.thread_id,
                    run.run_id,
                    run.status.as_str(),
                    run.started_at,
                ],
            )?;
            if projected != 1 {
                return Err(anyhow::anyhow!(
                    "Workflow-owned sub-agent run '{}' has no durable spawnAgent op",
                    run.run_id
                ));
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert a continuation run for an existing sub-agent child session.
    ///
    /// A resumed child gets a fresh immutable run row while reusing the source
    /// run's `child_session_id` (and therefore its conversation context and
    /// working directory). The source row must already be terminal, and the
    /// child session may have at most one queued/active run. Both checks and the
    /// insert live in one transaction so two concurrent resume requests cannot
    /// start overlapping turns against the same conversation history.
    pub fn insert_resumed_subagent_run(
        &self,
        source_run_id: &str,
        run: &crate::subagent::SubagentRun,
        dispatch_id: Option<&str>,
        dispatch_message: Option<&str>,
    ) -> Result<u64> {
        if dispatch_id.is_some() != dispatch_message.is_some() {
            return Err(anyhow::anyhow!(
                "Resume dispatch id and message must be provided together"
            ));
        }
        if run.thread_id != run.child_session_id {
            return Err(anyhow::anyhow!(
                "Sub-agent thread_id must equal child_session_id"
            ));
        }
        if run.continuation_of_run_id.as_deref() != Some(source_run_id) {
            return Err(anyhow::anyhow!(
                "Continuation provenance does not match source sub-agent run '{}'",
                source_run_id
            ));
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;

        let source: Option<(String, String, String, String, u32, String, String, String)> = tx
            .query_row(
                "SELECT parent_session_id, parent_agent_id, child_agent_id,
                        child_session_id, depth, status, owner_kind, owner_id
                   FROM subagent_runs WHERE run_id = ?1",
                params![source_run_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            source_parent,
            source_parent_agent,
            source_child_agent,
            source_child,
            depth,
            status,
            source_owner_kind,
            source_owner_id,
        )) = source
        else {
            return Err(anyhow::anyhow!(
                "Sub-agent run '{}' not found",
                source_run_id
            ));
        };
        let source_status = crate::subagent::SubagentStatus::from_str(&status);
        if !source_status.is_terminal() {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent run '{}': it is still '{}'",
                source_run_id,
                source_status.as_str()
            ));
        }
        if run.parent_session_id != source_parent
            || run.parent_agent_id != source_parent_agent
            || run.child_agent_id != source_child_agent
            || run.child_session_id != source_child
            || run.depth != depth
            || run.owner_kind.as_str() != source_owner_kind
            || run.owner_id != source_owner_id
        {
            return Err(anyhow::anyhow!(
                "Resume run identity does not match source sub-agent run '{}'",
                source_run_id
            ));
        }

        let thread: Option<(String, String, String, i64)> = tx
            .query_row(
                "SELECT lifecycle_state, owner_kind, owner_id, lease_epoch
                   FROM subagent_threads
                  WHERE thread_id = ?1 AND current_run_id = ?2",
                params![run.thread_id, source_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((lifecycle_state, thread_owner_kind, thread_owner_id, current_epoch)) = thread
        else {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent run '{}': it is not the current attempt",
                source_run_id
            ));
        };
        if lifecycle_state != "open" {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent thread '{}': lifecycle state is '{}'",
                run.thread_id,
                lifecycle_state
            ));
        }
        if thread_owner_kind != source_owner_kind || thread_owner_id != source_owner_id {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent thread '{}': owner provenance is inconsistent",
                run.thread_id
            ));
        }

        let child_identity: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT agent_id, parent_session_id FROM sessions WHERE id = ?1",
                params![run.child_session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if child_identity
            .as_ref()
            .map(|(agent_id, parent_id)| (agent_id.as_str(), parent_id.as_deref()))
            != Some((
                run.child_agent_id.as_str(),
                Some(run.parent_session_id.as_str()),
            ))
        {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent run '{}': child session identity is invalid",
                source_run_id
            ));
        }

        let nonterminal: i64 = tx.query_row(
            "SELECT COUNT(*) FROM subagent_runs
              WHERE child_session_id = ?1
                AND status IN ('queued', 'spawning', 'running')",
            params![run.child_session_id],
            |row| row.get(0),
        )?;
        if nonterminal > 0 {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent run '{}': child session already has an active continuation",
                source_run_id
            ));
        }

        let next_epoch = current_epoch.saturating_add(1).max(1) as u64;
        insert_run_row(&tx, run, next_epoch)?;
        if run.owner_kind == crate::subagent::SubagentOwnerKind::Workflow {
            let projected = tx.execute(
                "INSERT INTO workflow_agent_attempts (
                    workflow_run_id, thread_id, run_id, source_op_id,
                    continuation_of_run_id, role, control_mode, resolution_state,
                    created_at
                 )
                 SELECT ?1, ?2, ?3, wo.id, ?4, 'continuation', 'control',
                        CASE WHEN ?5 = 'completed' THEN 'resolved' ELSE 'pending' END,
                        ?6
                   FROM workflow_ops wo
                  WHERE wo.run_id = ?1
                    AND wo.child_handle = ?3
                    AND wo.op_type = 'resumeAgent'
                  LIMIT 1",
                params![
                    run.owner_id,
                    run.thread_id,
                    run.run_id,
                    source_run_id,
                    run.status.as_str(),
                    run.started_at,
                ],
            )?;
            if projected != 1 {
                return Err(anyhow::anyhow!(
                    "Workflow-owned continuation '{}' has no durable resumeAgent op",
                    run.run_id
                ));
            }
        }
        let updated = tx.execute(
            "UPDATE subagent_threads
                SET current_run_id = ?1, lease_epoch = ?2, updated_at = ?3
              WHERE thread_id = ?4
                AND current_run_id = ?5
                AND lease_epoch = ?6
                AND lifecycle_state = 'open'",
            params![
                run.run_id,
                next_epoch as i64,
                run.started_at,
                run.thread_id,
                source_run_id,
                current_epoch,
            ],
        )?;
        if updated != 1 {
            return Err(anyhow::anyhow!(
                "Cannot resume sub-agent run '{}': thread changed concurrently",
                source_run_id
            ));
        }
        if let (Some(dispatch_id), Some(dispatch_message)) = (dispatch_id, dispatch_message) {
            tx.execute(
                "INSERT INTO subagent_dispatches (
                    id, thread_id, source_run_id, target_run_id, dispatch_kind,
                    owner_kind, owner_id, message, state, created_at, delivered_at
                 ) VALUES (?1, ?2, ?3, ?4, 'resume', ?5, ?6, ?7, 'consumed', ?8, ?8)",
                params![
                    dispatch_id,
                    run.thread_id,
                    source_run_id,
                    run.run_id,
                    run.owner_kind.as_str(),
                    run.owner_id,
                    dispatch_message,
                    run.started_at,
                ],
            )?;
        }
        // The continuation explicitly consumes the predecessor. If an ordinary
        // parent delivery was still pending/in-flight, it must not later create
        // a duplicate parent turn after this follow-up already used the result.
        tx.execute(
            "UPDATE subagent_result_deliveries
                SET state = 'suppressed', suppress_reason = 'explicitly_continued',
                    delivered_at = ?1
              WHERE run_id = ?2 AND state IN ('pending', 'injecting')",
            params![run.started_at, source_run_id],
        )?;
        tx.commit()?;
        Ok(next_epoch)
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
        use crate::subagent::{SubagentStatus, SubagentTerminalReason};

        let terminal_reason = match &status {
            SubagentStatus::Completed => Some(SubagentTerminalReason::Success),
            SubagentStatus::Timeout => Some(SubagentTerminalReason::DeadlineExceeded),
            SubagentStatus::Killed => Some(SubagentTerminalReason::UserKilled),
            SubagentStatus::Interrupted => Some(SubagentTerminalReason::ProcessInterrupted),
            SubagentStatus::Error => Some(SubagentTerminalReason::Unknown),
            SubagentStatus::Queued | SubagentStatus::Spawning | SubagentStatus::Running => None,
        };
        self.update_subagent_status_with_reason(
            run_id,
            status,
            terminal_reason,
            result,
            error,
            model_used,
            duration_ms,
        )
    }

    /// Fenced lifecycle update with a stable terminal classification.
    ///
    /// The SQL joins the attempt's epoch to the thread's current epoch/run id.
    /// A stale worker can therefore neither resurrect an old attempt nor enqueue
    /// a duplicate parent delivery after a continuation took ownership.
    #[allow(clippy::too_many_arguments)]
    pub fn update_subagent_status_with_reason(
        &self,
        run_id: &str,
        status: crate::subagent::SubagentStatus,
        terminal_reason: Option<crate::subagent::SubagentTerminalReason>,
        result: Option<&str>,
        error: Option<&str>,
        model_used: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let changed = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let tx = conn.transaction()?;
            let changed = tx.execute(
                "UPDATE subagent_runs
                    SET status = ?1,
                        result = COALESCE(?2, result),
                        error = COALESCE(?3, error),
                        model_used = COALESCE(?4, model_used),
                        duration_ms = COALESCE(?5, duration_ms),
                        terminal_reason = COALESCE(?6, terminal_reason),
                        last_heartbeat_at = ?7,
                        finished_at = CASE
                            WHEN ?9 = 1 THEN COALESCE(finished_at, ?7)
                            ELSE finished_at
                        END
                  WHERE run_id = ?8
                    AND (status IN ('queued', 'spawning', 'running') OR status = ?1)
                    AND EXISTS (
                        SELECT 1 FROM subagent_threads st
                         WHERE st.thread_id = subagent_runs.child_session_id
                           AND st.current_run_id = subagent_runs.run_id
                           AND st.lease_epoch = subagent_runs.lease_epoch
                    )",
                params![
                    status.as_str(),
                    result,
                    error,
                    model_used,
                    duration_ms.map(|d| d as i64),
                    terminal_reason.map(|reason| reason.as_str()),
                    now,
                    run_id,
                    if status.is_terminal() { 1_i64 } else { 0_i64 },
                ],
            )?;
            if changed > 0 {
                tx.execute(
                    "UPDATE subagent_threads
                        SET updated_at = ?1
                      WHERE current_run_id = ?2",
                    params![now, run_id],
                )?;
                if status.is_terminal()
                    && !matches!(&status, crate::subagent::SubagentStatus::Killed)
                {
                    tx.execute(
                        "INSERT OR IGNORE INTO subagent_result_deliveries (
                            run_id, parent_session_id, state, attempt_count, requested_at
                         )
                         SELECT run_id, parent_session_id, 'pending', 0, ?1
                           FROM subagent_runs
                          WHERE run_id = ?2
                            AND delivery_kind = 'parent'
                            AND owner_kind = 'parent_session'
                            AND EXISTS (
                                SELECT 1 FROM sessions s
                                 WHERE s.id = subagent_runs.parent_session_id
                                   AND s.incognito = 0
                            )",
                        params![now, run_id],
                    )?;
                }
                if matches!(&status, crate::subagent::SubagentStatus::Completed) {
                    // A successful continuation resolves every earlier failed
                    // attempt in this Workflow-owned thread. V4 ignores this
                    // projection; V5 finish guards consume it.
                    tx.execute(
                        "UPDATE workflow_agent_attempts
                            SET resolution_state = 'resolved',
                                resolved_by_run_id = ?1
                          WHERE control_mode = 'control'
                            AND resolution_state = 'pending'
                            AND (workflow_run_id, thread_id) IN (
                                SELECT workflow_run_id, thread_id
                                  FROM workflow_agent_attempts
                                 WHERE run_id = ?1
                            )",
                        params![run_id],
                    )?;
                }
            }
            tx.commit()?;
            changed
        }; // drop the SessionDB lock before the cross-DB projection sync below.
        if changed == 0 {
            // Fencing is a successful no-op from the caller's perspective: the
            // authoritative newer attempt must continue undisturbed.
            return Ok(());
        }
        // R6: this is the single status choke point, so mirroring here keeps the
        // `background_jobs` subagent projection in lockstep with the truth source
        // for ALL transition paths (run lifecycle + the three kill fallbacks).
        // Best-effort + no-op when the run was never projected (foreground /
        // internal / incognito) — and it NEVER writes run content back.
        let became_terminal = status.is_terminal();
        crate::async_jobs::JobManager::sync_subagent_projection(run_id, status.clone());
        // R7.2: a terminal status may have freed a per-session concurrency slot —
        // wake the subagent scheduler to promote any parked (`Queued`) spawn.
        if became_terminal {
            crate::subagent::queue::wake_subagent_scheduler();
        }
        crate::workflow::on_workflow_child_status_changed(self, run_id, status);
        Ok(())
    }

    /// Persist token usage for a completed sub-agent run. This is intentionally
    /// separate from status transitions so kill/error paths can remain lightweight.
    pub fn set_subagent_usage(
        &self,
        run_id: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE subagent_runs
             SET input_tokens = COALESCE(?1, input_tokens),
                 output_tokens = COALESCE(?2, output_tokens)
             WHERE run_id = ?3",
            params![
                input_tokens.map(|v| v as i64),
                output_tokens.map(|v| v as i64),
                run_id,
            ],
        )?;
        Ok(())
    }

    pub fn set_subagent_delivery_kind(
        &self,
        run_id: &str,
        delivery_kind: crate::subagent::SubagentDeliveryKind,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE subagent_runs
                SET delivery_kind = ?1
              WHERE run_id = ?2
                AND status IN ('queued', 'spawning', 'running')",
            params![delivery_kind.as_str(), run_id],
        )?;
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
                "UPDATE subagent_runs
                    SET status = ?1,
                        last_heartbeat_at = ?4
                  WHERE run_id = ?2 AND status = ?3
                    AND EXISTS (
                        SELECT 1 FROM subagent_threads st
                         WHERE st.thread_id = subagent_runs.child_session_id
                           AND st.current_run_id = subagent_runs.run_id
                           AND st.lease_epoch = subagent_runs.lease_epoch
                    )",
                params![
                    to.as_str(),
                    run_id,
                    from.as_str(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )?
        }; // drop the SessionDB lock before the cross-DB projection sync below.
        if changed > 0 {
            let became_terminal = to.is_terminal();
            crate::async_jobs::JobManager::sync_subagent_projection(run_id, to.clone());
            if became_terminal {
                crate::subagent::queue::wake_subagent_scheduler();
            }
            crate::workflow::on_workflow_child_status_changed(self, run_id, to);
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
        let sql = format!("SELECT {SUBAGENT_RUN_COLUMNS} FROM subagent_runs WHERE run_id = ?1");
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![run_id], Self::row_to_subagent_run)?;
        match rows.next() {
            Some(Ok(run)) => Ok(Some(run)),
            Some(Err(e)) => Err(anyhow::anyhow!("DB error: {}", e)),
            None => Ok(None),
        }
    }

    /// Load the stable control record for a child conversation.
    pub fn get_subagent_thread(
        &self,
        thread_id: &str,
    ) -> Result<Option<crate::subagent::SubagentThread>> {
        use crate::subagent::{SubagentOwnerKind, SubagentThread, SubagentThreadState};

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT thread_id, parent_session_id, parent_agent_id, child_agent_id,
                    depth, owner_kind, owner_id, lifecycle_state, current_run_id,
                    lease_epoch, created_at, updated_at
               FROM subagent_threads
              WHERE thread_id = ?1",
            params![thread_id],
            |row| {
                Ok(SubagentThread {
                    thread_id: row.get(0)?,
                    parent_session_id: row.get(1)?,
                    parent_agent_id: row.get(2)?,
                    child_agent_id: row.get(3)?,
                    depth: row.get(4)?,
                    owner_kind: SubagentOwnerKind::from_str(&row.get::<_, String>(5)?),
                    owner_id: row.get(6)?,
                    lifecycle_state: SubagentThreadState::from_str(&row.get::<_, String>(7)?),
                    current_run_id: row.get(8)?,
                    lease_epoch: row.get::<_, i64>(9)?.max(0) as u64,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_current_subagent_run(
        &self,
        thread_id: &str,
    ) -> Result<Option<crate::subagent::SubagentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let sql = format!(
            "SELECT {SUBAGENT_RUN_COLUMNS}
               FROM subagent_runs
              WHERE run_id = (
                    SELECT current_run_id
                      FROM subagent_threads
                     WHERE thread_id = ?1
              )"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![thread_id], Self::row_to_subagent_run)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Persist a steer request only while the exact run remains the live
    /// attempt owned by the caller. The mailbox is merely the low-latency
    /// transport; this row is the durable provenance and replay truth.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_subagent_steer_dispatch(
        &self,
        dispatch_id: &str,
        thread_id: &str,
        source_run_id: &str,
        owner_kind: crate::subagent::SubagentOwnerKind,
        owner_id: &str,
        message: &str,
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let valid: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1
                  FROM subagent_threads st
                  JOIN subagent_runs sr ON sr.run_id = st.current_run_id
                 WHERE st.thread_id = ?1
                   AND st.current_run_id = ?2
                   AND st.owner_kind = ?3
                   AND st.owner_id = ?4
                   AND st.lifecycle_state = 'open'
                   AND sr.status IN ('queued', 'spawning', 'running')
                   AND sr.lease_epoch = st.lease_epoch
             )",
            params![thread_id, source_run_id, owner_kind.as_str(), owner_id],
            |row| row.get(0),
        )?;
        if !valid {
            return Err(anyhow::anyhow!(
                "Sub-agent thread changed or is not steerable"
            ));
        }
        tx.execute(
            "INSERT INTO subagent_dispatches (
                id, thread_id, source_run_id, target_run_id, dispatch_kind,
                owner_kind, owner_id, message, state, created_at
             ) VALUES (?1, ?2, ?3, ?3, 'steer', ?4, ?5, ?6, 'accepted', ?7)",
            params![
                dispatch_id,
                thread_id,
                source_run_id,
                owner_kind.as_str(),
                owner_id,
                message,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_subagent_dispatch_delivered(&self, dispatch_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE subagent_dispatches
                SET state = 'delivered', delivered_at = ?1
              WHERE id = ?2 AND state = 'accepted'",
            params![chrono::Utc::now().to_rfc3339(), dispatch_id],
        )?;
        Ok(())
    }

    pub fn mark_subagent_dispatch_refused(&self, dispatch_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE subagent_dispatches SET state = 'refused'
              WHERE id = ?1 AND state = 'accepted'",
            params![dispatch_id],
        )?;
        Ok(())
    }

    pub fn list_accepted_subagent_dispatches(&self, run_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, message
               FROM subagent_dispatches
              WHERE target_run_id = ?1 AND state = 'accepted'
              ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map(params![run_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Durable replacement for the process-local fetched set. The in-memory
    /// bit remains as a fast cancellation signal, but restart behavior reads
    /// this state.
    pub fn suppress_subagent_result_delivery(&self, run_id: &str, reason: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO subagent_result_deliveries (
                run_id, parent_session_id, state, suppress_reason, attempt_count,
                requested_at, delivered_at
             )
             SELECT run_id, parent_session_id, 'suppressed', ?1, 0, ?2, ?2
               FROM subagent_runs
              WHERE run_id = ?3
                AND delivery_kind = 'parent'
                AND owner_kind = 'parent_session'
                AND EXISTS (
                    SELECT 1 FROM sessions s
                     WHERE s.id = subagent_runs.parent_session_id
                       AND s.incognito = 0
                )
             ON CONFLICT(run_id) DO UPDATE SET
                state = 'suppressed',
                suppress_reason = excluded.suppress_reason,
                delivered_at = excluded.delivered_at
              WHERE subagent_result_deliveries.state IN ('pending', 'injecting')",
            params![reason, now, run_id],
        )?;
        Ok(())
    }

    pub fn claim_subagent_result_delivery(&self, run_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE subagent_result_deliveries
                SET state = 'injecting', attempt_count = attempt_count + 1,
                    last_error = NULL
              WHERE run_id = ?1 AND state = 'pending'",
            params![run_id],
        )?;
        Ok(changed == 1)
    }

    pub fn mark_subagent_result_delivered(&self, run_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE subagent_result_deliveries
                SET state = 'delivered', delivered_at = ?1, last_error = NULL
              WHERE run_id = ?2 AND state IN ('pending', 'injecting')",
            params![chrono::Utc::now().to_rfc3339(), run_id],
        )?;
        Ok(())
    }

    pub fn reset_and_list_pending_subagent_deliveries(
        &self,
    ) -> Result<Vec<crate::subagent::SubagentRun>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE subagent_result_deliveries
                SET state = 'pending', last_error = 'Interrupted during parent delivery'
              WHERE state = 'injecting'",
            [],
        )?;
        let sql = format!(
            "SELECT {SUBAGENT_RUN_COLUMNS}
               FROM subagent_runs
              WHERE run_id IN (
                    SELECT run_id
                      FROM subagent_result_deliveries
                     WHERE state = 'pending'
              )
              ORDER BY (
                    SELECT requested_at
                      FROM subagent_result_deliveries
                     WHERE subagent_result_deliveries.run_id = subagent_runs.run_id
              )"
        );
        let runs = {
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt.query_map([], Self::row_to_subagent_run)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(runs)
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
            "SELECT {SUBAGENT_RUN_COLUMNS} FROM subagent_runs WHERE run_id IN ({placeholders})"
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
        let sql = format!(
            "SELECT {SUBAGENT_RUN_COLUMNS}
               FROM subagent_runs
              WHERE parent_session_id = ?1
              ORDER BY started_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
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
        let sql = format!(
            "SELECT {SUBAGENT_RUN_COLUMNS}
               FROM subagent_runs
              WHERE parent_session_id = ?1 AND status IN ('spawning', 'running')
              ORDER BY started_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
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
        let sql = format!(
            "SELECT {SUBAGENT_RUN_COLUMNS}
               FROM subagent_runs
              WHERE child_session_id = ?1 AND status IN ('spawning', 'running')
              ORDER BY started_at DESC LIMIT 1"
        );
        let mut stmt = conn.prepare(&sql)?;
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
        let sql = format!(
            "SELECT {SUBAGENT_RUN_COLUMNS}
               FROM subagent_runs
              WHERE status IN ('spawning', 'running')
              ORDER BY started_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_subagent_run)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// Count queued or active runs involving an Agent on either side.
    pub fn count_nonterminal_subagent_runs_for_agent(&self, agent_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM subagent_runs
             WHERE status IN ('queued','spawning','running')
               AND (parent_agent_id=?1 OR child_agent_id=?1)",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Collect the transitive set of subagent CHILD session ids descended from
    /// `root_session_id` (walking `subagent_runs.parent_session_id →
    /// child_session_id`). Session delete/purge calls this BEFORE the cascade
    /// drops `subagent_runs`, so the cleanup fan-out can deny inner-tool
    /// approvals parked on those child sessions (G4): an inner approval keys on
    /// the child session, which the deleted parent's id can't match. Bounded by a
    /// visited set (no cycles in practice — a child can't be its own ancestor)
    /// plus a hard cap as a defensive backstop.
    pub fn collect_descendant_session_ids(&self, root_session_id: &str) -> Vec<String> {
        use std::collections::HashSet;
        const MAX_DESCENDANTS: usize = 4096;
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let Ok(mut stmt) =
            conn.prepare("SELECT child_session_id FROM subagent_runs WHERE parent_session_id = ?1")
        else {
            return Vec::new();
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut frontier = vec![root_session_id.to_string()];
        let mut out = Vec::new();
        while let Some(parent) = frontier.pop() {
            if out.len() >= MAX_DESCENDANTS {
                break;
            }
            let Ok(rows) = stmt.query_map(params![parent], |row| row.get::<_, String>(0)) else {
                continue;
            };
            for child in rows.flatten() {
                if seen.insert(child.clone()) {
                    frontier.push(child.clone());
                    out.push(child);
                }
            }
        }
        out
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

    /// Mark all non-terminal sub-agent runs as interrupted (orphan cleanup on startup).
    /// Includes `queued` (R7.2): a parked run's in-memory queue entry is lost on
    /// restart, so the row must settle (mirrors the tool-job `Queued→Interrupted`
    /// recovery) rather than linger forever as a phantom queued run.
    pub fn cleanup_orphan_subagent_runs(&self) -> Result<usize> {
        let now = chrono::Utc::now().to_rfc3339();
        let run_ids = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let tx = conn.transaction()?;
            let run_ids = {
                let mut stmt = tx.prepare(
                    "SELECT sr.run_id
                       FROM subagent_runs sr
                       JOIN subagent_threads st ON st.thread_id = sr.child_session_id
                      WHERE sr.status IN ('queued', 'spawning', 'running')
                        AND st.current_run_id = sr.run_id
                        AND st.lease_epoch = sr.lease_epoch",
                )?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            for run_id in &run_ids {
                tx.execute(
                    "UPDATE subagent_runs
                        SET status = 'interrupted',
                            terminal_reason = 'process_interrupted',
                            error = COALESCE(error, 'Interrupted: app restarted before completion'),
                            finished_at = COALESCE(finished_at, ?1),
                            last_heartbeat_at = ?1
                      WHERE run_id = ?2",
                    params![now, run_id],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO subagent_result_deliveries (
                        run_id, parent_session_id, state, attempt_count, requested_at
                     )
                     SELECT run_id, parent_session_id, 'pending', 0, ?1
                       FROM subagent_runs
                      WHERE run_id = ?2
                        AND delivery_kind = 'parent'
                        AND owner_kind = 'parent_session'
                        AND EXISTS (
                            SELECT 1 FROM sessions s
                             WHERE s.id = subagent_runs.parent_session_id
                               AND s.incognito = 0
                        )",
                    params![now, run_id],
                )?;
            }
            tx.commit()?;
            run_ids
        };
        for run_id in &run_ids {
            crate::async_jobs::JobManager::sync_subagent_projection(
                run_id,
                crate::subagent::SubagentStatus::Interrupted,
            );
            crate::workflow::on_workflow_child_status_changed(
                self,
                run_id,
                crate::subagent::SubagentStatus::Interrupted,
            );
        }
        if !run_ids.is_empty() {
            crate::subagent::queue::wake_subagent_scheduler();
        }
        Ok(run_ids.len())
    }

    pub(crate) fn row_to_subagent_run(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<crate::subagent::SubagentRun> {
        use crate::subagent::{
            SubagentDeliveryKind, SubagentOwnerKind, SubagentStatus, SubagentTerminalReason,
        };
        let duration_val: Option<i64> = row.get(13)?;
        let input_tokens_val: Option<i64> = row.get(16)?;
        let output_tokens_val: Option<i64> = row.get(17)?;
        Ok(crate::subagent::SubagentRun {
            run_id: row.get(0)?,
            thread_id: row.get(4)?,
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
            continuation_of_run_id: row.get(18)?,
            trigger_kind: row.get(19)?,
            terminal_reason: row
                .get::<_, Option<String>>(20)?
                .map(|value| SubagentTerminalReason::from_str(&value)),
            runner_owner: row.get(21)?,
            lease_epoch: row.get::<_, i64>(22)?.max(0) as u64,
            last_heartbeat_at: row.get(23)?,
            delivery_kind: SubagentDeliveryKind::from_str(&row.get::<_, String>(24)?),
            launch_spec_json: row.get(25)?,
            owner_kind: SubagentOwnerKind::from_str(&row.get::<_, String>(26)?),
            owner_id: row.get(27)?,
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
            thread_id: child_session.into(),
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
            continuation_of_run_id: None,
            trigger_kind: "spawn".into(),
            terminal_reason: None,
            runner_owner: None,
            lease_epoch: 1,
            last_heartbeat_at: None,
            delivery_kind: crate::subagent::SubagentDeliveryKind::Parent,
            launch_spec_json: None,
            owner_kind: crate::subagent::SubagentOwnerKind::ParentSession,
            owner_id: "parent".into(),
        }
    }

    #[test]
    fn find_active_run_by_child_session_matches_only_active_runs() {
        // R8 follow-up: maps an inner-tool approval's child session → the active
        // run whose projection should reflect AwaitingApproval. Terminal runs and
        // other sessions must not match (their projection is already settled).
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open_ephemeral_for_test(&tmp.path().join("s.db")).unwrap();
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
    fn resumed_run_reuses_child_but_keeps_source_terminal_and_serializes_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db.create_session("ha-main").unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut source = run("run-source", &child.id, SubagentStatus::Completed);
        source.parent_session_id = parent.id.clone();
        source.owner_id = parent.id.clone();
        db.insert_subagent_run(&source).unwrap();

        let mut resumed = run("run-next", &child.id, SubagentStatus::Spawning);
        resumed.parent_session_id = parent.id.clone();
        resumed.owner_id = parent.id.clone();
        resumed.task = "follow up".into();
        resumed.continuation_of_run_id = Some("run-source".into());
        db.insert_resumed_subagent_run("run-source", &resumed, None, None)
            .unwrap();

        assert_eq!(
            db.get_subagent_run("run-source").unwrap().unwrap().status,
            SubagentStatus::Completed,
            "resume must not resurrect the immutable source run"
        );
        assert_eq!(
            db.get_subagent_run("run-next")
                .unwrap()
                .unwrap()
                .child_session_id,
            child.id
        );

        let mut overlapping = run("run-overlap", &child.id, SubagentStatus::Queued);
        overlapping.parent_session_id = parent.id.clone();
        overlapping.owner_id = parent.id;
        overlapping.continuation_of_run_id = Some("run-source".into());
        let error = db
            .insert_resumed_subagent_run("run-source", &overlapping, None, None)
            .expect_err("one child conversation cannot run two continuation turns");
        assert!(
            error.to_string().contains("current attempt")
                || error.to_string().contains("active continuation")
        );
    }

    #[test]
    fn continuation_epoch_fences_late_writes_from_the_previous_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db.create_session("ha-main").unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut source = run("run-source", &child.id, SubagentStatus::Completed);
        source.parent_session_id = parent.id.clone();
        source.owner_id = parent.id.clone();
        source.result = Some("authoritative result".into());
        db.insert_subagent_run(&source).unwrap();

        let mut continuation = run("run-next", &child.id, SubagentStatus::Spawning);
        continuation.parent_session_id = parent.id.clone();
        continuation.owner_id = parent.id;
        continuation.continuation_of_run_id = Some(source.run_id.clone());
        db.insert_resumed_subagent_run(&source.run_id, &continuation, None, None)
            .unwrap();

        // A delayed callback from the old runner may repeat the same terminal
        // status. Without the thread/current-run + epoch fence it could still
        // overwrite the immutable predecessor's result after continuation.
        db.update_subagent_status(
            &source.run_id,
            SubagentStatus::Completed,
            Some("late stale result"),
            None,
            None,
            Some(99),
        )
        .unwrap();
        let persisted = db.get_subagent_run(&source.run_id).unwrap().unwrap();
        assert_eq!(persisted.result.as_deref(), Some("authoritative result"));
        assert_eq!(
            db.get_current_subagent_run(&child.id)
                .unwrap()
                .unwrap()
                .run_id,
            "run-next"
        );
    }

    #[test]
    fn ordinary_terminal_delivery_is_claimed_once_and_can_be_suppressed_durably() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db.create_session("ha-main").unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut delivery = run("run-delivery", &child.id, SubagentStatus::Running);
        delivery.parent_session_id = parent.id.clone();
        delivery.owner_id = parent.id;
        db.insert_subagent_run(&delivery).unwrap();
        db.update_subagent_status(
            "run-delivery",
            SubagentStatus::Completed,
            Some("done"),
            None,
            None,
            Some(1),
        )
        .unwrap();

        assert!(db.claim_subagent_result_delivery("run-delivery").unwrap());
        assert!(
            !db.claim_subagent_result_delivery("run-delivery").unwrap(),
            "the durable CAS must admit only one injector"
        );
        db.suppress_subagent_result_delivery("run-delivery", "explicitly_consumed")
            .unwrap();
        assert!(db
            .reset_and_list_pending_subagent_deliveries()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn startup_cleanup_classifies_live_attempts_as_interrupted_and_replayable() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db.create_session("ha-main").unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut orphan_run = run("run-orphan", &child.id, SubagentStatus::Running);
        orphan_run.parent_session_id = parent.id.clone();
        orphan_run.owner_id = parent.id;
        db.insert_subagent_run(&orphan_run).unwrap();

        assert_eq!(db.cleanup_orphan_subagent_runs().unwrap(), 1);
        let orphan = db.get_subagent_run("run-orphan").unwrap().unwrap();
        assert_eq!(orphan.status, SubagentStatus::Interrupted);
        assert_eq!(
            orphan.terminal_reason,
            Some(crate::subagent::SubagentTerminalReason::ProcessInterrupted)
        );
        let replay = db.reset_and_list_pending_subagent_deliveries().unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].run_id, "run-orphan");
    }

    #[test]
    fn startup_cleanup_never_replays_incognito_parent_delivery() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db
            .create_session_with_project("ha-main", None, Some(true))
            .unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut orphan_run = run("run-incognito", &child.id, SubagentStatus::Running);
        orphan_run.parent_session_id = parent.id.clone();
        orphan_run.owner_id = parent.id;
        db.insert_subagent_run(&orphan_run).unwrap();

        assert_eq!(db.cleanup_orphan_subagent_runs().unwrap(), 1);
        assert!(db
            .reset_and_list_pending_subagent_deliveries()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pending_parent_delivery_and_thread_identity_survive_database_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.db");
        let (parent_id, child_id) = {
            let db = SessionDB::open(&path).unwrap();
            let parent = db.create_session("ha-main").unwrap();
            let child = db
                .create_session_with_parent("helper", Some(&parent.id))
                .unwrap();
            let mut persisted = run("run-reopen", &child.id, SubagentStatus::Running);
            persisted.parent_session_id = parent.id.clone();
            persisted.owner_id = parent.id.clone();
            db.insert_subagent_run(&persisted).unwrap();
            db.update_subagent_status(
                &persisted.run_id,
                SubagentStatus::Completed,
                Some("done"),
                None,
                None,
                Some(1),
            )
            .unwrap();
            (parent.id, child.id)
        };

        let reopened = SessionDB::open(&path).unwrap();
        let thread = reopened
            .get_subagent_thread(&child_id)
            .unwrap()
            .expect("thread survives reopen");
        assert_eq!(thread.parent_session_id, parent_id);
        assert_eq!(thread.current_run_id.as_deref(), Some("run-reopen"));
        let replay = reopened
            .reset_and_list_pending_subagent_deliveries()
            .unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].thread_id, child_id);
    }

    #[test]
    fn continuation_cannot_change_thread_owner_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db.create_session("ha-main").unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut source = run("run-source", &child.id, SubagentStatus::Completed);
        source.parent_session_id = parent.id.clone();
        source.owner_id = parent.id.clone();
        db.insert_subagent_run(&source).unwrap();

        let mut continuation = run("run-next", &child.id, SubagentStatus::Spawning);
        continuation.parent_session_id = parent.id;
        continuation.continuation_of_run_id = Some(source.run_id.clone());
        continuation.owner_kind = crate::subagent::SubagentOwnerKind::Workflow;
        continuation.owner_id = "wfr_takeover".into();
        let error = db
            .insert_resumed_subagent_run(&source.run_id, &continuation, None, None)
            .expect_err("a continuation must retain the stable thread owner");
        assert!(error.to_string().contains("identity does not match"));
    }

    #[test]
    fn resume_rejects_a_nonterminal_source() {
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open(&tmp.path().join("s.db")).unwrap();
        let parent = db.create_session("ha-main").unwrap();
        let child = db
            .create_session_with_parent("helper", Some(&parent.id))
            .unwrap();
        let mut source = run("run-running", &child.id, SubagentStatus::Running);
        source.parent_session_id = parent.id.clone();
        source.owner_id = parent.id.clone();
        db.insert_subagent_run(&source).unwrap();
        let mut next = run("run-next", &child.id, SubagentStatus::Spawning);
        next.parent_session_id = parent.id.clone();
        next.owner_id = parent.id;
        next.continuation_of_run_id = Some("run-running".into());
        let error = db
            .insert_resumed_subagent_run("run-running", &next, None, None)
            .expect_err("a running child must be steered, not resumed");
        assert!(error.to_string().contains("still 'running'"));
    }

    #[test]
    fn collect_descendant_session_ids_walks_transitively() {
        // G4: deleting a parent must reach inner-tool approvals parked on its
        // transitive subagent child sessions. root → childA, root → childB;
        // childA → grandchild.
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open_ephemeral_for_test(&tmp.path().join("s.db")).unwrap();
        let mk = |run_id: &str, parent: &str, child: &str| SubagentRun {
            run_id: run_id.into(),
            thread_id: child.into(),
            parent_session_id: parent.into(),
            parent_agent_id: "ha-main".into(),
            child_agent_id: "helper".into(),
            child_session_id: child.into(),
            task: "t".into(),
            status: SubagentStatus::Running,
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
            owner_id: parent.into(),
        };
        db.insert_subagent_run(&mk("r1", "root", "childA")).unwrap();
        db.insert_subagent_run(&mk("r2", "root", "childB")).unwrap();
        db.insert_subagent_run(&mk("r3", "childA", "grandchild"))
            .unwrap();

        let mut got = db.collect_descendant_session_ids("root");
        got.sort();
        assert_eq!(got, vec!["childA", "childB", "grandchild"]);

        // A leaf with no children → empty (and no infinite walk).
        assert!(db.collect_descendant_session_ids("grandchild").is_empty());
        assert!(db.collect_descendant_session_ids("unknown").is_empty());
    }

    #[test]
    fn try_transition_subagent_status_is_a_guarded_cas() {
        // R7.2 promote-vs-cancel core guarantee: `Queued → Spawning` is a CAS
        // that fires at most once and NEVER resurrects a row a concurrent cancel
        // already stamped terminal.
        let tmp = tempfile::tempdir().unwrap();
        let db = SessionDB::open_ephemeral_for_test(&tmp.path().join("s.db")).unwrap();
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
