//! Durable `/loop` control plane.
//!
//! Loop is the recurrence layer above Goal / Workflow. It does not describe
//! execution strength; it schedules repeated triggers and records why each
//! trigger fired. The scheduler itself is cron: this module owns the session-
//! scoped control state and trace rows, while `cron` owns reliable timing.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cron::{CronDB, CronPayload, CronSchedule, NewCronJob};
use crate::session::{MessageRole, SessionDB};

const LOOP_TRACE_MAX_BYTES: usize = 64 * 1024;
const DEFAULT_UNTIL_INTERVAL_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopState {
    Active,
    Paused,
    Completed,
    Cancelled,
    Blocked,
}

impl LoopState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::Cancelled),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    pub fn can_resume(self) -> bool {
        matches!(self, Self::Paused | Self::Blocked)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopRunState {
    Running,
    Queued,
    Injected,
    Succeeded,
    Empty,
    Failed,
    Cancelled,
    Skipped,
}

impl LoopRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Injected => "injected",
            Self::Succeeded => "succeeded",
            Self::Empty => "empty",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "queued" => Some(Self::Queued),
            "injected" => Some(Self::Injected),
            "succeeded" => Some(Self::Succeeded),
            "empty" => Some(Self::Empty),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopTriggerKind {
    Interval,
    Cron,
    Condition,
    Event,
}

impl LoopTriggerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interval => "interval",
            Self::Cron => "cron",
            Self::Condition => "condition",
            Self::Event => "event",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "interval" => Some(Self::Interval),
            "cron" => Some(Self::Cron),
            "condition" => Some(Self::Condition),
            "event" => Some(Self::Event),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExecutionStrategy {
    Continue,
    Workflow,
}

impl Default for LoopExecutionStrategy {
    fn default() -> Self {
        Self::Continue
    }
}

impl LoopExecutionStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Workflow => "workflow",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "continue" => Some(Self::Continue),
            "workflow" => Some(Self::Workflow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopSchedule {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub cron_job_id: String,
    pub prompt: String,
    pub trigger_kind: LoopTriggerKind,
    pub trigger_spec: Value,
    pub execution_strategy: LoopExecutionStrategy,
    pub state: LoopState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<i64>,
    pub run_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_runtime_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_budget_micros: Option<i64>,
    pub approval_policy_snapshot: Value,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRun {
    pub id: String,
    pub loop_id: String,
    pub cron_job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_run_log_id: Option<i64>,
    pub session_id: String,
    pub seq: i64,
    pub state: LoopRunState,
    pub trigger_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub trace: Value,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopSnapshot {
    pub schedule: LoopSchedule,
    #[serde(default)]
    pub runs: Vec<LoopRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLoopScheduleInput {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub prompt: String,
    pub trigger_kind: LoopTriggerKind,
    #[serde(default)]
    pub trigger_spec: Value,
    #[serde(default)]
    pub execution_strategy: LoopExecutionStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runtime_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_budget_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoopRunAdmission {
    pub loop_id: String,
    pub run_id: String,
    pub session_id: String,
    pub prompt: String,
    pub trigger_kind: LoopTriggerKind,
    pub trigger_spec: Value,
    pub execution_strategy: LoopExecutionStrategy,
    pub agent_id: String,
    pub goal_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoopWorkflowLaunch {
    pub run_id: String,
    pub workflow_kind: String,
    pub execution_mode: String,
    pub template_id: String,
    pub template_version: String,
    pub requires_approval: bool,
}

#[derive(Debug, Clone)]
pub struct LoopRunRejection {
    pub loop_id: Option<String>,
    pub reason: String,
    pub pause_cron_job: bool,
}

#[derive(Debug, Clone)]
pub enum LoopRunDecision {
    NotLoop,
    Admit(LoopRunAdmission),
    Reject(LoopRunRejection),
}

#[derive(Debug, Clone)]
pub struct LoopAfterRunAction {
    pub loop_id: Option<String>,
    pub pause_cron_job: bool,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS loop_schedules (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            goal_id TEXT,
            cron_job_id TEXT NOT NULL UNIQUE,
            prompt TEXT NOT NULL,
            trigger_kind TEXT NOT NULL,
            trigger_spec_json TEXT NOT NULL DEFAULT '{}',
            execution_strategy TEXT NOT NULL DEFAULT 'continue',
            state TEXT NOT NULL,
            max_runs INTEGER,
            run_count INTEGER NOT NULL DEFAULT 0,
            max_runtime_secs INTEGER,
            token_budget INTEGER,
            cost_budget_micros INTEGER,
            approval_policy_snapshot_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            blocked_reason TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS loop_runs (
            id TEXT PRIMARY KEY,
            loop_id TEXT NOT NULL,
            cron_job_id TEXT NOT NULL,
            cron_run_log_id INTEGER,
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            state TEXT NOT NULL,
            trigger_reason TEXT NOT NULL,
            result_summary TEXT,
            error TEXT,
            trace_json TEXT NOT NULL DEFAULT '{}',
            started_at TEXT NOT NULL,
            finished_at TEXT,
            FOREIGN KEY (loop_id) REFERENCES loop_schedules(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_loop_schedules_session_updated
            ON loop_schedules(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_loop_schedules_state
            ON loop_schedules(state);
        CREATE INDEX IF NOT EXISTS idx_loop_schedules_goal
            ON loop_schedules(goal_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_loop_runs_loop_seq
            ON loop_runs(loop_id, seq DESC);
        CREATE INDEX IF NOT EXISTS idx_loop_runs_cron
            ON loop_runs(cron_job_id, started_at DESC);",
    )?;
    ensure_loop_column(
        conn,
        "execution_strategy",
        "ALTER TABLE loop_schedules ADD COLUMN execution_strategy TEXT NOT NULL DEFAULT 'continue';",
    )?;
    Ok(())
}

fn ensure_loop_column(conn: &Connection, column: &str, alter_sql: &str) -> Result<()> {
    let query = format!("SELECT {column} FROM loop_schedules LIMIT 1");
    if conn.prepare(&query).is_ok() {
        return Ok(());
    }
    conn.execute(alter_sql, [])?;
    Ok(())
}

impl SessionDB {
    pub fn create_loop_schedule(
        &self,
        cron_db: &CronDB,
        input: CreateLoopScheduleInput,
    ) -> Result<LoopSchedule> {
        if normalize_positive(input.cost_budget_micros).is_some() {
            return Err(anyhow!(
                "loop cost budget requires provider cost ledger support; use max runs, max runtime, or token budget for now"
            ));
        }
        let now = now_rfc3339();
        let id = format!("loop_{}", uuid::Uuid::new_v4().simple());
        let (goal_id, agent_id, prompt) = self.resolve_loop_create_context(&input)?;
        if input.execution_strategy == LoopExecutionStrategy::Workflow {
            if input.trigger_kind != LoopTriggerKind::Interval {
                return Err(anyhow!(
                    "loop workflow execution currently supports interval triggers only; condition loops still require conversation continuation"
                ));
            }
            let goal_id = goal_id
                .as_deref()
                .ok_or_else(|| anyhow!("loop workflow execution requires a bound Goal"))?;
            let goal = self
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            if goal
                .workflow_template_id
                .as_deref()
                .and_then(non_empty)
                .is_none()
            {
                return Err(anyhow!(
                    "loop workflow execution requires the bound Goal to select a domain workflow template"
                ));
            }
        }
        let schedule = cron_schedule_from_loop(&input)?;
        let trigger_spec = normalized_trigger_spec(input.trigger_kind, &input.trigger_spec)?;
        let trigger_spec_json = stable_json(&trigger_spec)?;
        let approval_policy_snapshot = json!({
            "permission": "inherits_session",
            "scheduler": "cron",
            "executionStrategy": input.execution_strategy,
            "unattended": "permission_engine_fail_closed_or_policy",
        });
        let approval_policy_snapshot_json = stable_json(&approval_policy_snapshot)?;

        let cron_job = cron_db.add_job(&NewCronJob {
            name: loop_job_name(goal_id.as_deref(), &prompt),
            description: Some(format!(
                "Loop schedule {} for session {}. Managed by /loop; pause/resume/stop from the loop control plane.",
                short_id(&id),
                input.session_id
            )),
            project_id: None,
            schedule,
            payload: CronPayload::SessionLoop {
                loop_id: id.clone(),
                session_id: input.session_id.clone(),
                prompt: prompt.clone(),
                agent_id: Some(agent_id),
                goal_id: goal_id.clone(),
            },
            max_failures: Some(5),
            notify_on_complete: Some(false),
            delivery_targets: Some(Vec::new()),
            prefix_delivery_with_name: Some(false),
            job_timeout_secs: input.max_runtime_secs.map(|v| v.max(30) as u64),
            permission_mode_override: None,
            sandbox_mode_override: None,
        })?;

        let insert_result = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO loop_schedules (
                    id, session_id, goal_id, cron_job_id, prompt, trigger_kind, trigger_spec_json, execution_strategy,
                    state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    approval_policy_snapshot_json, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12, ?13, ?14, ?15, ?15)",
                params![
                    id,
                    input.session_id,
                    goal_id,
                    cron_job.id,
                    prompt,
                    input.trigger_kind.as_str(),
                    trigger_spec_json,
                    input.execution_strategy.as_str(),
                    LoopState::Active.as_str(),
                    normalize_positive(input.max_runs),
                    normalize_positive(input.max_runtime_secs),
                    normalize_positive(input.token_budget),
                    normalize_positive(input.cost_budget_micros),
                    approval_policy_snapshot_json,
                    now,
                ],
            )
        };
        if let Err(err) = insert_result {
            let _ = cron_db.delete_job(&cron_job.id);
            return Err(err.into());
        }

        let schedule = self
            .get_loop_schedule(&id)?
            .ok_or_else(|| anyhow!("loop schedule {} was not persisted", id))?;
        if let Some(goal_id) = schedule.goal_id.as_deref() {
            let _ = self.link_goal_target(
                goal_id,
                "loop_schedule",
                &schedule.id,
                "recurring_trigger",
                json!({
                    "cronJobId": schedule.cron_job_id,
                    "triggerKind": schedule.trigger_kind,
                    "maxRuns": schedule.max_runs,
                    "maxRuntimeSecs": schedule.max_runtime_secs,
                }),
            );
        }
        emit_loop_event("loop:changed", &schedule);
        Ok(schedule)
    }

    pub fn get_loop_schedule(&self, loop_id: &str) -> Result<Option<LoopSchedule>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
            "SELECT id, session_id, goal_id, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    approval_policy_snapshot_json, created_at, updated_at, completed_at, blocked_reason
             FROM loop_schedules WHERE id = ?1",
                params![loop_id],
                row_to_loop_schedule,
            )
            .optional()?)
    }

    pub fn list_loop_schedules_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<LoopSchedule>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, goal_id, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    approval_policy_snapshot_json, created_at, updated_at, completed_at, blocked_reason
             FROM loop_schedules
             WHERE session_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_loop_schedule)?;
        collect_rows(rows)
    }

    pub fn loop_snapshot(&self, loop_id: &str, run_limit: usize) -> Result<Option<LoopSnapshot>> {
        let Some(schedule) = self.get_loop_schedule(loop_id)? else {
            return Ok(None);
        };
        let runs = self.list_loop_runs(loop_id, run_limit)?;
        Ok(Some(LoopSnapshot { schedule, runs }))
    }

    pub fn list_loop_runs(&self, loop_id: &str, limit: usize) -> Result<Vec<LoopRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, loop_id, cron_job_id, cron_run_log_id, session_id, seq, state,
                    trigger_reason, result_summary, error, trace_json, started_at, finished_at
             FROM loop_runs
             WHERE loop_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![loop_id, limit as i64], row_to_loop_run)?;
        collect_rows(rows)
    }

    pub fn pause_loop_schedule(&self, cron_db: &CronDB, loop_id: &str) -> Result<LoopSchedule> {
        let schedule = self.transition_loop_schedule(loop_id, LoopState::Paused, None)?;
        cron_db.toggle_job(&schedule.cron_job_id, false)?;
        Ok(schedule)
    }

    pub fn resume_loop_schedule(&self, cron_db: &CronDB, loop_id: &str) -> Result<LoopSchedule> {
        let current = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        if !current.state.can_resume() {
            return Err(anyhow!(
                "loop schedule {} cannot resume from state {}",
                loop_id,
                current.state.as_str()
            ));
        }
        let schedule = self.transition_loop_schedule(loop_id, LoopState::Active, None)?;
        cron_db.toggle_job(&schedule.cron_job_id, true)?;
        Ok(schedule)
    }

    pub fn stop_loop_schedule(&self, cron_db: &CronDB, loop_id: &str) -> Result<LoopSchedule> {
        let schedule = self.transition_loop_schedule(loop_id, LoopState::Cancelled, None)?;
        cron_db.toggle_job(&schedule.cron_job_id, false)?;
        Ok(schedule)
    }

    pub fn prepare_loop_cron_run(
        &self,
        cron_job_id: &str,
        session_id: &str,
        started_at: &str,
    ) -> Result<LoopRunDecision> {
        let Some(schedule) = self.loop_schedule_for_cron_job(cron_job_id)? else {
            return Ok(LoopRunDecision::NotLoop);
        };
        if schedule.session_id != session_id {
            return Ok(LoopRunDecision::Reject(LoopRunRejection {
                loop_id: Some(schedule.id),
                reason: "loop parent session mismatch".to_string(),
                pause_cron_job: true,
            }));
        }
        if schedule.state != LoopState::Active {
            return Ok(LoopRunDecision::Reject(LoopRunRejection {
                loop_id: Some(schedule.id),
                reason: format!("loop is {}", schedule.state.as_str()),
                pause_cron_job: true,
            }));
        }
        if let Some(limit) = schedule.max_runs {
            if schedule.run_count >= limit {
                self.complete_loop_due_to_limit(&schedule, "max_runs_reached")?;
                return Ok(LoopRunDecision::Reject(LoopRunRejection {
                    loop_id: Some(schedule.id),
                    reason: "max runs reached".to_string(),
                    pause_cron_job: true,
                }));
            }
        }
        if let Some(limit) = schedule.max_runtime_secs {
            if loop_elapsed_secs(&schedule.created_at)? >= limit {
                self.complete_loop_due_to_limit(&schedule, "max_runtime_reached")?;
                return Ok(LoopRunDecision::Reject(LoopRunRejection {
                    loop_id: Some(schedule.id),
                    reason: "max runtime reached".to_string(),
                    pause_cron_job: true,
                }));
            }
        }
        if let Some(limit) = schedule.token_budget {
            let used = self.loop_tokens_used_since(&schedule.session_id, &schedule.created_at)?;
            if used >= limit {
                let reason = format!("loop token budget exhausted ({used}/{limit})");
                self.block_loop_schedule(&schedule, &reason)?;
                return Ok(LoopRunDecision::Reject(LoopRunRejection {
                    loop_id: Some(schedule.id),
                    reason,
                    pause_cron_job: true,
                }));
            }
        }
        if let Some(goal_id) = schedule.goal_id.as_deref() {
            if let Err(err) = self.ensure_goal_budget_allows_new_workflow(goal_id) {
                self.block_loop_schedule(&schedule, &format!("goal budget exhausted: {err}"))?;
                return Ok(LoopRunDecision::Reject(LoopRunRejection {
                    loop_id: Some(schedule.id),
                    reason: err.to_string(),
                    pause_cron_job: true,
                }));
            }
        }

        let run_id = format!("lrun_{}", uuid::Uuid::new_v4().simple());
        let seq = schedule.run_count + 1;
        let trigger_reason = format!(
            "{} trigger from cron job {}",
            schedule.trigger_kind.as_str(),
            cron_job_id
        );
        let trace = json!({
            "triggerKind": schedule.trigger_kind,
            "triggerSpec": schedule.trigger_spec,
            "executionStrategy": schedule.execution_strategy,
            "cronJobId": cron_job_id,
            "seq": seq,
        });
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO loop_runs (
                    id, loop_id, cron_job_id, session_id, seq, state, trigger_reason,
                    trace_json, started_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    run_id,
                    schedule.id,
                    cron_job_id,
                    session_id,
                    seq,
                    LoopRunState::Running.as_str(),
                    trigger_reason,
                    stable_json(&trace)?,
                    started_at,
                ],
            )?;
        }
        Ok(LoopRunDecision::Admit(LoopRunAdmission {
            loop_id: schedule.id,
            run_id,
            session_id: session_id.to_string(),
            prompt: schedule.prompt,
            trigger_kind: schedule.trigger_kind,
            trigger_spec: schedule.trigger_spec,
            execution_strategy: schedule.execution_strategy,
            agent_id: self
                .get_session(session_id)?
                .map(|m| m.agent_id)
                .unwrap_or_else(|| "ha-main".to_string()),
            goal_id: schedule.goal_id,
        }))
    }

    pub fn finish_loop_cron_run(
        &self,
        cron_job_id: &str,
        loop_run_id: Option<&str>,
        cron_run_log_id: Option<i64>,
        state: LoopRunState,
        result_summary: Option<&str>,
        error: Option<&str>,
        finished_at: &str,
    ) -> Result<LoopAfterRunAction> {
        self.finish_loop_cron_run_with_trace(
            cron_job_id,
            loop_run_id,
            cron_run_log_id,
            state,
            result_summary,
            error,
            finished_at,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_loop_cron_run_with_trace(
        &self,
        cron_job_id: &str,
        loop_run_id: Option<&str>,
        cron_run_log_id: Option<i64>,
        state: LoopRunState,
        result_summary: Option<&str>,
        error: Option<&str>,
        finished_at: &str,
        extra_trace: Option<Value>,
    ) -> Result<LoopAfterRunAction> {
        let Some(schedule) = self.loop_schedule_for_cron_job(cron_job_id)? else {
            return Ok(LoopAfterRunAction {
                loop_id: None,
                pause_cron_job: false,
            });
        };
        let run_id = match loop_run_id {
            Some(id) => Some(id.to_string()),
            None => self.latest_running_loop_run_id(&schedule.id)?,
        };
        if let Some(run_id) = run_id.as_deref() {
            let mut trace_patch = json!({
                "cronRunLogId": cron_run_log_id,
                "finishedAt": finished_at,
            });
            if let Some(extra) = extra_trace {
                if let (Some(base), Some(extra)) = (trace_patch.as_object_mut(), extra.as_object())
                {
                    for (key, value) in extra {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
            self.update_loop_run_terminal(
                run_id,
                cron_run_log_id,
                state,
                result_summary,
                error,
                trace_patch,
                finished_at,
            )?;
        }
        let counted_run = run_id.is_some();
        let next_count = if counted_run {
            schedule.run_count + 1
        } else {
            schedule.run_count
        };
        let mut pause = false;
        let mut next_state = schedule.state;
        let mut blocked_reason = None;
        if schedule.state == LoopState::Active && counted_run {
            if state == LoopRunState::Succeeded
                && schedule.trigger_kind == LoopTriggerKind::Condition
                && condition_satisfied_marker(result_summary)
            {
                next_state = LoopState::Completed;
                pause = true;
            }
            if let Some(max_runs) = schedule.max_runs {
                if next_count >= max_runs {
                    next_state = LoopState::Completed;
                    pause = true;
                }
            }
            if let Some(max_runtime) = schedule.max_runtime_secs {
                if loop_elapsed_secs(&schedule.created_at)? >= max_runtime {
                    next_state = LoopState::Completed;
                    pause = true;
                }
            }
            if state == LoopRunState::Failed {
                blocked_reason = error.map(str::to_string);
            }
        }
        self.bump_loop_after_run(&schedule.id, next_count, next_state, blocked_reason)?;
        if let Some(goal_id) = schedule.goal_id.as_deref() {
            let _ = self.link_goal_target(
                goal_id,
                "loop_run",
                run_id.as_deref().unwrap_or(cron_job_id),
                "loop_triggered",
                json!({
                    "loopId": schedule.id,
                    "cronJobId": cron_job_id,
                    "state": state,
                    "summary": result_summary,
                    "error": error,
                }),
            );
        }
        Ok(LoopAfterRunAction {
            loop_id: Some(schedule.id),
            pause_cron_job: pause || next_state.is_terminal(),
        })
    }

    pub fn create_loop_workflow_run(
        &self,
        admission: &LoopRunAdmission,
    ) -> Result<LoopWorkflowLaunch> {
        let goal_id = admission
            .goal_id
            .as_deref()
            .ok_or_else(|| anyhow!("loop workflow execution requires a bound Goal"))?;
        let goal = self
            .get_goal(goal_id)?
            .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
        if goal.session_id != admission.session_id {
            return Err(anyhow!(
                "goal {} belongs to session {}; expected {}",
                goal.id,
                goal.session_id,
                admission.session_id
            ));
        }
        let template_id = goal
            .workflow_template_id
            .as_deref()
            .and_then(non_empty)
            .ok_or_else(|| {
                anyhow!(
                    "loop workflow execution requires Goal {} to bind a domain workflow template",
                    goal.id
                )
            })?;
        let user_context = loop_workflow_user_context(admission);
        let draft =
            self.preview_domain_workflow(crate::domain_workflow::PreviewDomainWorkflowInput {
                template_id: template_id.to_string(),
                version: goal.workflow_template_version.clone(),
                session_id: admission.session_id.clone(),
                goal_id: Some(goal.id.clone()),
                task_type: goal.workflow_task_type.clone(),
                objective: Some(goal.objective.clone()),
                mode_override: None,
                user_context: Some(user_context),
                require_plan_confirmation: false,
            })?;
        if !draft.script_preview.can_create {
            return Err(anyhow!(
                "domain workflow draft failed preflight: {}",
                draft.script_preview.gate_feedback
            ));
        }
        let run = self.create_workflow_run(crate::workflow::CreateWorkflowRunInput {
            session_id: admission.session_id.clone(),
            kind: draft.workflow_kind.clone(),
            execution_mode: draft.execution_mode.clone(),
            script_source: draft.script_source,
            budget: json!({}),
            parent_run_id: None,
            origin: Some(format!("loop:{}", admission.loop_id)),
            goal_id: Some(goal.id.clone()),
            worktree_id: None,
        })?;
        let _ = self.append_workflow_event(
            &run.id,
            "run_derived_from_loop",
            json!({
                "loopId": admission.loop_id,
                "loopRunId": admission.run_id,
                "triggerKind": admission.trigger_kind,
                "triggerSpec": admission.trigger_spec,
                "templateId": draft.template.id,
                "templateVersion": draft.template.version,
            }),
        );
        Ok(LoopWorkflowLaunch {
            run_id: run.id,
            workflow_kind: draft.workflow_kind,
            execution_mode: draft.execution_mode,
            template_id: draft.template.id,
            template_version: draft.template.version,
            requires_approval: draft.script_preview.requires_approval,
        })
    }

    fn resolve_loop_create_context(
        &self,
        input: &CreateLoopScheduleInput,
    ) -> Result<(Option<String>, String, String)> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let session: Option<(String, i64)> = conn
            .query_row(
                "SELECT agent_id, incognito FROM sessions WHERE id = ?1",
                params![input.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (session_agent_id, incognito) =
            session.ok_or_else(|| anyhow!("session not found: {}", input.session_id))?;
        if incognito != 0 {
            return Err(anyhow!(
                "Cannot create durable loop schedule for incognito session {}",
                input.session_id
            ));
        }
        let goal_id = match input.goal_id.as_deref() {
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
                     WHERE session_id = ?1 AND state IN ('active','paused','evaluating','blocked')
                     ORDER BY updated_at DESC
                     LIMIT 1",
                    params![input.session_id],
                    |row| row.get(0),
                )
                .optional()?,
        };
        let prompt = input.prompt.trim();
        if prompt.is_empty() && goal_id.is_none() {
            return Err(anyhow!(
                "/loop requires either an active goal or an explicit recurring prompt"
            ));
        }
        let prompt = if prompt.is_empty() {
            "Continue the active goal. Check whether the completion criteria are satisfied; if not, make the next useful step and record evidence.".to_string()
        } else {
            prompt.to_string()
        };
        Ok((
            goal_id,
            input
                .agent_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(session_agent_id),
            prompt,
        ))
    }

    fn loop_schedule_for_cron_job(&self, cron_job_id: &str) -> Result<Option<LoopSchedule>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
            "SELECT id, session_id, goal_id, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    approval_policy_snapshot_json, created_at, updated_at, completed_at, blocked_reason
             FROM loop_schedules WHERE cron_job_id = ?1",
                params![cron_job_id],
                row_to_loop_schedule,
            )
            .optional()?)
    }

    fn transition_loop_schedule(
        &self,
        loop_id: &str,
        state: LoopState,
        blocked_reason: Option<&str>,
    ) -> Result<LoopSchedule> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE loop_schedules
                 SET state = ?2, updated_at = ?3,
                     completed_at = CASE WHEN ?4 != 0 THEN ?3 ELSE completed_at END,
                     blocked_reason = ?5
                 WHERE id = ?1",
                params![
                    loop_id,
                    state.as_str(),
                    now,
                    if state.is_terminal() { 1i64 } else { 0i64 },
                    blocked_reason,
                ],
            )?;
        }
        let schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        emit_loop_event("loop:changed", &schedule);
        Ok(schedule)
    }

    fn complete_loop_due_to_limit(&self, schedule: &LoopSchedule, reason: &str) -> Result<()> {
        let _ = self.transition_loop_schedule(&schedule.id, LoopState::Completed, None)?;
        let _ = self.insert_skipped_loop_run(schedule, reason)?;
        Ok(())
    }

    fn block_loop_schedule(&self, schedule: &LoopSchedule, reason: &str) -> Result<()> {
        let _ = self.transition_loop_schedule(&schedule.id, LoopState::Blocked, Some(reason))?;
        let _ = self.insert_skipped_loop_run(schedule, reason)?;
        Ok(())
    }

    fn insert_skipped_loop_run(&self, schedule: &LoopSchedule, reason: &str) -> Result<String> {
        let run_id = format!("lrun_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let seq = schedule.run_count + 1;
        let trace = json!({
            "reason": reason,
            "cronJobId": schedule.cron_job_id,
            "skipped": true,
        });
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO loop_runs (
                id, loop_id, cron_job_id, session_id, seq, state, trigger_reason,
                error, trace_json, started_at, finished_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                run_id,
                schedule.id,
                schedule.cron_job_id,
                schedule.session_id,
                seq,
                LoopRunState::Skipped.as_str(),
                reason,
                reason,
                stable_json(&trace)?,
                now,
            ],
        )?;
        Ok(run_id)
    }

    fn update_loop_run_terminal(
        &self,
        run_id: &str,
        cron_run_log_id: Option<i64>,
        state: LoopRunState,
        result_summary: Option<&str>,
        error: Option<&str>,
        trace_patch: Value,
        finished_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let existing_trace: Option<String> = conn
            .query_row(
                "SELECT trace_json FROM loop_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut trace = existing_trace
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .unwrap_or_else(|| json!({}));
        if let (Some(base), Some(patch)) = (trace.as_object_mut(), trace_patch.as_object()) {
            for (key, value) in patch {
                base.insert(key.clone(), value.clone());
            }
        } else {
            trace = trace_patch;
        }
        let trace_json = bounded_json(&trace)?;
        conn.execute(
            "UPDATE loop_runs
             SET state = ?2,
                 cron_run_log_id = COALESCE(?3, cron_run_log_id),
                 result_summary = ?4,
                 error = ?5,
                 trace_json = ?6,
                 finished_at = ?7
             WHERE id = ?1",
            params![
                run_id,
                state.as_str(),
                cron_run_log_id,
                result_summary,
                error,
                trace_json,
                finished_at,
            ],
        )?;
        Ok(())
    }

    fn bump_loop_after_run(
        &self,
        loop_id: &str,
        run_count: i64,
        state: LoopState,
        blocked_reason: Option<String>,
    ) -> Result<()> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_schedules
             SET run_count = ?2,
                 state = ?3,
                 blocked_reason = COALESCE(?4, blocked_reason),
                 completed_at = CASE WHEN ?5 != 0 THEN COALESCE(completed_at, ?6) ELSE completed_at END,
                 updated_at = ?6
             WHERE id = ?1",
            params![
                loop_id,
                run_count,
                state.as_str(),
                blocked_reason,
                if state.is_terminal() { 1i64 } else { 0i64 },
                now,
            ],
        )?;
        Ok(())
    }

    fn latest_running_loop_run_id(&self, loop_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
                "SELECT id FROM loop_runs
             WHERE loop_id = ?1 AND state = 'running'
             ORDER BY started_at DESC
             LIMIT 1",
                params![loop_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn summarize_latest_assistant_after(
        &self,
        session_id: &str,
        started_at: &str,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
                "SELECT content FROM messages
             WHERE session_id = ?1 AND role = ?2 AND timestamp >= ?3
             ORDER BY id DESC
             LIMIT 1",
                params![session_id, MessageRole::Assistant.as_str(), started_at],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|s| truncate_utf8(&s, 500).to_string()))
    }

    fn loop_tokens_used_since(&self, session_id: &str, since: &str) -> Result<i64> {
        let since = DateTime::parse_from_rfc3339(since)?.with_timezone(&Utc);
        let mut tokens_used = 0i64;
        for message in self.load_session_messages(session_id).unwrap_or_default() {
            let Some(message_at) = DateTime::parse_from_rfc3339(&message.timestamp)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
            else {
                continue;
            };
            if message_at < since {
                continue;
            }
            tokens_used += message
                .tokens_in_last
                .or(message.tokens_in)
                .unwrap_or(0)
                .max(0);
            tokens_used += message.tokens_out.unwrap_or(0).max(0);
        }
        Ok(tokens_used)
    }
}

fn cron_schedule_from_loop(input: &CreateLoopScheduleInput) -> Result<CronSchedule> {
    match input.trigger_kind {
        LoopTriggerKind::Interval | LoopTriggerKind::Condition => {
            let secs = input
                .trigger_spec
                .get("intervalSecs")
                .or_else(|| input.trigger_spec.get("interval_secs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(if input.trigger_kind == LoopTriggerKind::Condition {
                    DEFAULT_UNTIL_INTERVAL_SECS
                } else {
                    0
                });
            if secs <= 0 {
                return Err(anyhow!("loop interval requires positive intervalSecs"));
            }
            Ok(CronSchedule::Every {
                interval_ms: (secs as u64).saturating_mul(1000),
                start_at: None,
            })
        }
        LoopTriggerKind::Cron => {
            let expression = input
                .trigger_spec
                .get("expression")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("cron loop requires triggerSpec.expression"))?;
            let timezone = input
                .trigger_spec
                .get("timezone")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(CronSchedule::Cron {
                expression: expression.to_string(),
                timezone,
            })
        }
        LoopTriggerKind::Event => Err(anyhow!(
            "event-triggered loops are reserved for a future event bus integration"
        )),
    }
}

fn normalized_trigger_spec(kind: LoopTriggerKind, spec: &Value) -> Result<Value> {
    match kind {
        LoopTriggerKind::Interval => {
            let secs = spec
                .get("intervalSecs")
                .or_else(|| spec.get("interval_secs"))
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow!("loop interval requires intervalSecs"))?;
            Ok(json!({ "intervalSecs": secs }))
        }
        LoopTriggerKind::Condition => {
            let secs = spec
                .get("intervalSecs")
                .or_else(|| spec.get("interval_secs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(DEFAULT_UNTIL_INTERVAL_SECS);
            let condition = spec
                .get("condition")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if condition.is_empty() {
                return Err(anyhow!("condition loop requires triggerSpec.condition"));
            }
            Ok(json!({ "intervalSecs": secs, "condition": condition }))
        }
        LoopTriggerKind::Cron | LoopTriggerKind::Event => Ok(spec.clone()),
    }
}

fn row_to_loop_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<LoopSchedule> {
    let trigger_kind: String = row.get(5)?;
    let trigger_spec_json: String = row.get(6)?;
    let execution_strategy: String = row.get(7)?;
    let state: String = row.get(8)?;
    let policy_json: String = row.get(14)?;
    Ok(LoopSchedule {
        id: row.get(0)?,
        session_id: row.get(1)?,
        goal_id: row.get(2)?,
        cron_job_id: row.get(3)?,
        prompt: row.get(4)?,
        trigger_kind: LoopTriggerKind::from_str(&trigger_kind).unwrap_or(LoopTriggerKind::Interval),
        trigger_spec: serde_json::from_str(&trigger_spec_json).unwrap_or_else(|_| json!({})),
        execution_strategy: LoopExecutionStrategy::from_str(&execution_strategy)
            .unwrap_or(LoopExecutionStrategy::Continue),
        state: LoopState::from_str(&state).unwrap_or(LoopState::Blocked),
        max_runs: row.get(9)?,
        run_count: row.get(10)?,
        max_runtime_secs: row.get(11)?,
        token_budget: row.get(12)?,
        cost_budget_micros: row.get(13)?,
        approval_policy_snapshot: serde_json::from_str(&policy_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        completed_at: row.get(17)?,
        blocked_reason: row.get(18)?,
    })
}

fn row_to_loop_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<LoopRun> {
    let state: String = row.get(6)?;
    let trace_json: String = row.get(10)?;
    Ok(LoopRun {
        id: row.get(0)?,
        loop_id: row.get(1)?,
        cron_job_id: row.get(2)?,
        cron_run_log_id: row.get(3)?,
        session_id: row.get(4)?,
        seq: row.get(5)?,
        state: LoopRunState::from_str(&state).unwrap_or(LoopRunState::Failed),
        trigger_reason: row.get(7)?,
        result_summary: row.get(8)?,
        error: row.get(9)?,
        trace: serde_json::from_str(&trace_json).unwrap_or_else(|_| json!({})),
        started_at: row.get(11)?,
        finished_at: row.get(12)?,
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

fn normalize_positive(value: Option<i64>) -> Option<i64> {
    value.filter(|v| *v > 0)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn loop_workflow_user_context(admission: &LoopRunAdmission) -> String {
    let trigger_spec =
        serde_json::to_string(&admission.trigger_spec).unwrap_or_else(|_| "{}".to_string());
    format!(
        "Loop trigger context:\n- loop_id: {}\n- loop_run_id: {}\n- trigger_kind: {}\n- trigger_spec: {}\n- recurring_prompt: {}\n",
        admission.loop_id,
        admission.run_id,
        admission.trigger_kind.as_str(),
        trigger_spec,
        admission.prompt
    )
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn loop_elapsed_secs(created_at: &str) -> Result<i64> {
    let created = DateTime::parse_from_rfc3339(created_at)?.with_timezone(&Utc);
    Ok((Utc::now() - created).num_seconds().max(0))
}

fn stable_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn bounded_json(value: &Value) -> Result<String> {
    let mut serialized = serde_json::to_string(value)?;
    if serialized.len() > LOOP_TRACE_MAX_BYTES {
        serialized = serde_json::to_string(&json!({
            "truncated": true,
            "preview": truncate_utf8(&serialized, LOOP_TRACE_MAX_BYTES),
        }))?;
    }
    Ok(serialized)
}

fn loop_job_name(goal_id: Option<&str>, prompt: &str) -> String {
    let subject = goal_id
        .map(|id| format!("goal {}", short_id(id)))
        .unwrap_or_else(|| truncate_utf8(prompt, 48).to_string());
    format!("[Loop] {}", subject)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn truncate_utf8(input: &str, max: usize) -> &str {
    if input.len() <= max {
        return input;
    }
    let mut end = max;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

pub fn build_loop_trigger_message(
    loop_id: &str,
    run_id: &str,
    goal_id: Option<&str>,
    trigger_kind: LoopTriggerKind,
    trigger_spec: &Value,
    prompt: &str,
) -> String {
    let goal = goal_id
        .map(|id| format!("<goal_id>{}</goal_id>\n", escape_xml(id)))
        .unwrap_or_default();
    let condition = if trigger_kind == LoopTriggerKind::Condition {
        trigger_spec
            .get("condition")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|condition| {
                format!(
                    "<condition>{}</condition>\n\
                     If the condition is already satisfied, start your response with the exact line \
                     \"LOOP_CONDITION_SATISFIED: <short reason>\" and do not continue work.\n",
                    escape_xml(condition)
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "<loop_trigger>\n\
         <loop_id>{}</loop_id>\n\
         <run_id>{}</run_id>\n\
         {}\
         {}\
         A scheduled loop trigger has fired. Follow the recurring prompt below. \
         If the goal or condition is already complete, say so clearly and stop; \
         otherwise make the next useful step, preserve normal permissions, and \
         leave evidence in the conversation.\n\
         <prompt>\n{}\n</prompt>\n\
         </loop_trigger>",
        escape_xml(loop_id),
        escape_xml(run_id),
        goal,
        condition,
        escape_xml(prompt)
    )
}

fn condition_satisfied_marker(summary: Option<&str>) -> bool {
    summary
        .map(|s| s.contains("LOOP_CONDITION_SATISFIED:"))
        .unwrap_or(false)
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn emit_loop_event(event: &str, schedule: &LoopSchedule) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            event,
            json!({
                "loopId": schedule.id,
                "sessionId": schedule.session_id,
                "goalId": schedule.goal_id,
                "state": schedule.state,
                "runCount": schedule.run_count,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::CreateGoalInput;
    use crate::session::NewMessage;

    fn temp_dbs() -> (tempfile::TempDir, SessionDB, CronDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_db = SessionDB::open(&dir.path().join("sessions.db")).expect("session db");
        {
            let conn = session_db.conn.lock().expect("lock session db");
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    thread_id TEXT,
                    session_id TEXT NOT NULL,
                    sender_id TEXT,
                    sender_name TEXT,
                    chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    attached_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );",
            )
            .expect("channel conversations table");
        }
        let cron_db = CronDB::open(&dir.path().join("cron.db")).expect("cron db");
        (dir, session_db, cron_db)
    }

    #[test]
    fn trigger_message_escapes_prompt_and_goal() {
        let msg = build_loop_trigger_message(
            "loop<&",
            "run&",
            Some("goal>"),
            LoopTriggerKind::Interval,
            &json!({ "intervalSecs": 60 }),
            "check <CI> & continue",
        );
        assert!(msg.contains("<loop_id>loop&lt;&amp;</loop_id>"));
        assert!(msg.contains("<run_id>run&amp;</run_id>"));
        assert!(msg.contains("<goal_id>goal&gt;</goal_id>"));
        assert!(msg.contains("check &lt;CI&gt; &amp; continue"));
    }

    #[test]
    fn condition_trigger_message_includes_condition_and_stop_marker_contract() {
        let msg = build_loop_trigger_message(
            "loop",
            "run",
            None,
            LoopTriggerKind::Condition,
            &json!({ "condition": "CI <green> & deployed" }),
            "inspect failures",
        );
        assert!(msg.contains("<condition>CI &lt;green&gt; &amp; deployed</condition>"));
        assert!(msg.contains("LOOP_CONDITION_SATISFIED:"));
        assert!(msg.contains("inspect failures"));
    }

    #[test]
    fn interval_trigger_requires_positive_secs() {
        let input = CreateLoopScheduleInput {
            session_id: "s".into(),
            goal_id: None,
            prompt: "poll".into(),
            trigger_kind: LoopTriggerKind::Interval,
            trigger_spec: json!({ "intervalSecs": 0 }),
            execution_strategy: LoopExecutionStrategy::Continue,
            max_runs: None,
            max_runtime_secs: None,
            token_budget: None,
            cost_budget_micros: None,
            agent_id: None,
        };
        assert!(cron_schedule_from_loop(&input).is_err());
    }

    #[test]
    fn create_loop_rejects_cost_budget_until_cost_ledger_exists() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let err = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: Some(1),
                    agent_id: None,
                },
            )
            .expect_err("cost budget is not supported yet");
        assert!(err.to_string().contains("cost ledger"));
    }

    #[test]
    fn token_budget_blocks_next_trigger() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: Some(10),
                    cost_budget_micros: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        let mut msg = NewMessage::assistant("spent budget");
        msg.tokens_in_last = Some(7);
        msg.tokens_out = Some(3);
        session_db
            .append_message(&session.id, &msg)
            .expect("append message");

        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare loop");
        match decision {
            LoopRunDecision::Reject(rejection) => {
                assert!(rejection.reason.contains("token budget exhausted"));
                assert!(rejection.pause_cron_job);
            }
            other => panic!("expected rejection, got {other:?}"),
        }
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load schedule")
            .expect("schedule exists");
        assert_eq!(updated.state, LoopState::Blocked);
    }

    #[test]
    fn rejected_cron_tick_does_not_increment_run_count() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        session_db
            .pause_loop_schedule(&cron_db, &schedule.id)
            .expect("pause loop");
        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare loop");
        assert!(matches!(decision, LoopRunDecision::Reject(_)));

        let finished_at = now_rfc3339();
        session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                None,
                None,
                LoopRunState::Skipped,
                None,
                Some("loop is paused"),
                &finished_at,
            )
            .expect("finish rejected tick");
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load schedule")
            .expect("schedule exists");
        assert_eq!(updated.run_count, 0);
        assert_eq!(updated.state, LoopState::Paused);
    }

    #[test]
    fn workflow_strategy_requires_bound_goal_template() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Keep writing brief fresh".to_string(),
                completion_criteria: "A reviewed draft exists".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let err = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    prompt: "".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Workflow,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    agent_id: None,
                },
            )
            .expect_err("workflow loop without goal template should fail");
        assert!(err.to_string().contains("domain workflow template"));
    }

    #[test]
    fn workflow_strategy_materializes_domain_workflow_run() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let goal = session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Refresh the weekly status memo".to_string(),
                completion_criteria: "Draft is reviewed against stakeholders".to_string(),
                domain: None,
                workflow_template_id: Some("writing-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("weekly_report".to_string()),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: Some(goal.goal.id.clone()),
                    prompt: "Update the memo with the newest evidence".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Workflow,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    agent_id: None,
                },
            )
            .expect("create workflow loop");
        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare loop");
        let admission = match decision {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };
        assert_eq!(
            admission.execution_strategy,
            LoopExecutionStrategy::Workflow
        );

        let launch = session_db
            .create_loop_workflow_run(&admission)
            .expect("create loop workflow run");
        assert_eq!(launch.template_id, "writing-brief");
        assert_eq!(launch.template_version, "1.0.0");
        assert_eq!(launch.workflow_kind, "domain:writing");
        let runs = session_db
            .list_workflow_runs_for_session(&session.id, 10)
            .expect("list workflow runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, launch.run_id);
        assert_eq!(runs[0].goal_id.as_deref(), Some(goal.goal.id.as_str()));
        let expected_origin = format!("loop:{}", schedule.id);
        assert_eq!(runs[0].origin.as_deref(), Some(expected_origin.as_str()));
        let finished_at = now_rfc3339();
        session_db
            .finish_loop_cron_run_with_trace(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("workflow launched"),
                None,
                &finished_at,
                Some(json!({
                    "executionStrategy": "workflow",
                    "workflowRunId": launch.run_id,
                    "templateId": launch.template_id,
                    "templateVersion": launch.template_version,
                })),
            )
            .expect("finish workflow loop run");
        let loop_runs = session_db
            .list_loop_runs(&schedule.id, 10)
            .expect("list loop runs");
        assert_eq!(loop_runs.len(), 1);
        assert_eq!(loop_runs[0].trace["triggerSpec"]["intervalSecs"], json!(60));
        assert_eq!(loop_runs[0].trace["workflowRunId"], json!(runs[0].id));
    }

    #[test]
    fn condition_marker_completes_loop_after_successful_run() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Condition,
                    trigger_spec: json!({
                        "condition": "CI is green",
                        "intervalSecs": 60,
                    }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        let started_at = now_rfc3339();
        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &started_at)
            .expect("prepare loop");
        let run_id = match decision {
            LoopRunDecision::Admit(admission) => admission.run_id,
            other => panic!("expected admission, got {other:?}"),
        };
        let finished_at = now_rfc3339();
        let action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&run_id),
                None,
                LoopRunState::Succeeded,
                Some("LOOP_CONDITION_SATISFIED: CI is green"),
                None,
                &finished_at,
            )
            .expect("finish run");
        assert!(action.pause_cron_job);
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load schedule")
            .expect("schedule exists");
        assert_eq!(updated.state, LoopState::Completed);
        assert_eq!(updated.run_count, 1);
    }
}
