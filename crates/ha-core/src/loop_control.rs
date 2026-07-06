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
use tokio::sync::broadcast::error::RecvError;

use crate::cron::{CronDB, CronPayload, CronSchedule, NewCronJob};
use crate::event_bus::AppEvent;
use crate::goal::GoalState;
use crate::session::{MessageRole, SessionDB};

const LOOP_TRACE_MAX_BYTES: usize = 64 * 1024;
const DEFAULT_UNTIL_INTERVAL_SECS: i64 = 300;
const EVENT_LOOP_IDLE_INTERVAL_SECS: u64 = 366 * 24 * 60 * 60;
const DEFAULT_EVENT_DEBOUNCE_SECS: i64 = 30;
const MAX_EVENT_DEBOUNCE_SECS: i64 = 3600;
const DEFAULT_LOOP_MAX_NO_PROGRESS_RUNS: i64 = 3;
const DEFAULT_LOOP_MAX_FAILURES: i64 = 3;
const DEFAULT_LOOP_BACKOFF_SECS: i64 = 300;
const MAX_LOOP_BACKOFF_SECS: i64 = 24 * 60 * 60;
const STRONG_PROGRESS_RELATIONS: &[&str] = &[
    "workflow_completed",
    "validation_passed",
    "validation_completed",
    "review_passed",
    "domain_quality_passed",
    "task_completed",
    "diff_snapshot",
    "file_changed",
    "artifact_created",
    "artifact_reviewed",
    "source_cited",
    "claim_checked",
    "data_quality_checked",
    "user_decision",
];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopProgressState {
    Progressed,
    WeakProgress,
    NoProgress,
    Blocked,
    Failed,
    AwaitingApproval,
}

impl LoopProgressState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Progressed => "progressed",
            Self::WeakProgress => "weak_progress",
            Self::NoProgress => "no_progress",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::AwaitingApproval => "awaiting_approval",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "progressed" => Some(Self::Progressed),
            "weak_progress" => Some(Self::WeakProgress),
            "no_progress" => Some(Self::NoProgress),
            "blocked" => Some(Self::Blocked),
            "failed" => Some(Self::Failed),
            "awaiting_approval" => Some(Self::AwaitingApproval),
            _ => None,
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_criterion_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_criterion_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_criterion_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_revision: Option<i64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_state: Option<LoopProgressState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_summary: Option<String>,
    pub no_progress_streak: i64,
    pub failure_streak: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_no_progress_runs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_failures: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_status: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_state: Option<LoopProgressState>,
    #[serde(default)]
    pub progress_delta: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_progress_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduling_decision: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_criterion_id: Option<String>,
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
    pub max_no_progress_runs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failures: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLoopSchedulePolicyInput {
    pub loop_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runtime_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_no_progress_runs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failures: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_secs: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct LoopRunAdmission {
    pub loop_id: String,
    pub run_id: String,
    pub session_id: String,
    pub prompt: String,
    pub trigger_kind: LoopTriggerKind,
    pub trigger_spec: Value,
    pub event_context: Option<Value>,
    pub execution_strategy: LoopExecutionStrategy,
    pub agent_id: String,
    pub goal_id: Option<String>,
    pub goal_criterion_id: Option<String>,
    pub goal_criterion_text: Option<String>,
    pub goal_criterion_kind: Option<String>,
    pub goal_revision: Option<i64>,
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
    pub backoff_secs: Option<i64>,
}

#[derive(Debug, Clone)]
struct LoopProgressEvaluation {
    state: LoopProgressState,
    summary: Option<String>,
    delta: Value,
    no_progress_reason: Option<String>,
    scheduling_decision: Option<String>,
}

#[derive(Debug, Clone)]
struct GoalEvidenceDelta {
    total_count: usize,
    strong_count: usize,
    items: Vec<Value>,
}

#[derive(Debug, Clone)]
struct LoopEventMatch {
    fingerprint: String,
    context: Value,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS loop_schedules (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            goal_id TEXT,
            goal_criterion_id TEXT,
            goal_criterion_text TEXT,
            goal_criterion_kind TEXT,
            goal_revision INTEGER,
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
            progress_state TEXT,
            progress_summary TEXT,
            no_progress_streak INTEGER NOT NULL DEFAULT 0,
            failure_streak INTEGER NOT NULL DEFAULT 0,
            max_no_progress_runs INTEGER NOT NULL DEFAULT 3,
            max_failures INTEGER NOT NULL DEFAULT 3,
            backoff_secs INTEGER NOT NULL DEFAULT 300,
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
            progress_state TEXT,
            progress_delta_json TEXT NOT NULL DEFAULT '{}',
            no_progress_reason TEXT,
            scheduling_decision TEXT,
            trace_json TEXT NOT NULL DEFAULT '{}',
            started_at TEXT NOT NULL,
            finished_at TEXT,
            FOREIGN KEY (loop_id) REFERENCES loop_schedules(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS loop_event_ticks (
            id TEXT PRIMARY KEY,
            loop_id TEXT NOT NULL,
            event_name TEXT NOT NULL,
            event_fingerprint TEXT NOT NULL,
            event_payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            consumed_at TEXT,
            loop_run_id TEXT,
            FOREIGN KEY (loop_id) REFERENCES loop_schedules(id) ON DELETE CASCADE,
            FOREIGN KEY (loop_run_id) REFERENCES loop_runs(id) ON DELETE SET NULL,
            UNIQUE(loop_id, event_fingerprint)
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
            ON loop_runs(cron_job_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_loop_event_ticks_loop_pending
            ON loop_event_ticks(loop_id, consumed_at, created_at);",
    )?;
    ensure_loop_column(
        conn,
        "execution_strategy",
        "ALTER TABLE loop_schedules ADD COLUMN execution_strategy TEXT NOT NULL DEFAULT 'continue';",
    )?;
    ensure_loop_column(
        conn,
        "goal_criterion_id",
        "ALTER TABLE loop_schedules ADD COLUMN goal_criterion_id TEXT;",
    )?;
    ensure_loop_column(
        conn,
        "goal_criterion_text",
        "ALTER TABLE loop_schedules ADD COLUMN goal_criterion_text TEXT;",
    )?;
    ensure_loop_column(
        conn,
        "goal_criterion_kind",
        "ALTER TABLE loop_schedules ADD COLUMN goal_criterion_kind TEXT;",
    )?;
    ensure_loop_column(
        conn,
        "goal_revision",
        "ALTER TABLE loop_schedules ADD COLUMN goal_revision INTEGER;",
    )?;
    ensure_loop_column(
        conn,
        "progress_state",
        "ALTER TABLE loop_schedules ADD COLUMN progress_state TEXT;",
    )?;
    ensure_loop_column(
        conn,
        "progress_summary",
        "ALTER TABLE loop_schedules ADD COLUMN progress_summary TEXT;",
    )?;
    ensure_loop_column(
        conn,
        "no_progress_streak",
        "ALTER TABLE loop_schedules ADD COLUMN no_progress_streak INTEGER NOT NULL DEFAULT 0;",
    )?;
    ensure_loop_column(
        conn,
        "failure_streak",
        "ALTER TABLE loop_schedules ADD COLUMN failure_streak INTEGER NOT NULL DEFAULT 0;",
    )?;
    ensure_loop_column(
        conn,
        "max_no_progress_runs",
        "ALTER TABLE loop_schedules ADD COLUMN max_no_progress_runs INTEGER NOT NULL DEFAULT 3;",
    )?;
    ensure_loop_column(
        conn,
        "max_failures",
        "ALTER TABLE loop_schedules ADD COLUMN max_failures INTEGER NOT NULL DEFAULT 3;",
    )?;
    ensure_loop_column(
        conn,
        "backoff_secs",
        "ALTER TABLE loop_schedules ADD COLUMN backoff_secs INTEGER NOT NULL DEFAULT 300;",
    )?;
    ensure_loop_run_column(
        conn,
        "progress_state",
        "ALTER TABLE loop_runs ADD COLUMN progress_state TEXT;",
    )?;
    ensure_loop_run_column(
        conn,
        "progress_delta_json",
        "ALTER TABLE loop_runs ADD COLUMN progress_delta_json TEXT NOT NULL DEFAULT '{}';",
    )?;
    ensure_loop_run_column(
        conn,
        "no_progress_reason",
        "ALTER TABLE loop_runs ADD COLUMN no_progress_reason TEXT;",
    )?;
    ensure_loop_run_column(
        conn,
        "scheduling_decision",
        "ALTER TABLE loop_runs ADD COLUMN scheduling_decision TEXT;",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_loop_schedules_goal_criterion
            ON loop_schedules(goal_id, goal_criterion_id, updated_at DESC);",
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

fn ensure_loop_run_column(conn: &Connection, column: &str, alter_sql: &str) -> Result<()> {
    let query = format!("SELECT {column} FROM loop_runs LIMIT 1");
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
                        "goal criterion binding requires a loop schedule bound to a Goal"
                    ));
                }
                None
            }
        };
        if input.execution_strategy == LoopExecutionStrategy::Workflow {
            if input.trigger_kind != LoopTriggerKind::Interval {
                return Err(anyhow!(
                    "loop workflow execution currently supports interval triggers only; condition and event loops still require conversation continuation"
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
        let max_no_progress_runs = normalize_positive(input.max_no_progress_runs)
            .unwrap_or(DEFAULT_LOOP_MAX_NO_PROGRESS_RUNS);
        let max_failures =
            normalize_positive(input.max_failures).unwrap_or(DEFAULT_LOOP_MAX_FAILURES);
        let backoff_secs =
            normalize_positive(input.backoff_secs).unwrap_or(DEFAULT_LOOP_BACKOFF_SECS);
        let approval_policy_snapshot = json!({
            "permission": "inherits_session",
            "scheduler": "cron",
            "executionStrategy": input.execution_strategy,
            "progressGuard": {
                "maxNoProgressRuns": max_no_progress_runs,
                "maxFailures": max_failures,
                "backoffSecs": backoff_secs,
            },
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
            max_failures: Some(max_failures.max(1) as u32),
            notify_on_complete: Some(false),
            delivery_targets: Some(Vec::new()),
            prefix_delivery_with_name: Some(false),
            job_timeout_secs: input.max_runtime_secs.map(|v| v.max(30) as u64),
            permission_mode_override: None,
            sandbox_mode_override: None,
        })?;
        if input.trigger_kind == LoopTriggerKind::Event {
            if let Err(err) = cron_db.toggle_job(&cron_job.id, false) {
                let _ = cron_db.delete_job(&cron_job.id);
                return Err(err);
            }
        }

        let insert_result = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO loop_schedules (
                    id, session_id, goal_id, goal_criterion_id, goal_criterion_text,
                    goal_criterion_kind, goal_revision, cron_job_id, prompt, trigger_kind, trigger_spec_json, execution_strategy,
                    state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    max_no_progress_runs, max_failures, backoff_secs,
                    approval_policy_snapshot_json, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?22)",
                params![
                    id,
                    input.session_id,
                    goal_id,
                    goal_criterion.as_ref().map(|criterion| criterion.id.as_str()),
                    goal_criterion.as_ref().map(|criterion| criterion.text.as_str()),
                    goal_criterion
                        .as_ref()
                        .map(|criterion| criterion.kind.as_str()),
                    goal_criterion
                        .as_ref()
                        .map(|criterion| criterion.goal_revision),
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
                    max_no_progress_runs,
                    max_failures,
                    backoff_secs.min(MAX_LOOP_BACKOFF_SECS),
                    approval_policy_snapshot_json,
                    now,
                ],
            )
        };
        if let Err(err) = insert_result {
            let _ = cron_db.delete_job(&cron_job.id);
            return Err(err.into());
        }

        let mut schedule = self
            .get_loop_schedule(&id)?
            .ok_or_else(|| anyhow!("loop schedule {} was not persisted", id))?;
        hydrate_loop_schedule_from_cron(cron_db, &mut schedule)?;
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
                    "goalCriterion": loop_schedule_goal_criterion_metadata(&schedule),
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
            "SELECT id, session_id, goal_id, goal_criterion_id, goal_criterion_text,
                    goal_criterion_kind, goal_revision, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    progress_state, progress_summary, no_progress_streak, failure_streak,
                    max_no_progress_runs, max_failures, backoff_secs,
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
            "SELECT id, session_id, goal_id, goal_criterion_id, goal_criterion_text,
                    goal_criterion_kind, goal_revision, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    progress_state, progress_summary, no_progress_streak, failure_streak,
                    max_no_progress_runs, max_failures, backoff_secs,
                    approval_policy_snapshot_json, created_at, updated_at, completed_at, blocked_reason
             FROM loop_schedules
             WHERE session_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_loop_schedule)?;
        collect_rows(rows)
    }

    pub fn list_loop_schedules_for_session_with_cron(
        &self,
        cron_db: &CronDB,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<LoopSchedule>> {
        let mut schedules = self.list_loop_schedules_for_session(session_id, limit)?;
        for schedule in &mut schedules {
            hydrate_loop_schedule_from_cron(cron_db, schedule)?;
        }
        Ok(schedules)
    }

    pub fn loop_snapshot(&self, loop_id: &str, run_limit: usize) -> Result<Option<LoopSnapshot>> {
        let Some(schedule) = self.get_loop_schedule(loop_id)? else {
            return Ok(None);
        };
        let runs = self.list_loop_runs(loop_id, run_limit)?;
        Ok(Some(LoopSnapshot { schedule, runs }))
    }

    pub fn loop_snapshot_with_cron(
        &self,
        cron_db: &CronDB,
        loop_id: &str,
        run_limit: usize,
    ) -> Result<Option<LoopSnapshot>> {
        let Some(mut snapshot) = self.loop_snapshot(loop_id, run_limit)? else {
            return Ok(None);
        };
        hydrate_loop_schedule_from_cron(cron_db, &mut snapshot.schedule)?;
        Ok(Some(snapshot))
    }

    pub fn list_loop_runs(&self, loop_id: &str, limit: usize) -> Result<Vec<LoopRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, loop_id, cron_job_id, cron_run_log_id, session_id, seq, state,
                    trigger_reason, result_summary, error, progress_state, progress_delta_json,
                    no_progress_reason, scheduling_decision, trace_json, started_at, finished_at
             FROM loop_runs
             WHERE loop_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![loop_id, limit as i64], row_to_loop_run)?;
        collect_rows(rows)
    }

    pub fn enqueue_loop_event_triggers(&self, event: &AppEvent) -> Result<Vec<String>> {
        if !is_loop_trigger_event_name(&event.name) {
            return Ok(Vec::new());
        }
        let Some(session_id) = loop_event_session_id(event) else {
            return Ok(Vec::new());
        };
        let schedules = self.list_active_event_loop_schedules_for_session(session_id)?;
        let now = Utc::now();
        let mut cron_job_ids = Vec::new();
        for schedule in schedules {
            let Some(event_match) = loop_event_matches_schedule(&schedule, event, &now)? else {
                continue;
            };
            if self.insert_loop_event_tick(
                &schedule.id,
                &event.name,
                &event_match.fingerprint,
                &event_match.context,
                &now.to_rfc3339(),
            )? {
                cron_job_ids.push(schedule.cron_job_id);
            }
        }
        Ok(cron_job_ids)
    }

    pub fn loop_has_pending_event_ticks(&self, loop_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM loop_event_ticks
                 WHERE loop_id = ?1 AND consumed_at IS NULL
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![loop_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn pause_loop_schedule(&self, cron_db: &CronDB, loop_id: &str) -> Result<LoopSchedule> {
        let mut schedule = self.transition_loop_schedule(loop_id, LoopState::Paused, None)?;
        cron_db.toggle_job(&schedule.cron_job_id, false)?;
        hydrate_loop_schedule_from_cron(cron_db, &mut schedule)?;
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
        let mut schedule = self.transition_loop_schedule(loop_id, LoopState::Active, None)?;
        cron_db.toggle_job(
            &schedule.cron_job_id,
            schedule.trigger_kind != LoopTriggerKind::Event,
        )?;
        hydrate_loop_schedule_from_cron(cron_db, &mut schedule)?;
        Ok(schedule)
    }

    pub fn update_loop_schedule_policy(
        &self,
        cron_db: &CronDB,
        input: UpdateLoopSchedulePolicyInput,
    ) -> Result<LoopSchedule> {
        let current = self
            .get_loop_schedule(&input.loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {}", input.loop_id))?;
        if current.state.is_terminal() {
            return Err(anyhow!(
                "loop schedule {} is {}; terminal loops cannot be edited",
                current.id,
                current.state.as_str()
            ));
        }
        let max_no_progress_runs = normalize_positive(input.max_no_progress_runs)
            .unwrap_or(DEFAULT_LOOP_MAX_NO_PROGRESS_RUNS);
        let max_failures =
            normalize_positive(input.max_failures).unwrap_or(DEFAULT_LOOP_MAX_FAILURES);
        let backoff_secs = normalize_positive(input.backoff_secs)
            .unwrap_or(DEFAULT_LOOP_BACKOFF_SECS)
            .min(MAX_LOOP_BACKOFF_SECS);
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE loop_schedules
                 SET max_runs = ?2,
                     max_runtime_secs = ?3,
                     token_budget = ?4,
                     max_no_progress_runs = ?5,
                     max_failures = ?6,
                     backoff_secs = ?7,
                     blocked_reason = CASE WHEN state = 'blocked' THEN NULL ELSE blocked_reason END,
                     no_progress_streak = CASE WHEN state = 'blocked' THEN 0 ELSE no_progress_streak END,
                     failure_streak = CASE WHEN state = 'blocked' THEN 0 ELSE failure_streak END,
                     updated_at = ?8
                 WHERE id = ?1",
                params![
                    input.loop_id,
                    normalize_positive(input.max_runs),
                    normalize_positive(input.max_runtime_secs),
                    normalize_positive(input.token_budget),
                    max_no_progress_runs,
                    max_failures,
                    backoff_secs,
                    now,
                ],
            )?;
        }
        if let Some(mut job) = cron_db.get_job(&current.cron_job_id)? {
            job.max_failures = max_failures.max(1) as u32;
            job.job_timeout_secs =
                normalize_positive(input.max_runtime_secs).map(|v| v.max(30) as u64);
            cron_db.update_job(&job)?;
        }
        let mut schedule = self
            .get_loop_schedule(&current.id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {}", current.id))?;
        hydrate_loop_schedule_from_cron(cron_db, &mut schedule)?;
        emit_loop_event("loop:changed", &schedule);
        Ok(schedule)
    }

    pub fn stop_loop_schedule(&self, cron_db: &CronDB, loop_id: &str) -> Result<LoopSchedule> {
        let mut schedule = self.transition_loop_schedule(loop_id, LoopState::Cancelled, None)?;
        cron_db.toggle_job(&schedule.cron_job_id, false)?;
        hydrate_loop_schedule_from_cron(cron_db, &mut schedule)?;
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
            let goal = self
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            match goal.state {
                GoalState::Completed => {
                    self.complete_loop_due_to_limit(&schedule, "goal_completed")?;
                    return Ok(LoopRunDecision::Reject(LoopRunRejection {
                        loop_id: Some(schedule.id),
                        reason: "goal already completed".to_string(),
                        pause_cron_job: true,
                    }));
                }
                GoalState::Failed | GoalState::Cancelled => {
                    let reason = format!("goal is {}", goal.state.as_str());
                    self.block_loop_schedule(&schedule, &reason)?;
                    return Ok(LoopRunDecision::Reject(LoopRunRejection {
                        loop_id: Some(schedule.id),
                        reason,
                        pause_cron_job: true,
                    }));
                }
                GoalState::Paused => {
                    let reason = "goal is paused".to_string();
                    self.block_loop_schedule(&schedule, &reason)?;
                    return Ok(LoopRunDecision::Reject(LoopRunRejection {
                        loop_id: Some(schedule.id),
                        reason,
                        pause_cron_job: true,
                    }));
                }
                GoalState::Active | GoalState::Evaluating | GoalState::Blocked => {}
            }
            if let Some(criterion_id) = schedule.goal_criterion_id.as_deref() {
                let current_binding = self
                    .resolve_goal_criterion_binding(goal_id, Some(criterion_id))
                    .map_err(|err| anyhow!("goal criterion needs rebind: {err}"));
                let binding = match current_binding {
                    Ok(Some(binding)) => binding,
                    Ok(None) => {
                        let reason =
                            "goal criterion needs rebind: criterion is missing".to_string();
                        self.block_loop_schedule(&schedule, &reason)?;
                        return Ok(LoopRunDecision::Reject(LoopRunRejection {
                            loop_id: Some(schedule.id),
                            reason,
                            pause_cron_job: true,
                        }));
                    }
                    Err(err) => {
                        let reason = err.to_string();
                        self.block_loop_schedule(&schedule, &reason)?;
                        return Ok(LoopRunDecision::Reject(LoopRunRejection {
                            loop_id: Some(schedule.id),
                            reason,
                            pause_cron_job: true,
                        }));
                    }
                };
                let stale = schedule.goal_revision != Some(binding.goal_revision)
                    || schedule.goal_criterion_text.as_deref() != Some(binding.text.as_str())
                    || schedule.goal_criterion_kind.as_deref() != Some(binding.kind.as_str());
                if stale {
                    let reason = format!(
                        "goal criterion needs rebind: schedule revision {:?}, current revision {}",
                        schedule.goal_revision, binding.goal_revision
                    );
                    self.block_loop_schedule(&schedule, &reason)?;
                    return Ok(LoopRunDecision::Reject(LoopRunRejection {
                        loop_id: Some(schedule.id),
                        reason,
                        pause_cron_job: true,
                    }));
                }
            }
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
        let mut trigger_reason = format!(
            "{} trigger from cron job {}",
            schedule.trigger_kind.as_str(),
            cron_job_id
        );
        let trace = json!({
            "triggerKind": schedule.trigger_kind,
            "triggerSpec": schedule.trigger_spec,
            "eventContext": null,
            "executionStrategy": schedule.execution_strategy,
            "cronJobId": cron_job_id,
            "seq": seq,
            "goalCriterion": loop_schedule_goal_criterion_metadata(&schedule),
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
        let event_context = if schedule.trigger_kind == LoopTriggerKind::Event {
            self.claim_next_loop_event_tick(&schedule.id, &run_id)?
        } else {
            None
        };
        if let Some(context) = event_context.as_ref() {
            if let Some(event_name) = context.get("eventName").and_then(Value::as_str) {
                trigger_reason = format!("event trigger {event_name} from cron job {cron_job_id}");
            }
            self.patch_loop_run_trace(
                &run_id,
                json!({ "eventContext": context }),
                Some(&trigger_reason),
            )?;
        }
        Ok(LoopRunDecision::Admit(LoopRunAdmission {
            loop_id: schedule.id,
            run_id,
            session_id: session_id.to_string(),
            prompt: schedule.prompt,
            trigger_kind: schedule.trigger_kind,
            trigger_spec: schedule.trigger_spec,
            event_context,
            execution_strategy: schedule.execution_strategy,
            agent_id: self
                .get_session(session_id)?
                .map(|m| m.agent_id)
                .unwrap_or_else(|| "ha-main".to_string()),
            goal_id: schedule.goal_id,
            goal_criterion_id: schedule.goal_criterion_id,
            goal_criterion_text: schedule.goal_criterion_text,
            goal_criterion_kind: schedule.goal_criterion_kind,
            goal_revision: schedule.goal_revision,
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
                backoff_secs: None,
            });
        };
        let run_id = match loop_run_id {
            Some(id) => Some(id.to_string()),
            None => self.latest_running_loop_run_id(&schedule.id)?,
        };
        let started_at = match run_id.as_deref() {
            Some(run_id) => self
                .loop_run_started_at(run_id)?
                .unwrap_or_else(|| schedule.updated_at.clone()),
            None => schedule.updated_at.clone(),
        };
        let progress = self.evaluate_loop_progress(
            &schedule,
            run_id.as_deref(),
            &started_at,
            state,
            result_summary,
            error,
            extra_trace.as_ref(),
        )?;
        if let Some(run_id) = run_id.as_deref() {
            let mut trace_patch = json!({
                "cronRunLogId": cron_run_log_id,
                "finishedAt": finished_at,
            });
            if let Some(extra) = extra_trace.as_ref() {
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
                Some(progress.state),
                progress.delta.clone(),
                progress.no_progress_reason.as_deref(),
                progress.scheduling_decision.as_deref(),
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
        let mut next_no_progress_streak = schedule.no_progress_streak;
        let mut next_failure_streak = schedule.failure_streak;
        let mut backoff_secs = None;
        let mut scheduling_decision = progress.scheduling_decision.clone();
        if schedule.state == LoopState::Active && counted_run {
            if state == LoopRunState::Succeeded
                && schedule.trigger_kind == LoopTriggerKind::Condition
                && condition_satisfied_marker(result_summary)
            {
                next_state = LoopState::Completed;
                pause = true;
                scheduling_decision = Some("completed_condition_satisfied".to_string());
            }
            if let Some(max_runs) = schedule.max_runs {
                if next_count >= max_runs {
                    next_state = LoopState::Completed;
                    pause = true;
                    scheduling_decision = Some("completed_max_runs".to_string());
                }
            }
            if let Some(max_runtime) = schedule.max_runtime_secs {
                if loop_elapsed_secs(&schedule.created_at)? >= max_runtime {
                    next_state = LoopState::Completed;
                    pause = true;
                    scheduling_decision = Some("completed_max_runtime".to_string());
                }
            }
            if !next_state.is_terminal() {
                match progress.state {
                    LoopProgressState::Progressed | LoopProgressState::WeakProgress => {
                        next_no_progress_streak = 0;
                        next_failure_streak = 0;
                        scheduling_decision.get_or_insert_with(|| "continue".to_string());
                    }
                    LoopProgressState::AwaitingApproval => {
                        next_failure_streak = 0;
                        scheduling_decision
                            .get_or_insert_with(|| "awaiting_follow_up_turn".to_string());
                    }
                    LoopProgressState::NoProgress => {
                        next_no_progress_streak += 1;
                        next_failure_streak = 0;
                        let max_no_progress = schedule
                            .max_no_progress_runs
                            .unwrap_or(DEFAULT_LOOP_MAX_NO_PROGRESS_RUNS);
                        if max_no_progress > 0 && next_no_progress_streak >= max_no_progress {
                            next_state = LoopState::Blocked;
                            pause = true;
                            blocked_reason =
                                Some(progress.no_progress_reason.clone().unwrap_or_else(|| {
                                    "loop made no durable progress".to_string()
                                }));
                            scheduling_decision = Some("blocked_no_progress_limit".to_string());
                        } else if let Some(delay) =
                            loop_backoff_delay(schedule.backoff_secs, next_no_progress_streak)
                        {
                            backoff_secs = Some(delay);
                            scheduling_decision = Some(format!("backoff_{delay}s"));
                        } else {
                            scheduling_decision = Some("continue".to_string());
                        }
                    }
                    LoopProgressState::Failed => {
                        next_failure_streak += 1;
                        let max_failures =
                            schedule.max_failures.unwrap_or(DEFAULT_LOOP_MAX_FAILURES);
                        if max_failures > 0 && next_failure_streak >= max_failures {
                            next_state = LoopState::Blocked;
                            pause = true;
                            blocked_reason = Some(
                                error
                                    .map(str::to_string)
                                    .unwrap_or_else(|| "loop failed repeatedly".to_string()),
                            );
                            scheduling_decision = Some("blocked_failure_limit".to_string());
                        } else if let Some(delay) =
                            loop_backoff_delay(schedule.backoff_secs, next_failure_streak)
                        {
                            backoff_secs = Some(delay);
                            scheduling_decision = Some(format!("backoff_{delay}s"));
                        } else {
                            scheduling_decision = Some("continue".to_string());
                        }
                    }
                    LoopProgressState::Blocked => {
                        next_state = LoopState::Blocked;
                        pause = true;
                        blocked_reason = Some(
                            error
                                .map(str::to_string)
                                .or_else(|| progress.no_progress_reason.clone())
                                .unwrap_or_else(|| "loop is blocked".to_string()),
                        );
                        scheduling_decision = Some("blocked".to_string());
                    }
                }
            }
        }
        if let Some(run_id) = run_id.as_deref() {
            if scheduling_decision.as_deref() != progress.scheduling_decision.as_deref() {
                self.update_loop_run_scheduling_decision(run_id, scheduling_decision.as_deref())?;
            }
        }
        self.bump_loop_after_run(
            &schedule.id,
            next_count,
            next_state,
            blocked_reason,
            Some(progress.state),
            progress.summary,
            next_no_progress_streak,
            next_failure_streak,
        )?;
        if let Some(updated) = self.get_loop_schedule(&schedule.id)? {
            emit_loop_event("loop:changed", &updated);
        }
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
                    "goalCriterion": loop_schedule_goal_criterion_metadata(&schedule),
                }),
            );
        }
        Ok(LoopAfterRunAction {
            loop_id: Some(schedule.id),
            pause_cron_job: pause || next_state.is_terminal(),
            backoff_secs,
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
            goal_criterion_id: admission.goal_criterion_id.clone(),
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
                "goalCriterionId": admission.goal_criterion_id.as_deref(),
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
            "SELECT id, session_id, goal_id, goal_criterion_id, goal_criterion_text,
                    goal_criterion_kind, goal_revision, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    progress_state, progress_summary, no_progress_streak, failure_streak,
                    max_no_progress_runs, max_failures, backoff_secs,
                    approval_policy_snapshot_json, created_at, updated_at, completed_at, blocked_reason
             FROM loop_schedules WHERE cron_job_id = ?1",
                params![cron_job_id],
                row_to_loop_schedule,
            )
            .optional()?)
    }

    fn list_active_event_loop_schedules_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<LoopSchedule>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, goal_id, goal_criterion_id, goal_criterion_text,
                    goal_criterion_kind, goal_revision, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, max_runs, run_count, max_runtime_secs, token_budget, cost_budget_micros,
                    progress_state, progress_summary, no_progress_streak, failure_streak,
                    max_no_progress_runs, max_failures, backoff_secs,
                    approval_policy_snapshot_json, created_at, updated_at, completed_at, blocked_reason
             FROM loop_schedules
             WHERE session_id = ?1 AND trigger_kind = 'event' AND state = 'active'
             ORDER BY updated_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_loop_schedule)?;
        collect_rows(rows)
    }

    fn insert_loop_event_tick(
        &self,
        loop_id: &str,
        event_name: &str,
        event_fingerprint: &str,
        event_context: &Value,
        created_at: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let rows = conn.execute(
            "INSERT OR IGNORE INTO loop_event_ticks (
                id, loop_id, event_name, event_fingerprint, event_payload_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                format!("letick_{}", uuid::Uuid::new_v4().simple()),
                loop_id,
                event_name,
                event_fingerprint,
                bounded_json(event_context)?,
                created_at,
            ],
        )?;
        Ok(rows > 0)
    }

    fn claim_next_loop_event_tick(
        &self,
        loop_id: &str,
        loop_run_id: &str,
    ) -> Result<Option<Value>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tick = conn
            .query_row(
                "SELECT id, event_name, event_fingerprint, event_payload_json, created_at
                 FROM loop_event_ticks
                 WHERE loop_id = ?1 AND consumed_at IS NULL
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![loop_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, event_name, event_fingerprint, payload_json, created_at)) = tick else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE loop_event_ticks
             SET consumed_at = ?2, loop_run_id = ?3
             WHERE id = ?1 AND consumed_at IS NULL",
            params![id, now, loop_run_id],
        )?;
        let payload: Value = serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({}));
        Ok(Some(json!({
            "eventName": event_name,
            "eventFingerprint": event_fingerprint,
            "createdAt": created_at,
            "payload": payload,
        })))
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
                     blocked_reason = ?5,
                     no_progress_streak = CASE WHEN ?2 = 'active' THEN 0 ELSE no_progress_streak END,
                     failure_streak = CASE WHEN ?2 = 'active' THEN 0 ELSE failure_streak END
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
        let completed_skip = matches!(
            reason,
            "goal_completed" | "max_runs_reached" | "max_runtime_reached"
        );
        let progress_state = if completed_skip {
            LoopProgressState::Progressed
        } else {
            LoopProgressState::Blocked
        };
        let scheduling_decision = if completed_skip {
            "completed"
        } else {
            "blocked"
        };
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO loop_runs (
                id, loop_id, cron_job_id, session_id, seq, state, trigger_reason,
                error, progress_state, progress_delta_json, no_progress_reason,
                scheduling_decision, trace_json, started_at, finished_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
            params![
                run_id,
                schedule.id,
                schedule.cron_job_id,
                schedule.session_id,
                seq,
                LoopRunState::Skipped.as_str(),
                reason,
                reason,
                progress_state.as_str(),
                stable_json(&json!({ "reason": reason }))?,
                if completed_skip { None } else { Some(reason) },
                scheduling_decision,
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
        progress_state: Option<LoopProgressState>,
        progress_delta: Value,
        no_progress_reason: Option<&str>,
        scheduling_decision: Option<&str>,
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
                 progress_state = ?6,
                 progress_delta_json = ?7,
                 no_progress_reason = ?8,
                 scheduling_decision = ?9,
                 trace_json = ?10,
                 finished_at = ?11
             WHERE id = ?1",
            params![
                run_id,
                state.as_str(),
                cron_run_log_id,
                result_summary,
                error,
                progress_state.map(|state| state.as_str()),
                bounded_json(&progress_delta)?,
                no_progress_reason,
                scheduling_decision,
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
        progress_state: Option<LoopProgressState>,
        progress_summary: Option<String>,
        no_progress_streak: i64,
        failure_streak: i64,
    ) -> Result<()> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_schedules
             SET run_count = ?2,
                 state = ?3,
                 blocked_reason = COALESCE(?4, blocked_reason),
                 progress_state = ?5,
                 progress_summary = ?6,
                 no_progress_streak = ?7,
                 failure_streak = ?8,
                 completed_at = CASE WHEN ?9 != 0 THEN COALESCE(completed_at, ?10) ELSE completed_at END,
                 updated_at = ?10
             WHERE id = ?1",
            params![
                loop_id,
                run_count,
                state.as_str(),
                blocked_reason,
                progress_state.map(|state| state.as_str()),
                progress_summary,
                no_progress_streak.max(0),
                failure_streak.max(0),
                if state.is_terminal() { 1i64 } else { 0i64 },
                now,
            ],
        )?;
        Ok(())
    }

    fn update_loop_run_scheduling_decision(
        &self,
        run_id: &str,
        scheduling_decision: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_runs SET scheduling_decision = ?2 WHERE id = ?1",
            params![run_id, scheduling_decision],
        )?;
        Ok(())
    }

    fn patch_loop_run_trace(
        &self,
        run_id: &str,
        trace_patch: Value,
        trigger_reason: Option<&str>,
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
        conn.execute(
            "UPDATE loop_runs
             SET trigger_reason = COALESCE(?2, trigger_reason),
                 trace_json = ?3
             WHERE id = ?1",
            params![run_id, trigger_reason, bounded_json(&trace)?],
        )?;
        Ok(())
    }

    fn loop_run_started_at(&self, run_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
                "SELECT started_at FROM loop_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn evaluate_loop_progress(
        &self,
        schedule: &LoopSchedule,
        run_id: Option<&str>,
        started_at: &str,
        run_state: LoopRunState,
        result_summary: Option<&str>,
        error: Option<&str>,
        extra_trace: Option<&Value>,
    ) -> Result<LoopProgressEvaluation> {
        let mut delta = json!({
            "runState": run_state,
            "startedAt": started_at,
        });
        if let Some(trace) = extra_trace {
            if let Some(workflow_run_id) = json_string(trace, "workflowRunId") {
                delta["workflowRunId"] = json!(workflow_run_id);
            }
        }

        let state_from_run = match run_state {
            LoopRunState::Failed | LoopRunState::Cancelled => Some(LoopProgressState::Failed),
            LoopRunState::Skipped => Some(LoopProgressState::Blocked),
            LoopRunState::Queued | LoopRunState::Injected | LoopRunState::Running => {
                Some(LoopProgressState::AwaitingApproval)
            }
            LoopRunState::Empty => Some(LoopProgressState::NoProgress),
            LoopRunState::Succeeded => None,
        };
        if let Some(state) = state_from_run {
            let no_progress_reason = match state {
                LoopProgressState::NoProgress => Some("loop run produced no output".to_string()),
                LoopProgressState::Blocked => error.map(str::to_string),
                _ => None,
            };
            return Ok(LoopProgressEvaluation {
                state,
                summary: progress_summary_for_state(state, result_summary, error),
                delta,
                no_progress_reason,
                scheduling_decision: match state {
                    LoopProgressState::AwaitingApproval => {
                        Some("awaiting_follow_up_turn".to_string())
                    }
                    _ => None,
                },
            });
        }

        if schedule.trigger_kind == LoopTriggerKind::Condition
            && condition_satisfied_marker(result_summary)
        {
            delta["conditionSatisfied"] = json!(true);
            return Ok(LoopProgressEvaluation {
                state: LoopProgressState::Progressed,
                summary: Some(
                    result_summary
                        .map(|summary| truncate_utf8(summary, 240).to_string())
                        .unwrap_or_else(|| "condition satisfied".to_string()),
                ),
                delta,
                no_progress_reason: None,
                scheduling_decision: Some("completed_condition_satisfied".to_string()),
            });
        }

        let workflow_trace_id = extra_trace.and_then(|trace| json_string(trace, "workflowRunId"));
        if let Some(goal_id) = schedule.goal_id.as_deref() {
            let evidence = self.goal_evidence_delta_since(goal_id, started_at, run_id)?;
            delta["goalEvidence"] = json!({
                "total": evidence.total_count,
                "strong": evidence.strong_count,
                "items": evidence.items,
            });
            if evidence.strong_count > 0 {
                return Ok(LoopProgressEvaluation {
                    state: LoopProgressState::Progressed,
                    summary: Some(format!(
                        "recorded {} durable Goal evidence item(s)",
                        evidence.strong_count
                    )),
                    delta,
                    no_progress_reason: None,
                    scheduling_decision: Some("continue".to_string()),
                });
            }
            if evidence.total_count > 0 || workflow_trace_id.is_some() {
                return Ok(LoopProgressEvaluation {
                    state: LoopProgressState::WeakProgress,
                    summary: Some(if evidence.total_count > 0 {
                        format!(
                            "recorded {} weak Goal evidence item(s)",
                            evidence.total_count
                        )
                    } else {
                        "created a derived workflow run".to_string()
                    }),
                    delta,
                    no_progress_reason: None,
                    scheduling_decision: Some("continue".to_string()),
                });
            }
            return Ok(LoopProgressEvaluation {
                state: LoopProgressState::NoProgress,
                summary: result_summary.map(|summary| truncate_utf8(summary, 240).to_string()),
                delta,
                no_progress_reason: Some(
                    "no new durable Goal evidence was recorded during this loop run".to_string(),
                ),
                scheduling_decision: None,
            });
        }

        if workflow_trace_id.is_some()
            || result_summary
                .map(|summary| !summary.trim().is_empty())
                .unwrap_or(false)
        {
            Ok(LoopProgressEvaluation {
                state: LoopProgressState::WeakProgress,
                summary: Some(
                    result_summary
                        .map(|summary| truncate_utf8(summary, 240).to_string())
                        .unwrap_or_else(|| "loop run produced output".to_string()),
                ),
                delta,
                no_progress_reason: None,
                scheduling_decision: Some("continue".to_string()),
            })
        } else {
            Ok(LoopProgressEvaluation {
                state: LoopProgressState::NoProgress,
                summary: None,
                delta,
                no_progress_reason: Some("loop run produced no durable signal".to_string()),
                scheduling_decision: None,
            })
        }
    }

    fn goal_evidence_delta_since(
        &self,
        goal_id: &str,
        started_at: &str,
        loop_run_id: Option<&str>,
    ) -> Result<GoalEvidenceDelta> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT target_type, target_id, relation, metadata_json, created_at
             FROM goal_links
             WHERE goal_id = ?1 AND created_at >= ?2
             ORDER BY created_at ASC, id ASC
             LIMIT 50",
        )?;
        let mut rows = stmt.query(params![goal_id, started_at])?;
        let mut total_count = 0usize;
        let mut strong_count = 0usize;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            let target_type: String = row.get(0)?;
            let target_id: String = row.get(1)?;
            let relation: String = row.get(2)?;
            let metadata_json: String = row.get(3)?;
            let created_at: String = row.get(4)?;
            if target_type == "loop_run" && loop_run_id == Some(target_id.as_str()) {
                continue;
            }
            if relation == "loop_triggered" {
                continue;
            }
            total_count += 1;
            let strong = is_strong_progress_relation(&relation);
            if strong {
                strong_count += 1;
            }
            items.push(json!({
                "targetType": target_type,
                "targetId": target_id,
                "relation": relation,
                "createdAt": created_at,
                "strong": strong,
                "metadata": serde_json::from_str::<Value>(&metadata_json).unwrap_or_else(|_| json!({})),
            }));
        }
        Ok(GoalEvidenceDelta {
            total_count,
            strong_count,
            items,
        })
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
        LoopTriggerKind::Event => Ok(CronSchedule::Every {
            interval_ms: EVENT_LOOP_IDLE_INTERVAL_SECS.saturating_mul(1000),
            start_at: None,
        }),
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
        LoopTriggerKind::Event => {
            let event_name = spec
                .get("eventName")
                .or_else(|| spec.get("event_name"))
                .or_else(|| spec.get("event"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("event loop requires triggerSpec.eventName"))?;
            if !is_supported_loop_event_name(event_name) {
                return Err(anyhow!("unsupported loop event trigger: {event_name}"));
            }
            let filters = spec.get("filters").cloned().unwrap_or_else(|| json!({}));
            if !filters.is_object() {
                return Err(anyhow!("event loop triggerSpec.filters must be an object"));
            }
            validate_loop_event_filters(event_name, &filters)?;
            let debounce_secs = spec
                .get("debounceSecs")
                .or_else(|| spec.get("debounce_secs"))
                .and_then(Value::as_i64)
                .unwrap_or(DEFAULT_EVENT_DEBOUNCE_SECS)
                .clamp(1, MAX_EVENT_DEBOUNCE_SECS);
            Ok(json!({
                "eventName": event_name,
                "filters": filters,
                "debounceSecs": debounce_secs,
            }))
        }
        LoopTriggerKind::Cron => Ok(spec.clone()),
    }
}

fn row_to_loop_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<LoopSchedule> {
    let trigger_kind: String = row.get(9)?;
    let trigger_spec_json: String = row.get(10)?;
    let execution_strategy: String = row.get(11)?;
    let state: String = row.get(12)?;
    let progress_state: Option<String> = row.get(18)?;
    let policy_json: String = row.get(25)?;
    Ok(LoopSchedule {
        id: row.get(0)?,
        session_id: row.get(1)?,
        goal_id: row.get(2)?,
        goal_criterion_id: row.get(3)?,
        goal_criterion_text: row.get(4)?,
        goal_criterion_kind: row.get(5)?,
        goal_revision: row.get(6)?,
        cron_job_id: row.get(7)?,
        prompt: row.get(8)?,
        trigger_kind: LoopTriggerKind::from_str(&trigger_kind).unwrap_or(LoopTriggerKind::Interval),
        trigger_spec: serde_json::from_str(&trigger_spec_json).unwrap_or_else(|_| json!({})),
        execution_strategy: LoopExecutionStrategy::from_str(&execution_strategy)
            .unwrap_or(LoopExecutionStrategy::Continue),
        state: LoopState::from_str(&state).unwrap_or(LoopState::Blocked),
        max_runs: row.get(13)?,
        run_count: row.get(14)?,
        max_runtime_secs: row.get(15)?,
        token_budget: row.get(16)?,
        cost_budget_micros: row.get(17)?,
        progress_state: progress_state
            .as_deref()
            .and_then(LoopProgressState::from_str),
        progress_summary: row.get(19)?,
        no_progress_streak: row.get(20)?,
        failure_streak: row.get(21)?,
        max_no_progress_runs: row.get(22)?,
        max_failures: row.get(23)?,
        backoff_secs: row.get(24)?,
        next_run_at: None,
        cron_status: None,
        approval_policy_snapshot: serde_json::from_str(&policy_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(26)?,
        updated_at: row.get(27)?,
        completed_at: row.get(28)?,
        blocked_reason: row.get(29)?,
    })
}

fn loop_schedule_goal_criterion_metadata(schedule: &LoopSchedule) -> Option<Value> {
    let id = schedule.goal_criterion_id.as_deref()?;
    Some(json!({
        "id": id,
        "text": schedule.goal_criterion_text.as_deref(),
        "kind": schedule.goal_criterion_kind.as_deref(),
        "goalRevision": schedule.goal_revision,
    }))
}

fn row_to_loop_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<LoopRun> {
    let state: String = row.get(6)?;
    let progress_state: Option<String> = row.get(10)?;
    let progress_delta_json: String = row.get(11)?;
    let trace_json: String = row.get(14)?;
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
        progress_state: progress_state
            .as_deref()
            .and_then(LoopProgressState::from_str),
        progress_delta: serde_json::from_str(&progress_delta_json).unwrap_or_else(|_| json!({})),
        no_progress_reason: row.get(12)?,
        scheduling_decision: row.get(13)?,
        trace: serde_json::from_str(&trace_json).unwrap_or_else(|_| json!({})),
        started_at: row.get(15)?,
        finished_at: row.get(16)?,
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

fn loop_backoff_delay(base: Option<i64>, streak: i64) -> Option<i64> {
    let base = base.unwrap_or(DEFAULT_LOOP_BACKOFF_SECS);
    if base <= 0 || streak <= 0 {
        return None;
    }
    Some(base.saturating_mul(streak).min(MAX_LOOP_BACKOFF_SECS))
}

fn hydrate_loop_schedule_from_cron(cron_db: &CronDB, schedule: &mut LoopSchedule) -> Result<()> {
    if let Some(job) = cron_db.get_job(&schedule.cron_job_id)? {
        schedule.next_run_at = if schedule.trigger_kind == LoopTriggerKind::Event {
            None
        } else {
            job.next_run_at
        };
        schedule.cron_status = Some(job.status.as_str().to_string());
        if schedule.trigger_kind == LoopTriggerKind::Event && schedule.state == LoopState::Active {
            schedule.cron_status = Some("event".to_string());
        }
    }
    Ok(())
}

fn is_strong_progress_relation(relation: &str) -> bool {
    STRONG_PROGRESS_RELATIONS.contains(&relation)
}

fn progress_summary_for_state(
    state: LoopProgressState,
    result_summary: Option<&str>,
    error: Option<&str>,
) -> Option<String> {
    match state {
        LoopProgressState::Failed | LoopProgressState::Blocked => error
            .or(result_summary)
            .map(|summary| truncate_utf8(summary, 240).to_string()),
        LoopProgressState::AwaitingApproval => {
            Some("loop trigger is queued for a follow-up turn".to_string())
        }
        LoopProgressState::NoProgress
        | LoopProgressState::Progressed
        | LoopProgressState::WeakProgress => {
            result_summary.map(|summary| truncate_utf8(summary, 240).to_string())
        }
    }
}

fn json_string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_supported_loop_event_name(event_name: &str) -> bool {
    matches!(
        event_name,
        "workflow:updated" | "goal:updated" | "task_updated"
    )
}

fn is_loop_trigger_event_name(event_name: &str) -> bool {
    is_supported_loop_event_name(event_name)
}

fn validate_loop_event_filters(event_name: &str, filters: &Value) -> Result<()> {
    let allowed = match event_name {
        "workflow:updated" => &["workflowState"][..],
        "goal:updated" => &["goalState"][..],
        "task_updated" => &["taskStatus"][..],
        _ => return Err(anyhow!("unsupported loop event trigger: {event_name}")),
    };
    let Some(object) = filters.as_object() else {
        return Err(anyhow!("event loop triggerSpec.filters must be an object"));
    };
    for (key, value) in object {
        if !allowed.contains(&key.as_str()) {
            return Err(anyhow!(
                "unsupported filter {key} for event loop trigger {event_name}"
            ));
        }
        if !value.is_string() {
            return Err(anyhow!("event loop filter {key} must be a string"));
        }
    }
    Ok(())
}

fn loop_event_session_id(event: &AppEvent) -> Option<&str> {
    event
        .payload
        .get("sessionId")
        .or_else(|| event.payload.get("session_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn loop_event_matches_schedule(
    schedule: &LoopSchedule,
    event: &AppEvent,
    now: &DateTime<Utc>,
) -> Result<Option<LoopEventMatch>> {
    let event_name = json_string(&schedule.trigger_spec, "eventName")
        .or_else(|| json_string(&schedule.trigger_spec, "event_name"))
        .or_else(|| json_string(&schedule.trigger_spec, "event"));
    if event_name != Some(event.name.as_str()) {
        return Ok(None);
    }
    let filters = schedule
        .trigger_spec
        .get("filters")
        .cloned()
        .unwrap_or_else(|| json!({}));
    validate_loop_event_filters(&event.name, &filters)?;
    let Some(session_id) = loop_event_session_id(event) else {
        return Ok(None);
    };
    if session_id != schedule.session_id {
        return Ok(None);
    }
    let identity = match event.name.as_str() {
        "workflow:updated" => {
            let state = json_string(&event.payload, "state").unwrap_or("");
            if !loop_filter_matches(&filters, "workflowState", state) {
                return Ok(None);
            }
            json!({
                "id": json_string(&event.payload, "id"),
                "state": state,
                "kind": json_string(&event.payload, "kind"),
                "goalId": json_string(&event.payload, "goalId"),
            })
        }
        "goal:updated" => {
            let state = json_string(&event.payload, "state").unwrap_or("");
            if !loop_filter_matches(&filters, "goalState", state) {
                return Ok(None);
            }
            json!({
                "id": json_string(&event.payload, "id"),
                "state": state,
                "revision": event.payload.get("revision").and_then(Value::as_i64),
            })
        }
        "task_updated" => {
            let task_status = filters
                .get("taskStatus")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let tasks = event
                .payload
                .get("tasks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let matching: Vec<Value> = tasks
                .into_iter()
                .filter(|task| {
                    let status = json_string(task, "status").unwrap_or("");
                    task_status
                        .map(|expected| expected == status)
                        .unwrap_or(true)
                })
                .map(|task| {
                    json!({
                        "id": task.get("id").and_then(Value::as_i64),
                        "status": json_string(&task, "status"),
                        "content": json_string(&task, "content"),
                    })
                })
                .collect();
            if matching.is_empty() {
                return Ok(None);
            }
            json!({ "tasks": matching })
        }
        _ => return Ok(None),
    };
    let debounce_secs = schedule
        .trigger_spec
        .get("debounceSecs")
        .or_else(|| schedule.trigger_spec.get("debounce_secs"))
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_EVENT_DEBOUNCE_SECS)
        .clamp(1, MAX_EVENT_DEBOUNCE_SECS);
    let bucket = now.timestamp().div_euclid(debounce_secs);
    let fingerprint_input = stable_json(&json!({
        "loopId": schedule.id,
        "eventName": event.name,
        "identity": identity,
        "bucket": bucket,
    }))?;
    let fingerprint = blake3::hash(fingerprint_input.as_bytes())
        .to_hex()
        .to_string();
    Ok(Some(LoopEventMatch {
        fingerprint,
        context: json!({
            "eventName": event.name,
            "sessionId": session_id,
            "matched": identity,
            "filters": filters,
            "debounceSecs": debounce_secs,
            "payload": event.payload,
        }),
    }))
}

fn loop_filter_matches(filters: &Value, key: &str, actual: &str) -> bool {
    filters
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|expected| expected == actual)
        .unwrap_or(true)
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
    let event_context = admission
        .event_context
        .as_ref()
        .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "none".to_string());
    format!(
        "Loop trigger context:\n- loop_id: {}\n- loop_run_id: {}\n- trigger_kind: {}\n- trigger_spec: {}\n- event_context: {}\n- recurring_prompt: {}\n",
        admission.loop_id,
        admission.run_id,
        admission.trigger_kind.as_str(),
        trigger_spec,
        event_context,
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
    goal_criterion_id: Option<&str>,
    goal_criterion_text: Option<&str>,
    trigger_kind: LoopTriggerKind,
    trigger_spec: &Value,
    event_context: Option<&Value>,
    prompt: &str,
) -> String {
    let goal = goal_id
        .map(|id| format!("<goal_id>{}</goal_id>\n", escape_xml(id)))
        .unwrap_or_default();
    let goal_criterion = goal_criterion_id
        .map(|id| {
            let text = goal_criterion_text
                .map(|text| {
                    format!(
                        "\n<goal_criterion_text>{}</goal_criterion_text>",
                        escape_xml(text)
                    )
                })
                .unwrap_or_default();
            format!(
                "<goal_criterion_id>{}</goal_criterion_id>{}\n",
                escape_xml(id),
                text
            )
        })
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
    let event = event_context
        .map(|context| {
            let serialized = serde_json::to_string(context).unwrap_or_else(|_| "{}".to_string());
            format!(
                "<event_context>{}</event_context>\n",
                escape_xml(&serialized)
            )
        })
        .unwrap_or_default();
    format!(
        "<loop_trigger>\n\
         <loop_id>{}</loop_id>\n\
         <run_id>{}</run_id>\n\
         {}\
         {}\
         {}\
         {}\
         A recurring loop trigger has fired. Follow the recurring prompt below. \
         If the goal or condition is already complete, say so clearly and stop; \
         otherwise make the next useful step, preserve normal permissions, and \
         leave evidence in the conversation.\n\
         <prompt>\n{}\n</prompt>\n\
         </loop_trigger>",
        escape_xml(loop_id),
        escape_xml(run_id),
        goal,
        goal_criterion,
        condition,
        event,
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

pub fn spawn_loop_event_trigger_watcher() {
    if !crate::runtime_lock::is_primary() {
        return;
    }
    let Some(bus) = crate::get_event_bus().cloned() else {
        app_warn!(
            "loop",
            "event_trigger",
            "EventBus not initialized — event-triggered loops disabled"
        );
        return;
    };
    let Some(session_db) = crate::get_session_db().cloned() else {
        app_warn!(
            "loop",
            "event_trigger",
            "SessionDB not initialized — event-triggered loops disabled"
        );
        return;
    };
    let Some(cron_db) = crate::get_cron_db().cloned() else {
        app_warn!(
            "loop",
            "event_trigger",
            "CronDB not initialized — event-triggered loops disabled"
        );
        return;
    };
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let cron_job_ids = match session_db.enqueue_loop_event_triggers(&event) {
                        Ok(ids) => ids,
                        Err(err) => {
                            app_warn!(
                                "loop",
                                "event_trigger",
                                "Failed to enqueue loop event trigger {}: {}",
                                event.name,
                                err
                            );
                            continue;
                        }
                    };
                    for cron_job_id in cron_job_ids {
                        let job = match cron_db.get_job(&cron_job_id) {
                            Ok(Some(job)) => job,
                            Ok(None) => {
                                app_warn!(
                                    "loop",
                                    "event_trigger",
                                    "Loop event trigger references missing cron job {}",
                                    cron_job_id
                                );
                                continue;
                            }
                            Err(err) => {
                                app_warn!(
                                    "loop",
                                    "event_trigger",
                                    "Failed to load loop cron job {}: {}",
                                    cron_job_id,
                                    err
                                );
                                continue;
                            }
                        };
                        let cron_db = cron_db.clone();
                        let session_db = session_db.clone();
                        tokio::spawn(async move {
                            crate::cron::execute_job_public(&cron_db, &session_db, &job).await;
                        });
                    }
                }
                Err(RecvError::Lagged(count)) => {
                    app_warn!(
                        "loop",
                        "event_trigger",
                        "Loop event trigger watcher lagged {} events",
                        count
                    );
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_eval::{DomainOperationalGateInput, DomainSoakReportInput};
    use crate::goal::{CreateGoalInput, UpdateGoalInput};
    use crate::session::NewMessage;
    use crate::workflow::WorkflowRunState;

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
            Some("criterion-1"),
            Some("finish <review> & ship"),
            LoopTriggerKind::Interval,
            &json!({ "intervalSecs": 60 }),
            None,
            "check <CI> & continue",
        );
        assert!(msg.contains("<loop_id>loop&lt;&amp;</loop_id>"));
        assert!(msg.contains("<run_id>run&amp;</run_id>"));
        assert!(msg.contains("<goal_id>goal&gt;</goal_id>"));
        assert!(msg.contains("<goal_criterion_id>criterion-1</goal_criterion_id>"));
        assert!(msg.contains(
            "<goal_criterion_text>finish &lt;review&gt; &amp; ship</goal_criterion_text>"
        ));
        assert!(msg.contains("check &lt;CI&gt; &amp; continue"));
    }

    #[test]
    fn condition_trigger_message_includes_condition_and_stop_marker_contract() {
        let msg = build_loop_trigger_message(
            "loop",
            "run",
            None,
            None,
            None,
            LoopTriggerKind::Condition,
            &json!({ "condition": "CI <green> & deployed" }),
            None,
            "inspect failures",
        );
        assert!(msg.contains("<condition>CI &lt;green&gt; &amp; deployed</condition>"));
        assert!(msg.contains("LOOP_CONDITION_SATISFIED:"));
        assert!(msg.contains("inspect failures"));
    }

    #[test]
    fn event_trigger_message_includes_bounded_event_context() {
        let msg = build_loop_trigger_message(
            "loop",
            "run",
            None,
            None,
            None,
            LoopTriggerKind::Event,
            &json!({ "eventName": "workflow:updated" }),
            Some(&json!({
                "eventName": "workflow:updated",
                "payload": { "id": "wf_1", "state": "completed" },
            })),
            "summarize workflow outcome",
        );
        assert!(msg.contains("<event_context>"));
        assert!(msg.contains("workflow:updated"));
        assert!(msg.contains("summarize workflow outcome"));
    }

    #[test]
    fn interval_trigger_requires_positive_secs() {
        let input = CreateLoopScheduleInput {
            session_id: "s".into(),
            goal_id: None,
            goal_criterion_id: None,
            prompt: "poll".into(),
            trigger_kind: LoopTriggerKind::Interval,
            trigger_spec: json!({ "intervalSecs": 0 }),
            execution_strategy: LoopExecutionStrategy::Continue,
            max_runs: None,
            max_runtime_secs: None,
            token_budget: None,
            cost_budget_micros: None,
            max_no_progress_runs: None,
            max_failures: None,
            backoff_secs: None,
            agent_id: None,
        };
        assert!(cron_schedule_from_loop(&input).is_err());
    }

    #[test]
    fn event_loop_enqueue_dedups_and_consumes_event_context() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "summarize completed workflow".into(),
                    trigger_kind: LoopTriggerKind::Event,
                    trigger_spec: json!({
                        "eventName": "workflow:updated",
                        "filters": { "workflowState": "completed" },
                        "debounceSecs": 30,
                    }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
                    agent_id: None,
                },
            )
            .expect("event loop");
        let cron_job = cron_db
            .get_job(&schedule.cron_job_id)
            .expect("cron job")
            .expect("cron job exists");
        assert_eq!(cron_job.status.as_str(), "paused");

        let event = AppEvent {
            name: "workflow:updated".to_string(),
            payload: json!({
                "id": "wf_1",
                "sessionId": session.id,
                "kind": "analysis",
                "state": "completed",
            }),
        };
        let first = session_db
            .enqueue_loop_event_triggers(&event)
            .expect("enqueue first");
        let second = session_db
            .enqueue_loop_event_triggers(&event)
            .expect("enqueue duplicate");
        assert_eq!(first, vec![schedule.cron_job_id.clone()]);
        assert!(second.is_empty());

        let admission = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &schedule.session_id, &now_rfc3339())
            .expect("prepare");
        let LoopRunDecision::Admit(admission) = admission else {
            panic!("event loop should admit");
        };
        let event_context = admission.event_context.expect("event context");
        assert_eq!(event_context["eventName"], json!("workflow:updated"));
        assert_eq!(
            event_context["payload"]["matched"]["state"],
            json!("completed")
        );

        let runs = session_db.list_loop_runs(&schedule.id, 1).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].trace["eventContext"]["eventName"],
            json!("workflow:updated")
        );
        assert!(!session_db
            .loop_has_pending_event_ticks(&schedule.id)
            .expect("pending"));
    }

    #[test]
    fn event_loop_filter_mismatch_does_not_enqueue() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "handle failed workflow".into(),
                    trigger_kind: LoopTriggerKind::Event,
                    trigger_spec: json!({
                        "eventName": "workflow:updated",
                        "filters": { "workflowState": "failed" },
                    }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
                    agent_id: None,
                },
            )
            .expect("event loop");
        let event = AppEvent {
            name: "workflow:updated".to_string(),
            payload: json!({
                "id": "wf_1",
                "sessionId": session.id,
                "state": "completed",
            }),
        };
        let enqueued = session_db
            .enqueue_loop_event_triggers(&event)
            .expect("enqueue mismatch");
        assert!(enqueued.is_empty());
        assert!(!session_db
            .loop_has_pending_event_ticks(&schedule.id)
            .expect("pending"));
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
                    goal_criterion_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: Some(1),
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
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
                    goal_criterion_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: Some(10),
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
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
                    goal_criterion_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
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
                    goal_criterion_id: None,
                    prompt: "".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Workflow,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
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
                completion_criteria: "[required] Draft is reviewed against stakeholders"
                    .to_string(),
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
                    goal_criterion_id: Some("criterion-1".to_string()),
                    prompt: "Update the memo with the newest evidence".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Workflow,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
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
        assert_eq!(admission.goal_criterion_id.as_deref(), Some("criterion-1"));

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
        assert_eq!(runs[0].goal_criterion_id.as_deref(), Some("criterion-1"));
        assert_eq!(
            runs[0].goal_criterion_text.as_deref(),
            Some("Draft is reviewed against stakeholders")
        );
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
    fn workflow_strategy_feeds_operational_and_soak_gates() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let goal = session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Keep the weekly writing brief fresh".to_string(),
                completion_criteria: "A reviewed writing brief workflow completes".to_string(),
                domain: Some("writing".to_string()),
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
                    goal_criterion_id: None,
                    prompt: "Refresh the brief from the newest evidence".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Workflow,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
                    agent_id: None,
                },
            )
            .expect("create workflow loop");
        let started_at = now_rfc3339();
        let admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &started_at)
            .expect("prepare loop")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };
        let launch = session_db
            .create_loop_workflow_run(&admission)
            .expect("create loop workflow run");
        session_db
            .transition_workflow_run(&launch.run_id, WorkflowRunState::Running, Some("loop_tick"))
            .expect("start workflow");
        session_db
            .transition_workflow_run(
                &launch.run_id,
                WorkflowRunState::Completed,
                Some("loop_tick_completed"),
            )
            .expect("complete workflow");
        let finished_at = now_rfc3339();
        session_db
            .finish_loop_cron_run_with_trace(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("workflow launched and drained"),
                None,
                &finished_at,
                Some(json!({
                    "executionStrategy": "workflow",
                    "workflowRunId": launch.run_id,
                    "workflowKind": launch.workflow_kind,
                    "templateId": launch.template_id,
                    "templateVersion": launch.template_version,
                })),
            )
            .expect("finish loop run");

        let operational = session_db
            .evaluate_domain_operational_gate(DomainOperationalGateInput {
                session_id: Some(session.id.clone()),
                domain: Some("writing".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(1),
                ..Default::default()
            })
            .expect("evaluate operational gate");
        assert_eq!(operational.status, "passed", "{operational:?}");
        assert_eq!(operational.summary.workflow_runs, 1);
        assert_eq!(operational.summary.completed_workflow_runs, 1);
        assert_eq!(operational.summary.loop_runs, 1);
        assert_eq!(operational.summary.succeeded_loop_runs, 1);
        assert_eq!(operational.summary.active_workflow_runs, 0);
        assert!(operational.blockers.is_empty());

        let soak = session_db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id.clone()),
                domain: Some("writing".to_string()),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            })
            .expect("generate soak report");
        assert_eq!(soak.status, "passed", "{soak:?}");
        assert_eq!(soak.summary.workflow_runs, 1);
        assert_eq!(soak.summary.completed_workflow_runs, 1);
        assert_eq!(soak.summary.loop_runs, 1);
        assert_eq!(soak.summary.succeeded_loop_runs, 1);
        assert_eq!(soak.summary.critical_incidents, 0);
        assert!(soak
            .timeline
            .iter()
            .any(|item| item.source == "workflow" && item.id == launch.run_id));
        assert!(soak
            .timeline
            .iter()
            .any(|item| item.source == "loop" && item.id == admission.run_id));
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
                    goal_criterion_id: None,
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
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
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

    #[test]
    fn no_progress_backoff_then_blocks_after_threshold() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: Some(2),
                    max_failures: Some(3),
                    backoff_secs: Some(60),
                    agent_id: None,
                },
            )
            .expect("create loop");

        let first = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare first")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected first admission, got {other:?}"),
        };
        let first_action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&first.run_id),
                None,
                LoopRunState::Succeeded,
                None,
                None,
                &now_rfc3339(),
            )
            .expect("finish first");
        assert_eq!(first_action.backoff_secs, Some(60));
        let after_first = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load first")
            .expect("schedule");
        assert_eq!(after_first.state, LoopState::Active);
        assert_eq!(
            after_first.progress_state,
            Some(LoopProgressState::NoProgress)
        );
        assert_eq!(after_first.no_progress_streak, 1);

        let second = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare second")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected second admission, got {other:?}"),
        };
        let second_action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&second.run_id),
                None,
                LoopRunState::Succeeded,
                None,
                None,
                &now_rfc3339(),
            )
            .expect("finish second");
        assert!(second_action.pause_cron_job);
        let after_second = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load second")
            .expect("schedule");
        assert_eq!(after_second.state, LoopState::Blocked);
        assert_eq!(after_second.no_progress_streak, 2);
        assert!(
            after_second
                .blocked_reason
                .as_deref()
                .unwrap_or("")
                .contains("no new durable Goal evidence")
                || after_second
                    .blocked_reason
                    .as_deref()
                    .unwrap_or("")
                    .contains("no durable signal")
        );
    }

    #[test]
    fn durable_goal_evidence_resets_no_progress_streak() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let goal = session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Finish the artifact".to_string(),
                completion_criteria: "A reviewed artifact exists".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
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
                    goal_criterion_id: None,
                    prompt: "make progress".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: Some(2),
                    max_failures: Some(3),
                    backoff_secs: Some(60),
                    agent_id: None,
                },
            )
            .expect("create loop");
        let started_at = now_rfc3339();
        let admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &started_at)
            .expect("prepare")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };
        session_db
            .link_goal_target(
                &goal.goal.id,
                "file",
                "/tmp/artifact.md",
                "file_changed",
                json!({ "source": "test" }),
            )
            .expect("link evidence");
        session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("updated artifact"),
                None,
                &now_rfc3339(),
            )
            .expect("finish");
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load")
            .expect("schedule");
        assert_eq!(updated.progress_state, Some(LoopProgressState::Progressed));
        assert_eq!(updated.no_progress_streak, 0);
        let runs = session_db.list_loop_runs(&schedule.id, 10).expect("runs");
        assert_eq!(runs[0].progress_state, Some(LoopProgressState::Progressed));
        assert_eq!(runs[0].progress_delta["goalEvidence"]["strong"], json!(1));
    }

    #[test]
    fn goal_completed_stops_bound_loop_before_next_trigger() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let goal = session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Ship".to_string(),
                completion_criteria: "Done".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
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
                    goal_criterion_id: None,
                    prompt: "continue".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        session_db
            .transition_goal(&goal.goal.id, GoalState::Completed, Some("done"))
            .expect("complete goal");
        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare");
        assert!(matches!(decision, LoopRunDecision::Reject(_)));
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load")
            .expect("schedule");
        assert_eq!(updated.state, LoopState::Completed);
    }

    #[test]
    fn criteria_revision_change_blocks_loop_until_rebind() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let goal = session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare release".to_string(),
                completion_criteria: "[required] Smoke test passes".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
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
                    goal_criterion_id: Some("criterion-1".to_string()),
                    prompt: "check release".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        session_db
            .update_goal(UpdateGoalInput {
                goal_id: goal.goal.id.clone(),
                objective: None,
                completion_criteria: Some("[required] Manual QA passes".to_string()),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
            })
            .expect("update goal");
        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare");
        assert!(matches!(decision, LoopRunDecision::Reject(_)));
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load")
            .expect("schedule");
        assert_eq!(updated.state, LoopState::Blocked);
        assert!(updated
            .blocked_reason
            .as_deref()
            .unwrap_or("")
            .contains("needs rebind"));
    }

    #[test]
    fn loop_policy_update_persists_budget_and_cron_guard() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "poll".into(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 60 }),
                    execution_strategy: LoopExecutionStrategy::Continue,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    max_no_progress_runs: None,
                    max_failures: None,
                    backoff_secs: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        let updated = session_db
            .update_loop_schedule_policy(
                &cron_db,
                UpdateLoopSchedulePolicyInput {
                    loop_id: schedule.id.clone(),
                    max_runs: Some(4),
                    max_runtime_secs: Some(120),
                    token_budget: Some(10_000),
                    max_no_progress_runs: Some(5),
                    max_failures: Some(6),
                    backoff_secs: Some(900),
                },
            )
            .expect("update policy");
        assert_eq!(updated.max_runs, Some(4));
        assert_eq!(updated.max_runtime_secs, Some(120));
        assert_eq!(updated.token_budget, Some(10_000));
        assert_eq!(updated.max_no_progress_runs, Some(5));
        assert_eq!(updated.max_failures, Some(6));
        assert_eq!(updated.backoff_secs, Some(900));
        assert!(updated.next_run_at.is_some());
        let cron_job = cron_db
            .get_job(&schedule.cron_job_id)
            .expect("load cron")
            .expect("cron job");
        assert_eq!(cron_job.max_failures, 6);
        assert_eq!(cron_job.job_timeout_secs, Some(120));
    }
}
