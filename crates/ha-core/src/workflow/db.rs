use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashSet;

use crate::session::{MessageRole, SessionDB};

use super::events;
use super::types::{
    CreateWorkflowRunFromTemplateInput, CreateWorkflowRunInput, ListSavedWorkflowTemplatesInput,
    PendingWorkflowMilestoneInjection, SaveWorkflowTemplateInput, SavedWorkflowTemplate,
    SavedWorkflowTemplateScope, StartedOpRecoveryAction, UpsertWorkflowOpInput,
    WorkflowAgentUsageSnapshot, WorkflowEffectClass, WorkflowEvent, WorkflowOp, WorkflowOpState,
    WorkflowRun, WorkflowRunSnapshot, WorkflowRunState, WorkflowRunUsageSnapshot,
    WorkflowWatchdogFinding,
};

const EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const SAVED_WORKFLOW_TEMPLATE_LIMIT_DEFAULT: usize = 50;
const SAVED_WORKFLOW_TEMPLATE_LIMIT_MAX: usize = 200;
const SAVED_WORKFLOW_TEMPLATE_NAME_MAX_CHARS: usize = 120;
const SAVED_WORKFLOW_TEMPLATE_DESCRIPTION_MAX_CHARS: usize = 1000;

#[derive(Debug, Clone, Default)]
struct WorkflowParentInjectionUsageSnapshot {
    turns: i64,
    messages: i64,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    provider_events: i64,
    provider_input_tokens: i64,
    provider_output_tokens: i64,
    provider_cache_creation_input_tokens: i64,
    provider_cache_read_input_tokens: i64,
    provider_total_tokens: i64,
    attribution: String,
}

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
            goal_criterion_id TEXT,
            goal_criterion_text TEXT,
            goal_criterion_kind TEXT,
            goal_revision INTEGER,
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
            ON workflow_events(run_id, seq);
        CREATE INDEX IF NOT EXISTS idx_workflow_events_type_id
            ON workflow_events(type, id);

        CREATE TABLE IF NOT EXISTS saved_workflow_templates (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            scope TEXT NOT NULL,
            project_id TEXT,
            kind TEXT NOT NULL,
            execution_mode TEXT NOT NULL,
            script_hash TEXT NOT NULL,
            script_source TEXT NOT NULL,
            budget_json TEXT NOT NULL DEFAULT '{}',
            source_run_id TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (source_run_id) REFERENCES workflow_runs(id) ON DELETE SET NULL
        );

        CREATE INDEX IF NOT EXISTS idx_saved_workflow_templates_scope_updated
            ON saved_workflow_templates(scope, project_id, enabled, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_saved_workflow_templates_source_run
            ON saved_workflow_templates(source_run_id);",
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
        .prepare("SELECT goal_criterion_id FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN goal_criterion_id TEXT;")?;
    }
    if conn
        .prepare("SELECT goal_criterion_text FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN goal_criterion_text TEXT;")?;
    }
    if conn
        .prepare("SELECT goal_criterion_kind FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN goal_criterion_kind TEXT;")?;
    }
    if conn
        .prepare("SELECT goal_revision FROM workflow_runs LIMIT 1")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE workflow_runs ADD COLUMN goal_revision INTEGER;")?;
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
        "CREATE INDEX IF NOT EXISTS idx_workflow_runs_goal_criterion
            ON workflow_runs(goal_id, goal_criterion_id, updated_at DESC);",
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
                            goal_id,
                            goal_session_id,
                            input.session_id
                        ));
                    }
                    Some(goal_id.to_string())
                }
                None => conn
                    .query_row(
                        "SELECT id FROM goals
                         WHERE session_id = ?1
                           AND (
                                state IN ('active','paused','evaluating','blocked')
                                OR (state = 'completed' AND closure_decision IS NULL)
                           )
                         ORDER BY updated_at DESC
                         LIMIT 1",
                        params![input.session_id],
                        |row| row.get(0),
                    )
                    .optional()?,
            }
        };
        let goal_criterion = match goal_id.as_deref() {
            Some(goal_id) => {
                self.resolve_goal_criterion_binding(goal_id, input.goal_criterion_id.as_deref())?
            }
            None => {
                if input
                    .goal_criterion_id
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty())
                {
                    return Err(anyhow!(
                        "goal criterion binding requires a workflow run bound to a Goal"
                    ));
                }
                None
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
                    goal_criterion_id, goal_criterion_text, goal_criterion_kind, goal_revision,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
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
                    goal_criterion.as_ref().map(|criterion| criterion.id.as_str()),
                    goal_criterion.as_ref().map(|criterion| criterion.text.as_str()),
                    goal_criterion
                        .as_ref()
                        .map(|criterion| criterion.kind.as_str()),
                    goal_criterion
                        .as_ref()
                        .map(|criterion| criterion.goal_revision),
                    now
                ],
            )?;
        }

        let run = self
            .get_workflow_run(&id)?
            .ok_or_else(|| anyhow!("workflow run {} was not persisted", id))?;
        if let Some(worktree_id) = run.worktree_id.as_deref() {
            match self.link_managed_worktree_to_workflow_run(worktree_id, &run.id) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    crate::app_warn!(
                        "workflow",
                        "worktree_link",
                        "managed worktree {} disappeared before linking to workflow run {}",
                        worktree_id,
                        run.id
                    );
                }
                Err(err) => {
                    crate::app_warn!(
                        "workflow",
                        "worktree_link",
                        "failed to link managed worktree {} to workflow run {}: {err:#}",
                        worktree_id,
                        run.id
                    );
                }
            }
        }
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
                "goalCriterion": workflow_run_goal_criterion_metadata(&run),
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
                    "goalCriterion": workflow_run_goal_criterion_metadata(&run),
                }),
            );
            if let Err(err) = self.link_goal_worktree_evidence_for_workflow_run(&run) {
                crate::app_warn!(
                    "goal",
                    "worktree_evidence",
                    "failed to link worktree evidence for workflow run {}: {err:#}",
                    run.id
                );
            }
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
                    parent_run_id, origin, goal_id, goal_criterion_id,
                    goal_criterion_text, goal_criterion_kind, goal_revision, worktree_id,
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
                    parent_run_id, origin, goal_id, goal_criterion_id,
                    goal_criterion_text, goal_criterion_kind, goal_revision, worktree_id,
                    created_at, updated_at, completed_at
             FROM workflow_runs
             WHERE session_id = ?1
             ORDER BY updated_at DESC, created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], row_to_run)?;
        collect_rows(rows)
    }

    pub fn save_workflow_template_from_run(
        &self,
        input: SaveWorkflowTemplateInput,
    ) -> Result<SavedWorkflowTemplate> {
        if !input.explicit_save_consent {
            return Err(anyhow!(
                "saving a workflow template requires explicit user consent"
            ));
        }
        let name = normalize_saved_template_name(&input.name)?;
        let description = normalize_saved_template_description(input.description.as_deref());
        let run = self
            .get_workflow_run(&input.source_run_id)?
            .ok_or_else(|| anyhow!("workflow run not found: {}", input.source_run_id))?;
        if run.state != WorkflowRunState::Completed {
            return Err(anyhow!(
                "only completed workflow runs can be saved as templates; run {} is {}",
                run.id,
                run.state.as_str()
            ));
        }
        let project_id = self.resolve_saved_workflow_template_project_id(
            &run.session_id,
            input.scope,
            input.project_id.as_deref(),
        )?;
        let now = now_rfc3339();
        let id = format!("wft_{}", uuid::Uuid::new_v4().simple());
        let budget_json = stable_json(&run.budget)?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO saved_workflow_templates (
                    id, name, description, scope, project_id, kind, execution_mode,
                    script_hash, script_source, budget_json, source_run_id, enabled,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12)",
                params![
                    id,
                    name,
                    description,
                    input.scope.as_str(),
                    project_id,
                    run.kind,
                    run.execution_mode,
                    run.script_hash,
                    run.script_source,
                    budget_json,
                    run.id,
                    now
                ],
            )?;
        }
        self.get_saved_workflow_template(&id)?
            .ok_or_else(|| anyhow!("saved workflow template {} was not persisted", id))
    }

    pub fn list_saved_workflow_templates(
        &self,
        input: ListSavedWorkflowTemplatesInput,
    ) -> Result<Vec<SavedWorkflowTemplate>> {
        let limit = input
            .limit
            .unwrap_or(SAVED_WORKFLOW_TEMPLATE_LIMIT_DEFAULT)
            .clamp(1, SAVED_WORKFLOW_TEMPLATE_LIMIT_MAX);
        let project_id = normalize_optional(input.project_id.as_deref());
        let mut clauses = Vec::new();
        let mut values: Vec<String> = Vec::new();
        if !input.include_disabled {
            clauses.push("enabled = 1".to_string());
        }
        if let Some(project_id) = project_id {
            clauses.push("(scope = 'user' OR (scope = 'project' AND project_id = ?))".to_string());
            values.push(project_id.to_string());
        } else {
            clauses.push("scope = 'user'".to_string());
        }
        let where_sql = format!("WHERE {}", clauses.join(" AND "));
        values.push(limit.to_string());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, description, scope, project_id, kind, execution_mode,
                    script_hash, script_source, budget_json, source_run_id, enabled,
                    created_at, updated_at
             FROM saved_workflow_templates
             {where_sql}
             ORDER BY updated_at DESC, created_at DESC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(values.iter()), row_to_saved_template)?;
        collect_rows(rows)
    }

    pub fn get_saved_workflow_template(
        &self,
        template_id: &str,
    ) -> Result<Option<SavedWorkflowTemplate>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, name, description, scope, project_id, kind, execution_mode,
                    script_hash, script_source, budget_json, source_run_id, enabled,
                    created_at, updated_at
             FROM saved_workflow_templates
             WHERE id = ?1",
            params![template_id],
            row_to_saved_template,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn create_workflow_run_from_template(
        &self,
        input: CreateWorkflowRunFromTemplateInput,
    ) -> Result<WorkflowRun> {
        let template = self
            .get_saved_workflow_template(&input.template_id)?
            .ok_or_else(|| anyhow!("saved workflow template not found: {}", input.template_id))?;
        if !template.enabled {
            return Err(anyhow!(
                "saved workflow template {} is disabled",
                template.id
            ));
        }
        self.ensure_saved_workflow_template_visible_to_session(&template, &input.session_id)?;
        self.create_workflow_run(CreateWorkflowRunInput {
            session_id: input.session_id,
            kind: template.kind,
            execution_mode: template.execution_mode,
            script_source: template.script_source,
            budget: input.budget.unwrap_or(template.budget),
            parent_run_id: None,
            origin: Some(format!("template:{}", template.id)),
            goal_id: input.goal_id,
            goal_criterion_id: input.goal_criterion_id,
            worktree_id: input.worktree_id,
        })
    }

    pub fn workflow_run_snapshot(
        &self,
        run_id: &str,
        event_limit: usize,
    ) -> Result<Option<WorkflowRunSnapshot>> {
        let Some(run) = self.get_workflow_run(run_id)? else {
            return Ok(None);
        };
        let mut ops = self.list_workflow_ops(run_id)?;
        self.hydrate_workflow_agent_ops(&mut ops)?;
        let events = self.list_workflow_events(run_id, event_limit)?;
        let agent_usage = self.workflow_agent_usage_snapshot(run_id)?;
        let usage = self.workflow_run_usage_snapshot(&run, &agent_usage)?;
        Ok(Some(WorkflowRunSnapshot {
            run,
            ops,
            events,
            agent_usage,
            usage,
        }))
    }

    fn hydrate_workflow_agent_ops(&self, ops: &mut [WorkflowOp]) -> Result<()> {
        for op in ops {
            if op.op_type != "spawnAgent" {
                continue;
            }
            let Some(run_id) = op.child_handle.as_deref() else {
                continue;
            };
            let Some(child) = self.get_subagent_run(run_id)? else {
                continue;
            };
            let mut output = op.output.take().unwrap_or_else(|| json!({}));
            if let Value::Object(ref mut map) = output {
                map.insert("runId".to_string(), json!(child.run_id));
                map.insert("run_id".to_string(), json!(child.run_id));
                map.insert("sessionId".to_string(), json!(child.child_session_id));
                map.insert("session_id".to_string(), json!(child.child_session_id));
                map.insert("status".to_string(), json!(child.status.as_str()));
                map.insert("resultAvailable".to_string(), json!(child.result.is_some()));
                map.insert("durationMs".to_string(), json!(child.duration_ms));
                map.insert("inputTokens".to_string(), json!(child.input_tokens));
                map.insert("outputTokens".to_string(), json!(child.output_tokens));
                if let Some(error) = child.error.as_deref() {
                    map.insert("error".to_string(), json!(error));
                }
            }
            op.output = Some(output);
        }
        Ok(())
    }

    pub fn workflow_agent_usage_snapshot(
        &self,
        run_id: &str,
    ) -> Result<WorkflowAgentUsageSnapshot> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut snapshot = conn.query_row(
            "SELECT
                COUNT(DISTINCT wo.child_handle),
                COALESCE(SUM(CASE WHEN sr.status = 'completed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN sr.status IN ('queued','spawning','running') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN sr.run_id IS NULL THEN 1
                    WHEN sr.status IN ('error','timeout','killed') THEN 1
                    ELSE 0
                END), 0),
                COALESCE(SUM(CASE WHEN sr.input_tokens IS NOT NULL OR sr.output_tokens IS NOT NULL THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(sr.input_tokens), 0),
                COALESCE(SUM(sr.output_tokens), 0)
             FROM workflow_ops wo
             LEFT JOIN subagent_runs sr ON sr.run_id = wo.child_handle
             WHERE wo.run_id = ?1
               AND wo.op_type = 'spawnAgent'
               AND wo.child_handle IS NOT NULL
               AND wo.child_handle != ''",
            params![run_id],
            |row| {
                let input_tokens = row.get::<_, i64>(5)?;
                let output_tokens = row.get::<_, i64>(6)?;
                Ok(WorkflowAgentUsageSnapshot {
                    spawned_agents: row.get(0)?,
                    completed_agents: row.get(1)?,
                    running_agents: row.get(2)?,
                    failed_agents: row.get(3)?,
                    terminal_agents: 0,
                    consumed_results: 0,
                    pending_results: 0,
                    suppressed_results: 0,
                    attributed_agents: row.get(4)?,
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens.saturating_add(output_tokens),
                    attribution: "workflow_ops.child_handle=subagent_runs.run_id".to_string(),
                })
            },
        )?;
        snapshot.terminal_agents = snapshot
            .completed_agents
            .saturating_add(snapshot.failed_agents);

        let mut consumed = HashSet::new();
        let mut suppressed = HashSet::new();
        let mut stmt = conn.prepare(
            "SELECT type, payload_json
             FROM workflow_events
             WHERE run_id = ?1
               AND type IN ('workflow_agent_result_consumed','workflow_agent_result_suppressed')",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (event_type, payload_json) = row?;
            let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
                continue;
            };
            let Some(ids) = payload.get("childRunIds").and_then(Value::as_array) else {
                continue;
            };
            let target = if event_type == "workflow_agent_result_suppressed" {
                &mut suppressed
            } else {
                &mut consumed
            };
            for id in ids.iter().filter_map(Value::as_str) {
                target.insert(id.to_string());
            }
        }
        let mut stmt = conn.prepare(
            "SELECT op_type, output_json
             FROM workflow_ops
             WHERE run_id = ?1
               AND state = 'completed'
               AND op_type IN ('waitAll','agentResult','finish')
               AND output_json IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (op_type, output_json) = row?;
            let Ok(output) = serde_json::from_str::<Value>(&output_json) else {
                continue;
            };
            if op_type == "waitAll"
                && output
                    .get("resultMode")
                    .or_else(|| output.get("result_mode"))
                    .and_then(Value::as_str)
                    == Some("status")
            {
                continue;
            }
            collect_handled_workflow_agent_ids(&output, &mut consumed);
        }
        snapshot.consumed_results = consumed.len() as i64;
        snapshot.suppressed_results = suppressed.len() as i64;
        let handled_results = consumed.union(&suppressed).count() as i64;
        snapshot.pending_results = snapshot.terminal_agents.saturating_sub(handled_results);
        if snapshot.spawned_agents == 0 {
            snapshot.attribution = "no_spawn_agent_ops".to_string();
        }
        Ok(snapshot)
    }

    pub fn workflow_run_usage_snapshot(
        &self,
        run: &WorkflowRun,
        agent_usage: &WorkflowAgentUsageSnapshot,
    ) -> Result<WorkflowRunUsageSnapshot> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let window_end = run
            .completed_at
            .clone()
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let parent_injection_usage =
            workflow_parent_injection_usage_snapshot_with_conn(&conn, run)?;
        let mut snapshot = conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cache_creation_input_tokens), 0),
                COALESCE(SUM(cache_read_input_tokens), 0)
             FROM model_usage_events
             WHERE session_id = ?1
               AND timestamp >= ?2
               AND timestamp <= ?3",
            params![&run.session_id, &run.created_at, window_end],
            |row| {
                let parent_input_tokens = row.get::<_, i64>(1)?;
                let parent_output_tokens = row.get::<_, i64>(2)?;
                let parent_total_tokens = parent_input_tokens.saturating_add(parent_output_tokens);
                let agent_input_tokens = agent_usage.input_tokens;
                let agent_output_tokens = agent_usage.output_tokens;
                let agent_total_tokens = agent_usage.total_tokens;
                Ok(WorkflowRunUsageSnapshot {
                    parent_events: row.get(0)?,
                    parent_input_tokens,
                    parent_output_tokens,
                    parent_cache_creation_input_tokens: row.get(3)?,
                    parent_cache_read_input_tokens: row.get(4)?,
                    parent_total_tokens,
                    parent_injection_turns: parent_injection_usage.turns,
                    parent_injection_messages: parent_injection_usage.messages,
                    parent_injection_input_tokens: parent_injection_usage.input_tokens,
                    parent_injection_output_tokens: parent_injection_usage.output_tokens,
                    parent_injection_total_tokens: parent_injection_usage.total_tokens,
                    parent_injection_provider_events: parent_injection_usage.provider_events,
                    parent_injection_provider_input_tokens: parent_injection_usage
                        .provider_input_tokens,
                    parent_injection_provider_output_tokens: parent_injection_usage
                        .provider_output_tokens,
                    parent_injection_provider_cache_creation_input_tokens:
                        parent_injection_usage.provider_cache_creation_input_tokens,
                    parent_injection_provider_cache_read_input_tokens: parent_injection_usage
                        .provider_cache_read_input_tokens,
                    parent_injection_provider_total_tokens: parent_injection_usage
                        .provider_total_tokens,
                    parent_injection_attribution: parent_injection_usage.attribution.clone(),
                    agent_input_tokens,
                    agent_output_tokens,
                    agent_total_tokens,
                    total_tokens: parent_total_tokens.saturating_add(agent_total_tokens),
                    attribution: "session_model_usage_between_workflow_run_bounds+workflow_ops.child_handle=subagent_runs.run_id".to_string(),
                })
            },
        )?;
        snapshot.attribution = match (snapshot.parent_events > 0, agent_usage.spawned_agents > 0) {
            (true, true) => {
                "session_model_usage_between_workflow_run_bounds+workflow_ops.child_handle=subagent_runs.run_id"
            }
            (true, false) => "session_model_usage_between_workflow_run_bounds",
            (false, true) => "workflow_ops.child_handle=subagent_runs.run_id",
            (false, false) => "no_parent_usage_rows_or_spawn_agent_ops",
        }
        .to_string();
        Ok(snapshot)
    }

    pub fn list_workflow_watchdog_findings(
        &self,
        session_id: &str,
        stale_secs: i64,
    ) -> Result<Vec<WorkflowWatchdogFinding>> {
        let stale_secs = stale_secs.max(0);
        let now = Utc::now();
        let runs = self.list_workflow_runs_for_session(session_id, 100)?;
        let mut findings = Vec::new();

        for run in runs {
            if !matches!(
                run.state,
                WorkflowRunState::Running | WorkflowRunState::Recovering
            ) {
                continue;
            }

            let latest_event = self.list_workflow_events(&run.id, 1)?.into_iter().next();
            let (last_activity_at, stale_for_secs) =
                workflow_last_activity(&run, latest_event.as_ref(), &now);

            if workflow_run_owner_recoverable(run.state, run.primary_owner.as_deref()) {
                findings.push(WorkflowWatchdogFinding {
                    run_id: run.id,
                    session_id: run.session_id,
                    severity: "warning".to_string(),
                    code: "workflow_recoverable_owner".to_string(),
                    message:
                        "Workflow is active but its runtime owner is missing or no longer alive."
                            .to_string(),
                    state: run.state.as_str().to_string(),
                    primary_owner: run.primary_owner,
                    last_activity_at,
                    stale_secs: stale_for_secs,
                    latest_event_type: latest_event.as_ref().map(|event| event.event_type.clone()),
                    latest_event_seq: latest_event.as_ref().map(|event| event.seq),
                });
                continue;
            }

            if stale_for_secs.is_some_and(|secs| secs > stale_secs) {
                findings.push(WorkflowWatchdogFinding {
                    run_id: run.id,
                    session_id: run.session_id,
                    severity: "warning".to_string(),
                    code: "workflow_no_recent_progress".to_string(),
                    message: "Workflow is still active but has not recorded recent progress."
                        .to_string(),
                    state: run.state.as_str().to_string(),
                    primary_owner: run.primary_owner,
                    last_activity_at,
                    stale_secs: stale_for_secs,
                    latest_event_type: latest_event.as_ref().map(|event| event.event_type.clone()),
                    latest_event_seq: latest_event.as_ref().map(|event| event.seq),
                });
            }
        }

        Ok(findings)
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
                        primary_owner = CASE WHEN ?1 IN ('awaiting_approval','awaiting_user','paused','completed','failed','cancelled','blocked') THEN NULL ELSE primary_owner END,
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
                        "goalCriterion": workflow_run_goal_criterion_metadata(&run),
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
            if let Ok(Some(retro)) = self.ensure_coding_workflow_retro_for_run(&run) {
                let _ = self.append_workflow_event(
                    &run.id,
                    "coding_retro_recorded",
                    json!({
                        "retroId": retro.id,
                        "summary": retro.summary,
                        "recommendations": retro.recommendations,
                    }),
                );
            }
        }
        Ok(run)
    }

    pub fn pause_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        let run = self.transition_workflow_run(
            run_id,
            WorkflowRunState::Paused,
            Some("pause_requested"),
        )?;
        self.append_workflow_control_action(&run, "pause", "pause_requested")?;
        Ok(run)
    }

    pub fn resume_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        let run = self.transition_workflow_run(
            run_id,
            WorkflowRunState::Running,
            Some("resume_requested"),
        )?;
        self.append_workflow_control_action(&run, "resume", "resume_requested")?;
        Ok(run)
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
        let run = self.transition_workflow_run(
            run_id,
            WorkflowRunState::Running,
            Some("approval_granted"),
        )?;
        self.append_workflow_control_action(&run, "approve", "approval_granted")?;
        Ok(run)
    }

    pub fn cancel_workflow_run(&self, run_id: &str) -> Result<WorkflowRun> {
        let run = self.transition_workflow_run(
            run_id,
            WorkflowRunState::Cancelled,
            Some("cancel_requested"),
        )?;
        self.append_workflow_control_action(&run, "cancel", "cancel_requested")?;
        Ok(run)
    }

    fn append_workflow_control_action(
        &self,
        run: &WorkflowRun,
        action: &str,
        reason: &str,
    ) -> Result<()> {
        self.append_workflow_event(
            &run.id,
            "run_control_action",
            json!({
                "action": action,
                "reason": reason,
                "resultState": run.state.as_str(),
                "accepted": true,
                "surface": "user_control",
            }),
        )?;
        Ok(())
    }

    pub fn claim_workflow_run_for_recovery(
        &self,
        run_id: &str,
        owner: &str,
    ) -> Result<Option<WorkflowRun>> {
        self.claim_workflow_run_owner(run_id, owner, WorkflowOwnerClaim::Recovery)
    }

    pub fn claim_workflow_run_for_launch(
        &self,
        run_id: &str,
        owner: &str,
    ) -> Result<Option<WorkflowRun>> {
        self.claim_workflow_run_owner(run_id, owner, WorkflowOwnerClaim::Launch)
    }

    fn claim_workflow_run_owner(
        &self,
        run_id: &str,
        owner: &str,
        claim: WorkflowOwnerClaim,
    ) -> Result<Option<WorkflowRun>> {
        let now = now_rfc3339();
        let (state, current_owner) = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let row = conn
                .query_row(
                    "SELECT state, primary_owner FROM workflow_runs WHERE id = ?1",
                    params![run_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?;
            let Some((raw_state, current_owner)) = row else {
                return Ok(None);
            };
            (parse_run_state(&raw_state)?, current_owner)
        };

        let Some(target_state) =
            workflow_owner_claim_target_state(state, current_owner.as_deref(), claim)
        else {
            return Ok(None);
        };

        let current_owner_param = current_owner.as_deref();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE workflow_runs
                SET state = ?1, primary_owner = ?2, updated_at = ?3
             WHERE id = ?4
               AND state = ?5
               AND (
                    (primary_owner IS NULL AND ?6 IS NULL)
                    OR primary_owner = ?6
               )",
            params![
                target_state.as_str(),
                owner,
                now,
                run_id,
                state.as_str(),
                current_owner_param
            ],
        )?;
        drop(conn);

        if changed == 0 {
            return Ok(None);
        }
        let run = self
            .get_workflow_run(run_id)?
            .ok_or_else(|| anyhow!("workflow run {} not found after claim", run_id))?;
        let event_type = match claim {
            WorkflowOwnerClaim::Recovery => "run_recovery_claimed",
            WorkflowOwnerClaim::Launch => "run_launch_claimed",
        };
        let _ = self.append_workflow_event(
            run_id,
            event_type,
            json!({
                "owner": owner,
                "fromState": state.as_str(),
                "toState": target_state.as_str(),
            }),
        )?;
        events::emit_run_changed("workflow:updated", &run);
        Ok(Some(run))
    }

    pub fn list_recoverable_workflow_runs(&self) -> Result<Vec<WorkflowRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, state, execution_mode, script_hash, script_source,
                    budget_json, cursor_seq, primary_owner, blocked_reason,
                    parent_run_id, origin, goal_id, goal_criterion_id,
                    goal_criterion_text, goal_criterion_kind, goal_revision, worktree_id,
                    created_at, updated_at, completed_at
             FROM workflow_runs
             WHERE state IN ('draft', 'running', 'recovering')
             ORDER BY updated_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_run)?;
        Ok(collect_rows(rows)?
            .into_iter()
            .filter(|run| workflow_run_owner_recoverable(run.state, run.primary_owner.as_deref()))
            .collect())
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

    pub fn list_workflow_ops_for_child(&self, child_handle: &str) -> Result<Vec<WorkflowOp>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, op_key, op_type, effect_class, input_hash, input_json,
                    state, output_json, error_json, child_handle, started_at, completed_at
             FROM workflow_ops
             WHERE child_handle = ?1 AND op_type = 'spawnAgent'
             ORDER BY started_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![child_handle], row_to_op)?;
        collect_rows(rows)
    }

    pub fn list_terminal_children_for_active_workflows(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, crate::subagent::SubagentStatus)>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT wo.child_handle, sr.status
             FROM workflow_ops wo
             JOIN workflow_runs wr ON wr.id = wo.run_id
             JOIN subagent_runs sr ON sr.run_id = wo.child_handle
             WHERE wo.op_type = 'spawnAgent'
               AND wo.child_handle IS NOT NULL
               AND wo.child_handle != ''
               AND wr.state IN ('running','recovering')
               AND sr.status IN ('completed','error','timeout','killed')
             ORDER BY wo.started_at ASC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.clamp(1, 1000) as i64], |row| {
            let run_id = row.get::<_, String>(0)?;
            let status = crate::subagent::SubagentStatus::from_str(&row.get::<_, String>(1)?);
            Ok((run_id, status))
        })?;
        collect_rows(rows)
    }

    pub fn workflow_agent_result_handled(&self, run_id: &str, child_run_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT payload_json
             FROM workflow_events
             WHERE run_id = ?1
               AND type IN (
                 'workflow_agent_result_consumed',
                 'workflow_agent_result_suppressed'
               )",
        )?;
        let rows = stmt.query_map(params![run_id], |row| row.get::<_, String>(0))?;
        for row in rows {
            let Ok(payload) = serde_json::from_str::<Value>(&row?) else {
                continue;
            };
            if payload
                .get("childRunIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(child_run_id)))
            {
                return Ok(true);
            }
        }

        let mut stmt = conn.prepare(
            "SELECT op_type, output_json
             FROM workflow_ops
             WHERE run_id = ?1
               AND state = 'completed'
               AND op_type IN ('waitAll','agentResult','finish')
               AND output_json IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (op_type, output_json) = row?;
            let Ok(output) = serde_json::from_str::<Value>(&output_json) else {
                continue;
            };
            if op_type == "waitAll"
                && output
                    .get("resultMode")
                    .or_else(|| output.get("result_mode"))
                    .and_then(Value::as_str)
                    == Some("status")
            {
                continue;
            }
            let mut consumed = HashSet::new();
            collect_handled_workflow_agent_ids(&output, &mut consumed);
            if consumed.contains(child_run_id) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn workflow_agent_checkpoint_injection_run_ids(
        &self,
        run_id: &str,
        child_run_id: &str,
    ) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT seq, payload_json
             FROM workflow_events
             WHERE run_id = ?1 AND type = 'workflow_checkpoint'
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut injection_run_ids = Vec::new();
        for row in rows {
            let (seq, payload_json) = row?;
            let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
                continue;
            };
            if payload.get("childRunId").and_then(Value::as_str) == Some(child_run_id) {
                injection_run_ids.push(format!("{run_id}:workflow-event:{seq}"));
            }
        }
        Ok(injection_run_ids)
    }

    pub fn workflow_agent_terminal_event_exists(
        &self,
        run_id: &str,
        child_run_id: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT payload_json
             FROM workflow_events
             WHERE run_id = ?1 AND type = 'workflow_agent_terminal'",
        )?;
        let rows = stmt.query_map(params![run_id], |row| row.get::<_, String>(0))?;
        for row in rows {
            let Ok(payload) = serde_json::from_str::<Value>(&row?) else {
                continue;
            };
            if payload.get("childRunId").and_then(Value::as_str) == Some(child_run_id) {
                return Ok(true);
            }
        }
        Ok(false)
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

    pub fn get_workflow_event_by_seq(
        &self,
        run_id: &str,
        seq: i64,
    ) -> Result<Option<WorkflowEvent>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, run_id, seq, type, payload_json, created_at
             FROM workflow_events
             WHERE run_id = ?1 AND seq = ?2",
            params![run_id, seq],
            row_to_event,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn workflow_milestone_injection_settled(
        &self,
        run_id: &str,
        source_event_type: &str,
        source_event_seq: i64,
    ) -> Result<bool> {
        let payloads = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT payload_json
                 FROM workflow_events
                 WHERE run_id = ?1
                   AND type IN (
                     'workflow_milestone_injection_delivered',
                     'workflow_milestone_injection_suppressed'
                   )",
            )?;
            let rows = stmt.query_map(params![run_id], |row| row.get::<_, String>(0))?;
            collect_rows(rows)?
        };
        for payload_json in payloads {
            let payload: Value = serde_json::from_str(&payload_json)?;
            if payload.get("sourceEventType").and_then(Value::as_str) == Some(source_event_type)
                && payload.get("sourceEventSeq").and_then(Value::as_i64) == Some(source_event_seq)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn list_pending_workflow_milestone_injections(
        &self,
        limit: usize,
    ) -> Result<Vec<PendingWorkflowMilestoneInjection>> {
        let pending_limit = limit.clamp(1, 200);
        let requested_events = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT req.id, req.run_id, req.seq, req.type, req.payload_json, req.created_at
                 FROM workflow_events req
                 WHERE req.type = 'workflow_milestone_injection_requested'
                   AND NOT EXISTS (
                     SELECT 1
                     FROM workflow_events settled
                     WHERE settled.run_id = req.run_id
                       AND settled.type IN (
                         'workflow_milestone_injection_delivered',
                         'workflow_milestone_injection_suppressed'
                       )
                       AND json_extract(settled.payload_json, '$.sourceEventType') =
                           json_extract(req.payload_json, '$.sourceEventType')
                       AND json_extract(settled.payload_json, '$.sourceEventSeq') =
                           json_extract(req.payload_json, '$.sourceEventSeq')
                   )
                 ORDER BY req.id ASC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![pending_limit as i64], row_to_event)?;
            collect_rows(rows)?
        };

        let mut pending = Vec::new();
        for requested in requested_events {
            let Some(source_event_type) = requested
                .payload
                .get("sourceEventType")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let Some(source_event_seq) = requested
                .payload
                .get("sourceEventSeq")
                .and_then(Value::as_i64)
            else {
                continue;
            };
            if self.workflow_milestone_injection_settled(
                &requested.run_id,
                &source_event_type,
                source_event_seq,
            )? {
                continue;
            }
            let Some(source_event) =
                self.get_workflow_event_by_seq(&requested.run_id, source_event_seq)?
            else {
                continue;
            };
            if source_event.event_type != source_event_type {
                continue;
            }
            pending.push(PendingWorkflowMilestoneInjection {
                run_id: requested.run_id.clone(),
                source_event_type,
                source_event_seq,
                requested_event_seq: requested.seq,
                requested_at: requested.created_at,
                source_event,
            });
            if pending.len() >= pending_limit {
                break;
            }
        }
        Ok(pending)
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

    fn resolve_saved_workflow_template_project_id(
        &self,
        session_id: &str,
        scope: SavedWorkflowTemplateScope,
        requested_project_id: Option<&str>,
    ) -> Result<Option<String>> {
        let requested_project_id = normalize_optional(requested_project_id);
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let row: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT incognito, project_id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (incognito, session_project_id) =
            row.ok_or_else(|| anyhow!("Session not found: {}", session_id))?;
        if incognito != 0 {
            return Err(anyhow!(
                "Cannot save durable workflow template for incognito session {}",
                session_id
            ));
        }
        match scope {
            SavedWorkflowTemplateScope::User => Ok(None),
            SavedWorkflowTemplateScope::Project => {
                let Some(session_project_id) = session_project_id else {
                    return Err(anyhow!(
                        "project-scoped workflow templates require a project session"
                    ));
                };
                if let Some(requested_project_id) = requested_project_id {
                    if requested_project_id != session_project_id {
                        return Err(anyhow!(
                            "workflow template project {} does not match session project {}",
                            requested_project_id,
                            session_project_id
                        ));
                    }
                }
                Ok(Some(session_project_id))
            }
        }
    }

    fn ensure_saved_workflow_template_visible_to_session(
        &self,
        template: &SavedWorkflowTemplate,
        session_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let row: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT incognito, project_id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (incognito, session_project_id) =
            row.ok_or_else(|| anyhow!("Session not found: {}", session_id))?;
        if incognito != 0 {
            return Err(anyhow!(
                "Cannot create durable workflow run from template for incognito session {}",
                session_id
            ));
        }
        if template.scope == SavedWorkflowTemplateScope::Project
            && template.project_id.as_deref() != session_project_id.as_deref()
        {
            return Err(anyhow!(
                "workflow template {} is scoped to project {:?}; session {} belongs to {:?}",
                template.id,
                template.project_id,
                session_id,
                session_project_id
            ));
        }
        Ok(())
    }
}

fn collect_handled_workflow_agent_ids(value: &Value, target: &mut HashSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_handled_workflow_agent_ids(item, target);
            }
        }
        Value::Object(map) => {
            let terminal = map
                .get("terminal")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| {
                    matches!(
                        map.get("status").and_then(Value::as_str),
                        Some("completed" | "error" | "timeout" | "killed" | "not_found")
                    )
                });
            if terminal {
                if let Some(run_id) = map
                    .get("runId")
                    .or_else(|| map.get("run_id"))
                    .and_then(Value::as_str)
                {
                    target.insert(run_id.to_string());
                }
            }
            for key in ["runs", "agentResults"] {
                if let Some(nested) = map.get(key) {
                    collect_handled_workflow_agent_ids(nested, target);
                }
            }
        }
        _ => {}
    }
}

fn workflow_parent_injection_usage_snapshot_with_conn(
    conn: &Connection,
    run: &WorkflowRun,
) -> Result<WorkflowParentInjectionUsageSnapshot> {
    let escaped_run_id = escape_sql_like_literal(&run.id);
    let final_result_pattern = format!("%\"workflow_result\"%\"run_id\":\"{}\"%", escaped_run_id);
    let milestone_pattern = format!(
        "%\"workflow_result\"%\"run_id\":\"{}:workflow-event:%",
        escaped_run_id
    );
    let mut stmt = conn.prepare(
        "SELECT id
         FROM messages
         WHERE session_id = ?1
           AND role = ?2
           AND (
                attachments_meta LIKE ?3 ESCAPE '\\'
                OR attachments_meta LIKE ?4 ESCAPE '\\'
           )
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(
        params![
            &run.session_id,
            MessageRole::User.as_str(),
            final_result_pattern,
            milestone_pattern,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let mut trigger_message_ids = collect_rows(rows)?;
    trigger_message_ids.sort_unstable();
    trigger_message_ids.dedup();

    let mut snapshot = WorkflowParentInjectionUsageSnapshot {
        attribution: "no_workflow_result_injection_messages".to_string(),
        ..Default::default()
    };
    if trigger_message_ids.is_empty() {
        return Ok(snapshot);
    }

    snapshot.turns = trigger_message_ids.len() as i64;
    for trigger_message_id in trigger_message_ids {
        let next_user_message_id = conn
            .query_row(
                "SELECT id
                 FROM messages
                 WHERE session_id = ?1
                   AND role = ?2
                   AND id > ?3
                 ORDER BY id ASC
                 LIMIT 1",
                params![
                    &run.session_id,
                    MessageRole::User.as_str(),
                    trigger_message_id,
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let upper_bound = next_user_message_id.unwrap_or(i64::MAX);
        let message_usage = conn.query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(
                    CASE
                        WHEN COALESCE(tokens_in_last, tokens_in, 0) > 0
                        THEN COALESCE(tokens_in_last, tokens_in, 0)
                        ELSE 0
                    END
                ), 0),
                COALESCE(SUM(
                    CASE
                        WHEN COALESCE(tokens_out, 0) > 0 THEN tokens_out
                        ELSE 0
                    END
                ), 0)
             FROM messages
             WHERE session_id = ?1
               AND id >= ?2
               AND id < ?3
               AND role IN (?4, ?5)",
            params![
                &run.session_id,
                trigger_message_id,
                upper_bound,
                MessageRole::User.as_str(),
                MessageRole::Assistant.as_str(),
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        let provider_usage = conn.query_row(
            "SELECT
                COUNT(u.id),
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0),
                COALESCE(SUM(u.cache_creation_input_tokens), 0),
                COALESCE(SUM(u.cache_read_input_tokens), 0)
             FROM messages m
             JOIN model_usage_events u ON u.request_key = ('message:' || m.id)
             WHERE m.session_id = ?1
               AND m.id >= ?2
               AND m.id < ?3
               AND m.role = ?4
               AND u.kind = ?5",
            params![
                &run.session_id,
                trigger_message_id,
                upper_bound,
                MessageRole::Assistant.as_str(),
                crate::model_usage::KIND_CHAT,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;

        snapshot.messages = snapshot.messages.saturating_add(message_usage.0);
        snapshot.input_tokens = snapshot.input_tokens.saturating_add(message_usage.1);
        snapshot.output_tokens = snapshot.output_tokens.saturating_add(message_usage.2);
        snapshot.provider_events = snapshot.provider_events.saturating_add(provider_usage.0);
        snapshot.provider_input_tokens = snapshot
            .provider_input_tokens
            .saturating_add(provider_usage.1);
        snapshot.provider_output_tokens = snapshot
            .provider_output_tokens
            .saturating_add(provider_usage.2);
        snapshot.provider_cache_creation_input_tokens = snapshot
            .provider_cache_creation_input_tokens
            .saturating_add(provider_usage.3);
        snapshot.provider_cache_read_input_tokens = snapshot
            .provider_cache_read_input_tokens
            .saturating_add(provider_usage.4);
    }
    snapshot.total_tokens = snapshot.input_tokens.saturating_add(snapshot.output_tokens);
    snapshot.provider_total_tokens = snapshot
        .provider_input_tokens
        .saturating_add(snapshot.provider_output_tokens);
    snapshot.attribution = if snapshot.provider_events > 0 {
        "workflow_result_message_boundary+model_usage_events.request_key=message_id".to_string()
    } else {
        "workflow_result_message_boundary".to_string()
    };
    Ok(snapshot)
}

fn escape_sql_like_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
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
        goal_criterion_id: row.get(14)?,
        goal_criterion_text: row.get(15)?,
        goal_criterion_kind: row.get(16)?,
        goal_revision: row.get(17)?,
        worktree_id: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        completed_at: row.get(21)?,
    })
}

fn row_to_saved_template(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedWorkflowTemplate> {
    let scope: String = row.get(3)?;
    let budget_json: String = row.get(9)?;
    Ok(SavedWorkflowTemplate {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        scope: parse_saved_template_scope_sql(&scope)?,
        project_id: row.get(4)?,
        kind: row.get(5)?,
        execution_mode: row.get(6)?,
        script_hash: row.get(7)?,
        script_source: row.get(8)?,
        budget: json_from_sql(&budget_json)?,
        source_run_id: row.get(10)?,
        enabled: row.get::<_, i64>(11)? != 0,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn workflow_run_goal_criterion_metadata(run: &WorkflowRun) -> Option<Value> {
    let id = run.goal_criterion_id.as_deref()?;
    Some(json!({
        "id": id,
        "text": run.goal_criterion_text.as_deref(),
        "kind": run.goal_criterion_kind.as_deref(),
        "goalRevision": run.goal_revision,
    }))
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

#[derive(Debug, Clone, Copy)]
enum WorkflowOwnerClaim {
    Launch,
    Recovery,
}

fn workflow_owner_pid(owner: &str) -> Option<u32> {
    owner
        .rsplit_once(":pid:")
        .and_then(|(_, raw)| raw.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
}

fn workflow_owner_recoverable(owner: Option<&str>) -> bool {
    let Some(owner) = owner.map(str::trim).filter(|owner| !owner.is_empty()) else {
        return true;
    };
    let Some(pid) = workflow_owner_pid(owner) else {
        return false;
    };
    !crate::platform::pid_alive(pid)
}

fn workflow_run_owner_recoverable(state: WorkflowRunState, owner: Option<&str>) -> bool {
    match state {
        WorkflowRunState::Draft => {
            owner.map(str::trim).is_some_and(|owner| !owner.is_empty())
                && workflow_owner_recoverable(owner)
        }
        WorkflowRunState::Running | WorkflowRunState::Recovering => {
            workflow_owner_recoverable(owner)
        }
        _ => false,
    }
}

fn workflow_owner_claim_target_state(
    state: WorkflowRunState,
    owner: Option<&str>,
    claim: WorkflowOwnerClaim,
) -> Option<WorkflowRunState> {
    match (claim, state) {
        (WorkflowOwnerClaim::Launch, WorkflowRunState::Draft) => {
            workflow_owner_recoverable(owner).then_some(WorkflowRunState::Draft)
        }
        (WorkflowOwnerClaim::Launch, WorkflowRunState::Running | WorkflowRunState::Recovering)
        | (
            WorkflowOwnerClaim::Recovery,
            WorkflowRunState::Running | WorkflowRunState::Recovering,
        ) => workflow_owner_recoverable(owner).then_some(WorkflowRunState::Recovering),
        (WorkflowOwnerClaim::Recovery, WorkflowRunState::Draft) => {
            workflow_run_owner_recoverable(state, owner).then_some(WorkflowRunState::Draft)
        }
        _ => None,
    }
}

fn workflow_last_activity(
    run: &WorkflowRun,
    latest_event: Option<&WorkflowEvent>,
    now: &DateTime<Utc>,
) -> (Option<String>, Option<i64>) {
    let run_updated = DateTime::parse_from_rfc3339(&run.updated_at)
        .map(|dt| dt.with_timezone(&Utc))
        .ok();
    let event_created = latest_event
        .and_then(|event| DateTime::parse_from_rfc3339(&event.created_at).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let last = match (run_updated, event_created) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    (
        last.map(|dt| dt.to_rfc3339()),
        last.map(|dt| (*now - dt).num_seconds().max(0)),
    )
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

fn parse_saved_template_scope_sql(value: &str) -> rusqlite::Result<SavedWorkflowTemplateScope> {
    SavedWorkflowTemplateScope::from_str(value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown saved workflow template scope: {value}").into(),
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

fn normalize_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn clamp_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn normalize_saved_template_name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("saved workflow template name must not be empty"));
    }
    Ok(clamp_chars(value, SAVED_WORKFLOW_TEMPLATE_NAME_MAX_CHARS))
}

fn normalize_saved_template_description(value: Option<&str>) -> Option<String> {
    normalize_optional(value)
        .map(|value| clamp_chars(value, SAVED_WORKFLOW_TEMPLATE_DESCRIPTION_MAX_CHARS))
}
