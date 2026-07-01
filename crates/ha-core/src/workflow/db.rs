use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::session::SessionDB;

use super::events;
use super::types::{
    CreateWorkflowRunInput, StartedOpRecoveryAction, UpsertWorkflowOpInput, WorkflowEffectClass,
    WorkflowEvent, WorkflowOp, WorkflowOpState, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState,
};

const EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            state TEXT NOT NULL,
            execution_mode TEXT NOT NULL,
            script_hash TEXT NOT NULL,
            script_source TEXT NOT NULL,
            budget_json TEXT NOT NULL DEFAULT '{}',
            cursor_seq INTEGER NOT NULL DEFAULT 0,
            primary_owner TEXT,
            blocked_reason TEXT,
            parent_run_id TEXT,
            origin TEXT,
            goal_id TEXT,
            worktree_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (parent_run_id) REFERENCES workflow_runs(id) ON DELETE SET NULL,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS workflow_ops (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            op_key TEXT NOT NULL,
            op_type TEXT NOT NULL,
            effect_class TEXT NOT NULL,
            input_hash TEXT NOT NULL,
            input_json TEXT NOT NULL,
            state TEXT NOT NULL,
            output_json TEXT,
            error_json TEXT,
            child_handle TEXT,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            FOREIGN KEY (run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE,
            UNIQUE(run_id, op_key)
        );

        CREATE TABLE IF NOT EXISTS workflow_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            type TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE,
            UNIQUE(run_id, seq)
        );

        CREATE INDEX IF NOT EXISTS idx_workflow_runs_session_updated
            ON workflow_runs(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_state
            ON workflow_runs(state);
        CREATE INDEX IF NOT EXISTS idx_workflow_ops_run
            ON workflow_ops(run_id, op_key);
        CREATE INDEX IF NOT EXISTS idx_workflow_ops_state
            ON workflow_ops(state);
        CREATE INDEX IF NOT EXISTS idx_workflow_events_run_seq
            ON workflow_events(run_id, seq);",
    )?;
    if conn
        .prepare("SELECT parent_run_id FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN parent_run_id TEXT;")?;
    }
    if conn
        .prepare("SELECT origin FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN origin TEXT;")?;
    }
    if conn
        .prepare("SELECT goal_id FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN goal_id TEXT;")?;
    }
    if conn
        .prepare("SELECT worktree_id FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN worktree_id TEXT;")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_workflow_runs_parent
            ON workflow_runs(parent_run_id);",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_workflow_runs_goal
            ON workflow_runs(goal_id, updated_at DESC);",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_workflow_runs_worktree
            ON workflow_runs(worktree_id);",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn create_workflow_run(&self, input: CreateWorkflowRunInput) -> Result<WorkflowRun> {
        let now = now_rfc3339();
        let id = format!("wfr_{}", uuid::Uuid::new_v4().simple());
        let script_hash = blake3_hex(input.script_source.as_bytes());
        let budget_json = stable_json(&input.budget)?;
        let goal_id = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let incognito: Option<i64> = conn
                .query_row(
                    "SELECT incognito FROM sessions WHERE id = ?1",
                    params![input.session_id],
                    |row| row.get(0),
                )
                .optional()?;
            let incognito =
                incognito.ok_or_else(|| anyhow!("Session not found: {}", input.session_id))?;
            if incognito != 0 {
                return Err(anyhow!(
                    "Cannot create durable workflow run for incognito session {}",
                    input.session_id
                ));
            }
            if let Some(parent_run_id) = input.parent_run_id.as_deref() {
                let parent_session_id: Option<String> = conn
                    .query_row(
                        "SELECT session_id FROM workflow_runs WHERE id = ?1",
                        params![parent_run_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let parent_session_id = parent_session_id
                    .ok_or_else(|| anyhow!("parent workflow run not found: {parent_run_id}"))?;
                if parent_session_id != input.session_id {
                    return Err(anyhow!(
                        "parent workflow run {} belongs to session {}; expected {}",
                        parent_run_id,
                        parent_session_id,
                        input.session_id
                    ));
                }
            }
            match input.goal_id.as_deref() {
                Some(goal_id) => {
                    let goal_session_id: Option<String> = conn
                        .query_row(
                            "SELECT session_id FROM goals WHERE id = ?1",
                            params![goal_id],
                            |row| row.get(0),
                        )
                        .optional()?;
                    let goal_session_id =
                        goal_session_id.ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
                    if goal_session_id != input.session_id {
                        return Err(anyhow!(
                            "goal {} belongs to session {}; expected {}",
                            goal_id, goal_session_id, input.session_id
                        ));
                    }
                    Some(goal_id.to_string())
                }
                None => conn
                    .query_row(
                        "SELECT id FROM goals
                         WHERE session_id = ?1 AND state IN ('active','paused','evaluating','blocked')
                         ORDER BY updated_at DESC
                         LIMIT 1",
                        params![input.session_id],
                        |row| row.get(0),
                    )
                    .optional()?,
            }
        };
        if let Some(worktree_id) = input.worktree_id.as_deref() {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT session_id, state FROM managed_worktrees WHERE id = ?1",
                    params![worktree_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (worktree_session_id, state) =
                row.ok_or_else(|| anyhow!("managed worktree not found: {worktree_id}"))?;
            if worktree_session_id != input.session_id {
                return Err(anyhow!(
                    "managed worktree {} belongs to session {}; expected {}",
                    worktree_id,
                    worktree_session_id,
                    input.session_id
                ));
            }
            if state != "active" && state != "handoff" {
                return Err(anyhow!(
                    "managed worktree {} is {}; expected active or handoff",
                    worktree_id,
                    state
                ));
            }
        }
        if let Some(goal_id) = goal_id.as_deref() {
            self.ensure_goal_budget_allows_new_workflow(goal_id)?;
        }
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO workflow_runs (
                    id, session_id, kind, state, execution_mode, script_hash, script_source,
                    budget_json, cursor_seq, parent_run_id, origin, goal_id, worktree_id,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13, ?13)",
                params![
                    id,
                    input.session_id,
                    input.kind,
                    WorkflowRunState::Draft.as_str(),
                    input.execution_mode,
                    script_hash,
                    input.script_source,
                    budget_json,
                    input.parent_run_id,
                    input.origin,
                    goal_id,
                    input.worktree_id,
                    now
                ],
            )?;
        }

        let run = self
            .get_workflow_run(&id)?
            .ok_or_else(|| anyhow!("workflow run {} was not persisted", id))?;
        let _ = self.append_workflow_event(
            &run.id,
            "run_created",
            json!({
                "sessionId": run.session_id,
                "kind": run.kind,
                "state": run.state,
                "parentRunId": run.parent_run_id,
                "origin": run.origin,
                "goalId": run.goal_id,
                "worktreeId": run.worktree_id,
            }),
        )?;
        if let Some(parent_run_id) = run.parent_run_id.as_deref() {
            let payload = json!({
                "parentRunId": parent_run_id,
                "childRunId": run.id,
                "origin": run.origin,
            });
            let _ = self.append_workflow_event(&run.id, "run_derived_from", payload.clone())?;
            let _ =
                self.append_workflow_event(parent_run_id, "run_derived_child_created", payload)?;
        }
        if let Some(goal_id) = run.goal_id.as_deref() {
            let _ = self.link_goal_target(
                goal_id,
                "workflow_run",
                &run.id,
                if run.origin.as_deref() == Some("repair") {
                    "repair_run"
                } else {
                    "execution_run"
                },
                json!({
                    "kind": run.kind,
                    "state": run.state,
                    "parentRunId": run.parent_run_id,
                    "origin": run.origin,
                    "worktreeId": run.worktree_id,
                }),
            );
        }
        let preview = super::preview::preview_workflow_run(self, &run);
        let _ = self.append_workflow_event(&run.id, "script_permission_preview", json!(preview))?;
        events::emit_run_changed("workflow:created", &run);
        Ok(run)
    }

    pub fn get_workflow_run(&self, run_id: &str) -> Result<Option<WorkflowRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, kind, state, execution_mode, script_hash, script_source,
                    budget_json, cursor_seq, primary_owner, blocked_reason,
                    parent_run_id, origin, goal_id, worktree_id,
                    created_at, updated_at, completed_at
             FROM workflow_runs WHERE id = ?1",
            params![run_id],
            row_to_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_workflow_runs_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<WorkflowRun>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, state, execution_mode, script_hash, script_source,
                    budget_json, cursor_seq, primary_owner, blocked_reason,
                    parent_run_id, origin, goal_id, worktree_id,
                    created_at, updated_at, completed_at
             FROM workflow_runs
             WHERE session_id = ?1
             ORDER BY updated_at DESC, created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], row_to_run)?;
        collect_rows(rows)
    }

    pub fn workflow_run_snapshot(
        &self,
        run_id: &str,
        event_limit: usize,
    ) -> Result<Option<WorkflowRunSnapshot>> {
        let Some(run) = self.get_workflow_run(run_id)? else {
            return Ok(None);
        };
        let ops = self.list_workflow_ops(run_id)?;
        let events = self.list_workflow_events(run_id, event_limit)?;
        Ok(Some(WorkflowRunSnapshot { run, ops, events }))
    }

    pub fn transition_workflow_run(
        &self,
        run_id: &str,
        next: WorkflowRunState,
        reason: Option<&str>,
    ) -> Result<WorkflowRun> {
        let now = now_rfc3339();
        let previous = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<String> = conn
                .query_row(
                    "SELECT state FROM workflow_runs WHERE id = ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .optional()?;
            let current = current.ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;
            let previous = parse_run_state(&current)?;
            if !previous.can_transition_to(next) {
                return Err(anyhow!(
                    "invalid workflow run transition: {} -> {}",
                    previous.as_str(),
                    next.as_str()
                ));
            }
            conn.execute(
                "UPDATE workflow_runs
                    SET state = ?1,
                        blocked_reason = CASE WHEN ?1 = 'blocked' THEN ?2 ELSE NULL END,
                        primary_owner = CASE WHEN ?1 IN ('paused','completed','failed','cancelled','blocked') THEN NULL ELSE primary_owner END,
                        completed_at = CASE WHEN ?1 IN ('completed','failed','cancelled','blocked') THEN ?3 ELSE completed_at END,
                        updated_at = ?3
                 WHERE id = ?4",
                params![next.as_str(), reason, now, run_id],
            )?;
            previous
        };

        let run = self
            .get_workflow_run(run_id)?
            .ok_or_else(|| anyhow!("workflow run {} not found after transition", run_id))?;
        let _ = self.append_workflow_event(
            run_id,
            "run_state_changed",
            json!({
                "from": previous.as_str(),
                "to": next.as_str(),
                "reason": reason,
            }),
        )?;
        events::emit_run_changed("workflow:updated", &run);
        if next.is_terminal() {
            if let Some(goal_id) = run.goal_id.as_deref() {
                let relation = match next {
                    WorkflowRunState::Completed => "workflow_completed",
                    WorkflowRunState::Failed => "workflow_failed",
                    WorkflowRunState::Cancelled => "workflow_cancelled",
                    WorkflowRunState::Blocked => "workflow_blocked",
                    _ => "workflow_terminal",
                };
                let _ = self.link_goal_target(
                    goal_id,
                    "workflow_run",
                    &run.id,
                    relation,
                    json!({
                        "kind": run.kind,
                        "state": run.state,
                        "blockedReason": run.blocked_reason,
                        "completedAt": run.completed_at,
                        "reason": reason,
                        "worktreeId": run.worktree_id,
                    }),
                );
                if matches!(
                    next,
                    WorkflowRunState::Completed
                        | WorkflowRunState::Failed
                        | WorkflowRunState::Blocked
                ) {
                    let _ = self.evaluate_goal(goal_id);
                }
            }
        }
        Ok(run)
    }

    pub fn pause_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        self.transition_workflow_run(run_id, WorkflowRunState::Paused, Some("pause_requested"))
    }

    pub fn resume_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        self.transition_workflow_run(run_id, WorkflowRunState::Running, Some("resume_requested"))
    }

    pub fn approve_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        let Some(run) = self.get_workflow_run(run_id)? else {
            return Err(anyhow!("workflow run {} not found", run_id));
        };
        if run.state != WorkflowRunState::AwaitingApproval {
            return Err(anyhow!(
                "workflow run {} is {}; only awaiting_approval runs can be approved",
                run_id,
                run.state.as_str()
            ));
        }
        self.transition_workflow_run(run_id, WorkflowRunState::Running, Some("approval_granted"))
    }

    pub fn cancel_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        self.transition_workflow_run(
            run_id,
            WorkflowRunState::Cancelled,
            Some("cancel_requested"),
        )
    }

    pub fn claim_workflow_run_for_recovery(
        &self,
        run_id: &str,
        owner: &str,
    ) -> Result<Option<WorkflowRun>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE workflow_runs
                SET state = 'recovering', primary_owner = ?1, updated_at = ?2
             WHERE id = ?3
               AND state = 'running'
               AND (primary_owner IS NULL OR primary_owner = '')",
            params![owner, now, run_id],
        )?;
        drop(conn);

        if changed == 0 {
            return Ok(None);
        }
        let run = self
            .get_workflow_run(run_id)?
            .ok_or_else(|| anyhow!("workflow run {} not found after claim", run_id))?;
        let _ =
            self.append_workflow_event(run_id, "run_recovery_claimed", json!({ "owner": owner }))?;
        events::emit_run_changed("workflow:updated", &run);
        Ok(Some(run))
    }

    pub fn list_recoverable_workflow_runs(&self) -> Result<Vec<WorkflowRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, state, execution_mode, script_hash, script_source,
                    budget_json, cursor_seq, primary_owner, blocked_reason,
                    parent_run_id, origin, goal_id, worktree_id,
                    created_at, updated_at, completed_at
             FROM workflow_runs
             WHERE state = 'running' AND (primary_owner IS NULL OR primary_owner = '')
             ORDER BY updated_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_run)?;
        collect_rows(rows)
    }

    pub fn upsert_workflow_op_started(&self, input: UpsertWorkflowOpInput) -> Result<WorkflowOp> {
        self.ensure_workflow_run_allows_new_op(&input.run_id)?;
        let input_json = stable_json(&input.input)?;
        let input_hash = blake3_hex(input_json.as_bytes());
        if let Some(existing) = self.get_workflow_op(&input.run_id, &input.op_key)? {
            if existing.input_hash != input_hash {
                let _ = self.transition_workflow_run(
                    &input.run_id,
                    WorkflowRunState::Blocked,
                    Some(&format!("input_hash_mismatch:{}", input.op_key)),
                );
                return Err(anyhow!(
                    "workflow op {} input hash changed for run {}; run was blocked",
                    input.op_key,
                    input.run_id
                ));
            }
            if existing.state.is_terminal() {
                return Ok(existing);
            }
        }
        let now = now_rfc3339();
        let id = format!("wfo_{}", uuid::Uuid::new_v4().simple());
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO workflow_ops (
                    id, run_id, op_key, op_type, effect_class, input_hash, input_json,
                    state, child_handle, started_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'started', ?8, ?9)
                 ON CONFLICT(run_id, op_key) DO UPDATE SET
                    op_type = excluded.op_type,
                    effect_class = excluded.effect_class,
                    input_hash = excluded.input_hash,
                    input_json = excluded.input_json,
                    state = CASE
                        WHEN workflow_ops.state IN ('completed','failed') THEN workflow_ops.state
                        ELSE 'started'
                    END,
                    child_handle = COALESCE(workflow_ops.child_handle, excluded.child_handle),
                    started_at = CASE
                        WHEN workflow_ops.state IN ('completed','failed') THEN workflow_ops.started_at
                        ELSE excluded.started_at
                    END",
                params![
                    id,
                    input.run_id,
                    input.op_key,
                    input.op_type,
                    input.effect_class.as_str(),
                    input_hash,
                    input_json,
                    input.child_handle,
                    now,
                ],
            )?;
            conn.execute(
                "UPDATE workflow_runs SET updated_at = ?1 WHERE id = ?2",
                params![now, input.run_id],
            )?;
        }
        let op = self
            .get_workflow_op(&input.run_id, &input.op_key)?
            .ok_or_else(|| anyhow!("workflow op {} was not persisted", input.op_key))?;
        let _ = self.append_workflow_event(
            &input.run_id,
            "op_started",
            json!({
                "opKey": op.op_key,
                "opType": op.op_type,
                "effectClass": op.effect_class,
                "state": op.state,
            }),
        )?;
        events::emit_op_changed("workflow:op_updated", &op);
        Ok(op)
    }

    pub fn complete_workflow_op(
        &self,
        run_id: &str,
        op_key: &str,
        output: Value,
    ) -> Result<WorkflowOp> {
        self.finish_workflow_op(
            run_id,
            op_key,
            WorkflowOpState::Completed,
            Some(output),
            None,
        )
    }

    pub fn fail_workflow_op(&self, run_id: &str, op_key: &str, error: Value) -> Result<WorkflowOp> {
        self.finish_workflow_op(run_id, op_key, WorkflowOpState::Failed, None, Some(error))
    }

    fn finish_workflow_op(
        &self,
        run_id: &str,
        op_key: &str,
        state: WorkflowOpState,
        output: Option<Value>,
        error: Option<Value>,
    ) -> Result<WorkflowOp> {
        let now = now_rfc3339();
        let output_json = output.as_ref().map(stable_json).transpose()?;
        let error_json = error.as_ref().map(stable_json).transpose()?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let changed = conn.execute(
                "UPDATE workflow_ops
                    SET state = ?1,
                        output_json = ?2,
                        error_json = ?3,
                        completed_at = ?4
                 WHERE run_id = ?5
                   AND op_key = ?6
                   AND state = 'started'",
                params![state.as_str(), output_json, error_json, now, run_id, op_key],
            )?;
            if changed == 0 {
                return Err(anyhow!(
                    "workflow op {} for run {} is not started or does not exist",
                    op_key,
                    run_id
                ));
            }
            conn.execute(
                "UPDATE workflow_runs
                    SET cursor_seq = cursor_seq + 1, updated_at = ?1
                 WHERE id = ?2",
                params![now, run_id],
            )?;
        }
        let op = self
            .get_workflow_op(run_id, op_key)?
            .ok_or_else(|| anyhow!("workflow op {} not found after finish", op_key))?;
        let _ = self.append_workflow_event(
            run_id,
            if state == WorkflowOpState::Completed {
                "op_completed"
            } else {
                "op_failed"
            },
            json!({ "opKey": op_key, "state": state }),
        )?;
        if op.state.is_terminal() {
            if let Some(run) = self.get_workflow_run(run_id)? {
                let _ = self.link_goal_evidence_for_workflow_op(&run, &op);
            }
        }
        events::emit_op_changed("workflow:op_updated", &op);
        Ok(op)
    }

    pub fn get_workflow_op(&self, run_id: &str, op_key: &str) -> Result<Option<WorkflowOp>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, run_id, op_key, op_type, effect_class, input_hash, input_json,
                    state, output_json, error_json, child_handle, started_at, completed_at
             FROM workflow_ops WHERE run_id = ?1 AND op_key = ?2",
            params![run_id, op_key],
            row_to_op,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_workflow_ops(&self, run_id: &str) -> Result<Vec<WorkflowOp>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, op_key, op_type, effect_class, input_hash, input_json,
                    state, output_json, error_json, child_handle, started_at, completed_at
             FROM workflow_ops
             WHERE run_id = ?1
             ORDER BY started_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![run_id], row_to_op)?;
        collect_rows(rows)
    }

    pub fn list_workflow_child_handles(&self, run_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT op_type, child_handle
             FROM workflow_ops
             WHERE run_id = ?1 AND child_handle IS NOT NULL AND child_handle != ''
             ORDER BY started_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        collect_rows(rows)
    }

    pub fn started_op_recovery_action(
        &self,
        run_id: &str,
        op_key: &str,
    ) -> Result<Option<StartedOpRecoveryAction>> {
        let Some(op) = self.get_workflow_op(run_id, op_key)? else {
            return Ok(None);
        };
        if op.state != WorkflowOpState::Started {
            return Ok(None);
        }
        Ok(Some(match op.effect_class {
            WorkflowEffectClass::Pure => StartedOpRecoveryAction::RerunPure,
            WorkflowEffectClass::Idempotent => StartedOpRecoveryAction::RecheckIdempotent,
            WorkflowEffectClass::NonIdempotent => {
                if matches!(op.op_type.as_str(), "spawnAgent" | "validate")
                    || op.op_type.starts_with("tool:")
                {
                    if let Some(handle) = op.child_handle {
                        StartedOpRecoveryAction::AttachChildHandle(handle)
                    } else {
                        StartedOpRecoveryAction::BlockNonIdempotent
                    }
                } else {
                    StartedOpRecoveryAction::BlockNonIdempotent
                }
            }
        }))
    }

    pub fn block_run_for_started_non_idempotent_op(
        &self,
        run_id: &str,
        op_key: &str,
    ) -> Result<WorkflowRun> {
        self.transition_workflow_run(
            run_id,
            WorkflowRunState::Blocked,
            Some(&format!("started_non_idempotent_op:{op_key}")),
        )
    }

    pub fn append_workflow_event(
        &self,
        run_id: &str,
        event_type: &str,
        payload: Value,
    ) -> Result<WorkflowEvent> {
        let payload_json = bounded_event_payload(payload)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM workflow_events WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO workflow_events (run_id, seq, type, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, seq, event_type, payload_json, now],
        )?;
        let id = conn.last_insert_rowid();
        let event = WorkflowEvent {
            id,
            run_id: run_id.to_string(),
            seq,
            event_type: event_type.to_string(),
            payload: serde_json::from_str(&payload_json)?,
            created_at: now,
        };
        drop(conn);
        events::emit_event("workflow:event", &event);
        Ok(event)
    }

    pub fn list_workflow_events(&self, run_id: &str, limit: usize) -> Result<Vec<WorkflowEvent>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, seq, type, payload_json, created_at
             FROM workflow_events
             WHERE run_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![run_id, limit], row_to_event)?;
        let mut events = collect_rows(rows)?;
        events.reverse();
        Ok(events)
    }

    fn ensure_workflow_run_allows_new_op(&self, run_id: &str) -> Result<()> {
        let run = self
            .get_workflow_run(run_id)?
            .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;
        if run.state == WorkflowRunState::Cancelled {
            return Err(anyhow!(
                "workflow run {} is cancelled; refusing to start new op",
                run_id
            ));
        }
        if run.state.is_terminal() {
            return Err(anyhow!(
                "workflow run {} is terminal ({}); refusing to start new op",
                run_id,
                run.state.as_str()
            ));
        }
        if run.state != WorkflowRunState::Running {
            return Err(anyhow!(
                "workflow run {} is {}; refusing to start new op",
                run_id,
                run.state.as_str()
            ));
        }
        Ok(())
    }
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRun> {
    let state: String = row.get(3)?;
    let budget_json: String = row.get(7)?;
    Ok(WorkflowRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        kind: row.get(2)?,
        state: parse_run_state_sql(&state)?,
        execution_mode: row.get(4)?,
        script_hash: row.get(5)?,
        script_source: row.get(6)?,
        budget: json_from_sql(&budget_json)?,
        cursor_seq: row.get(8)?,
        primary_owner: row.get(9)?,
        blocked_reason: row.get(10)?,
        parent_run_id: row.get(11)?,
        origin: row.get(12)?,
        goal_id: row.get(13)?,
        worktree_id: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        completed_at: row.get(17)?,
    })
}

fn row_to_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowOp> {
    let effect_class: String = row.get(4)?;
    let input_json: String = row.get(6)?;
    let state: String = row.get(7)?;
    let output_json: Option<String> = row.get(8)?;
    let error_json: Option<String> = row.get(9)?;
    Ok(WorkflowOp {
        id: row.get(0)?,
        run_id: row.get(1)?,
        op_key: row.get(2)?,
        op_type: row.get(3)?,
        effect_class: parse_effect_class_sql(&effect_class)?,
        input_hash: row.get(5)?,
        input: json_from_sql(&input_json)?,
        state: parse_op_state_sql(&state)?,
        output: output_json.as_deref().map(json_from_sql).transpose()?,
        error: error_json.as_deref().map(json_from_sql).transpose()?,
        child_handle: row.get(10)?,
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowEvent> {
    let payload_json: String = row.get(4)?;
    Ok(WorkflowEvent {
        id: row.get(0)?,
        run_id: row.get(1)?,
        seq: row.get(2)?,
        event_type: row.get(3)?,
        payload: json_from_sql(&payload_json)?,
        created_at: row.get(5)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn parse_run_state(value: &str) -> Result<WorkflowRunState> {
    WorkflowRunState::from_str(value).ok_or_else(|| anyhow!("unknown workflow run state: {value}"))
}

fn parse_run_state_sql(value: &str) -> rusqlite::Result<WorkflowRunState> {
    WorkflowRunState::from_str(value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown workflow run state: {value}").into(),
        )
    })
}

fn parse_op_state_sql(value: &str) -> rusqlite::Result<WorkflowOpState> {
    WorkflowOpState::from_str(value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown workflow op state: {value}").into(),
        )
    })
}

fn parse_effect_class_sql(value: &str) -> rusqlite::Result<WorkflowEffectClass> {
    WorkflowEffectClass::from_str(value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown workflow effect class: {value}").into(),
        )
    })
}

fn json_from_sql(value: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
    })
}

fn stable_json(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(Into::into)
}

fn bounded_event_payload(payload: Value) -> Result<String> {
    let json = stable_json(&payload)?;
    if json.len() <= EVENT_PAYLOAD_MAX_BYTES {
        return Ok(json);
    }
    stable_json(&json!({
        "truncated": true,
        "originalBytes": json.len(),
        "preview": crate::truncate_utf8(&json, EVENT_PAYLOAD_MAX_BYTES),
    }))
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
