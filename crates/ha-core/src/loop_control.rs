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
use std::{
    collections::HashMap,
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::cron::{CronDB, CronPayload, CronSchedule, NewCronJob};
use crate::event_bus::AppEvent;
use crate::goal::GoalState;
use crate::session::{MessageRole, SessionDB};

const LOOP_TRACE_MAX_BYTES: usize = 64 * 1024;
const DEFAULT_UNTIL_INTERVAL_SECS: i64 = 300;
const EVENT_LOOP_IDLE_INTERVAL_SECS: u64 = 366 * 24 * 60 * 60;
const DEFAULT_EVENT_DEBOUNCE_SECS: i64 = 30;
const MAX_EVENT_DEBOUNCE_SECS: i64 = 3600;
const DEFAULT_DYNAMIC_LOOP_FALLBACK_SECS: i64 = 20 * 60;
const MIN_DYNAMIC_LOOP_RESCHEDULE_SECS: i64 = 60;
const MAX_DYNAMIC_LOOP_RESCHEDULE_SECS: i64 = 60 * 60;
const DEFAULT_DYNAMIC_LOOP_MAX_RUNTIME_SECS: i64 = 7 * 24 * 60 * 60;
const LOOP_MD_MAX_BYTES: usize = 25_000;
const BUILTIN_LOOP_MAINTENANCE_PROMPT: &str = "\
Continue this session as a self-paced maintenance loop.

At each iteration:
- Review the current goal, visible tasks, workflow/loop status, recent assistant context, and workspace state.
- Take one conservative useful step toward unfinished user-visible work.
- Prefer verification, cleanup, documentation, review follow-up, or clear progress reporting when there is no obvious edit to make.
- Do not invent work, do not make irreversible external changes without explicit approval, and stop or report blocked when there is nothing useful to do.";
const DEFAULT_LOOP_MAX_NO_PROGRESS_RUNS: i64 = 3;
const DEFAULT_LOOP_MAX_FAILURES: i64 = 3;
const DEFAULT_LOOP_BACKOFF_SECS: i64 = 300;
const MAX_LOOP_BACKOFF_SECS: i64 = 24 * 60 * 60;
const LOOP_MONITOR_MAX_FAILURES: i64 = 3;
const LOOP_MONITOR_PAYLOAD_MAX_BYTES: usize = 8 * 1024;
const MAX_ACTIVE_WATCHES_PER_LOOP: i64 = 16;
const MAX_ACTIVE_MONITORS_PER_SESSION: i64 = 8;
const MAX_ACTIVE_MONITORS_GLOBAL: i64 = 64;
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

#[derive(Debug, Clone)]
pub struct DefaultLoopPromptResolution {
    pub prompt: String,
    pub metadata: Value,
}

pub fn resolve_default_loop_prompt_for_session(
    session_db: &SessionDB,
    session_id: &str,
) -> DefaultLoopPromptResolution {
    for candidate in loop_prompt_candidates(session_db, session_id) {
        if let Some(resolution) = read_loop_prompt_file(&candidate) {
            return resolution;
        }
    }
    DefaultLoopPromptResolution {
        prompt: BUILTIN_LOOP_MAINTENANCE_PROMPT.to_string(),
        metadata: json!({
            "enabled": true,
            "source": "builtin",
            "contentHash": hash_text(BUILTIN_LOOP_MAINTENANCE_PROMPT),
        }),
    }
}

pub fn dynamic_loop_trigger_spec_with_maintenance_prompt(metadata: Value) -> Value {
    json!({
        "fallbackSecs": DEFAULT_DYNAMIC_LOOP_FALLBACK_SECS,
        "fallbackUsed": false,
        "maintenancePrompt": normalize_maintenance_prompt_spec(Some(&metadata))
            .unwrap_or_else(|| json!({ "enabled": true, "source": "unknown" })),
    })
}

pub fn default_dynamic_loop_trigger_spec() -> Value {
    json!({
        "fallbackSecs": DEFAULT_DYNAMIC_LOOP_FALLBACK_SECS,
        "fallbackUsed": false,
    })
}

/// Start an active Loop schedule immediately through the same primary-only
/// Cron execution path used by owner `run-now` commands. This only enqueues an
/// immediate run; it does not alter the recurring schedule.
pub fn spawn_loop_schedule_run_now(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<SessionDB>,
    loop_id: &str,
) -> Result<()> {
    if !crate::runtime_lock::is_primary() {
        return Err(anyhow!(
            "run-now is unavailable on this instance: scheduled jobs only run on the primary"
        ));
    }
    let schedule = session_db
        .get_loop_schedule(loop_id)?
        .ok_or_else(|| anyhow!("Loop schedule not found"))?;
    if schedule.state.is_terminal() {
        return Err(anyhow!(
            "loop schedule {} is {}",
            schedule.id,
            schedule.state.as_str()
        ));
    }
    if schedule.state != LoopState::Active {
        return Err(anyhow!(
            "loop schedule {} must be active before run-now; current state is {}",
            schedule.id,
            schedule.state.as_str()
        ));
    }
    let job = cron_db
        .get_job(&schedule.cron_job_id)?
        .ok_or_else(|| anyhow!("Cron job not found"))?;
    crate::cron::spawn_job_execution(cron_db.clone(), session_db.clone(), job);
    Ok(())
}

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
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopWatchKind {
    AppEvent,
    Job,
    Subagent,
    File,
    Command,
    Websocket,
}

impl LoopWatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppEvent => "app_event",
            Self::Job => "job",
            Self::Subagent => "subagent",
            Self::File => "file",
            Self::Command => "command",
            Self::Websocket => "websocket",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "app_event" => Some(Self::AppEvent),
            "job" => Some(Self::Job),
            "subagent" => Some(Self::Subagent),
            "file" => Some(Self::File),
            "command" => Some(Self::Command),
            "websocket" => Some(Self::Websocket),
            _ => None,
        }
    }

    fn is_event_backed(self) -> bool {
        matches!(
            self,
            Self::AppEvent | Self::Job | Self::Subagent | Self::Command
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopWatch {
    pub id: String,
    pub loop_id: String,
    pub kind: LoopWatchKind,
    pub spec: Value,
    pub active: bool,
    pub generation: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fingerprint: Option<String>,
    pub failure_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor_job_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

enum LoopMonitorHandle {
    File {
        generation: i64,
        _watcher: notify::RecommendedWatcher,
    },
    Async {
        generation: i64,
        cancel: CancellationToken,
    },
}

fn loop_monitors() -> &'static StdMutex<HashMap<String, LoopMonitorHandle>> {
    static MONITORS: OnceLock<StdMutex<HashMap<String, LoopMonitorHandle>>> = OnceLock::new();
    MONITORS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn stop_loop_monitor(watch_id: &str) {
    // Take the handle out of the registry first and only then act on it: a
    // File handle's drop joins the notify event thread, and that thread's
    // callback may itself be waiting on the registry mutex (one-shot settle) —
    // dropping under the lock would deadlock the pair.
    let removed = loop_monitors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(watch_id);
    if let Some(LoopMonitorHandle::Async { cancel, .. }) = removed {
        cancel.cancel();
    }
}

fn remove_loop_monitor_generation(watch_id: &str, generation: i64) {
    let removed = {
        let mut monitors = loop_monitors()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let matches_generation = match monitors.get(watch_id) {
            Some(LoopMonitorHandle::File {
                generation: current,
                ..
            })
            | Some(LoopMonitorHandle::Async {
                generation: current,
                ..
            }) => *current == generation,
            None => false,
        };
        if matches_generation {
            monitors.remove(watch_id)
        } else {
            None
        }
    };
    if let Some(handle) = removed {
        if matches!(handle, LoopMonitorHandle::File { .. }) {
            // Dropping a File handle joins the notify event thread. This fn is
            // reachable from that very thread (the one-shot settle runs inside
            // the notify callback), where an inline drop would self-join and
            // deadlock — while our caller chain sits on the registry mutex.
            // Hand the drop to a detached thread; settles are rare one-shots.
            std::thread::spawn(move || drop(handle));
        }
    }
}

impl LoopTriggerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interval => "interval",
            Self::Cron => "cron",
            Self::Condition => "condition",
            Self::Event => "event",
            Self::Dynamic => "dynamic",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "interval" => Some(Self::Interval),
            "cron" => Some(Self::Cron),
            "condition" => Some(Self::Condition),
            "event" => Some(Self::Event),
            "dynamic" => Some(Self::Dynamic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExecutionStrategy {
    #[default]
    Continue,
    Workflow,
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
    pub usage: LoopRunUsageSnapshot,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRunUsageSnapshot {
    pub message_count: i64,
    pub user_turns: i64,
    pub assistant_messages: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub attribution: String,
    pub provider_events: i64,
    pub provider_input_tokens: i64,
    pub provider_output_tokens: i64,
    pub provider_cache_creation_input_tokens: i64,
    pub provider_cache_read_input_tokens: i64,
    pub provider_total_tokens: i64,
    pub provider_attribution: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopSnapshot {
    pub schedule: LoopSchedule,
    #[serde(default)]
    pub runs: Vec<LoopRun>,
    #[serde(default)]
    pub watches: Vec<LoopWatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopWatchdogFinding {
    pub loop_id: String,
    pub session_id: String,
    pub severity: String,
    pub code: String,
    pub message: String,
    pub next_run_at: Option<String>,
    pub overdue_secs: Option<i64>,
    pub cron_status: Option<String>,
    pub latest_run_id: Option<String>,
    pub latest_run_state: Option<String>,
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
struct LoopMaintenancePromptRefresh {
    prompt: String,
    trigger_spec: Value,
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
enum DynamicLoopDecision {
    Reschedule { delay_secs: i64, reason: String },
    Stop { reason: String },
    Block { reason: String },
    Missing,
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

        CREATE TABLE IF NOT EXISTS loop_watches (
            id TEXT PRIMARY KEY,
            loop_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            spec_json TEXT NOT NULL DEFAULT '{}',
            active INTEGER NOT NULL DEFAULT 1,
            generation INTEGER NOT NULL DEFAULT 1,
            last_event_at TEXT,
            last_fingerprint TEXT,
            failure_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            monitor_job_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (loop_id) REFERENCES loop_schedules(id) ON DELETE CASCADE
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
            ON loop_event_ticks(loop_id, consumed_at, created_at);
        CREATE INDEX IF NOT EXISTS idx_loop_watches_loop_active
            ON loop_watches(loop_id, active, updated_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_loop_watches_loop_kind_spec
            ON loop_watches(loop_id, kind, spec_json);",
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
    ensure_loop_watch_column(
        conn,
        "monitor_job_id",
        "ALTER TABLE loop_watches ADD COLUMN monitor_job_id TEXT;",
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

fn ensure_loop_watch_column(conn: &Connection, column: &str, alter_sql: &str) -> Result<()> {
    let query = format!("SELECT {column} FROM loop_watches LIMIT 1");
    if conn.prepare(&query).is_ok() {
        return Ok(());
    }
    conn.execute(alter_sql, [])?;
    Ok(())
}

impl SessionDB {
    fn deactivate_loop_watch_for_monitor_job(&self, monitor_job_id: &str) -> Result<bool> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let watch_id: Option<String> = conn
            .query_row(
                "SELECT id FROM loop_watches WHERE monitor_job_id = ?1 AND active = 1",
                params![monitor_job_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(watch_id) = watch_id else {
            return Ok(false);
        };
        conn.execute(
            "UPDATE loop_watches
             SET active = 0, generation = generation + 1,
                 monitor_job_id = NULL, updated_at = ?2
             WHERE id = ?1 AND active = 1",
            params![watch_id, now],
        )?;
        drop(conn);
        stop_loop_monitor(&watch_id);
        Ok(true)
    }

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

    pub fn list_loop_watchdog_findings(
        &self,
        cron_db: &CronDB,
        session_id: &str,
        grace_secs: i64,
    ) -> Result<Vec<LoopWatchdogFinding>> {
        let grace_secs = grace_secs.max(0);
        let now = Utc::now();
        let schedules = self.list_loop_schedules_for_session_with_cron(cron_db, session_id, 100)?;
        let mut findings = Vec::new();

        for schedule in schedules {
            if schedule.state != LoopState::Active {
                continue;
            }
            if schedule.trigger_kind == LoopTriggerKind::Event {
                continue;
            }

            let Some(cron_job) = cron_db.get_job(&schedule.cron_job_id)? else {
                findings.push(LoopWatchdogFinding {
                    loop_id: schedule.id,
                    session_id: schedule.session_id,
                    severity: "warning".to_string(),
                    code: "loop_cron_missing".to_string(),
                    message: "Loop is active but its backing Cron job is missing.".to_string(),
                    next_run_at: schedule.next_run_at.clone(),
                    overdue_secs: schedule.next_run_at.as_deref().and_then(|value| {
                        DateTime::parse_from_rfc3339(value)
                            .ok()
                            .map(|dt| (now - dt.with_timezone(&Utc)).num_seconds().max(0))
                    }),
                    cron_status: None,
                    latest_run_id: None,
                    latest_run_state: None,
                });
                continue;
            };

            if cron_job.status.as_str() != "active" {
                continue;
            }

            let latest_run = self.list_loop_runs(&schedule.id, 1)?.into_iter().next();
            if let Some(run) = latest_run.as_ref() {
                if run.state == LoopRunState::Running && cron_job.running_at.is_none() {
                    if let Ok(started_at) = DateTime::parse_from_rfc3339(&run.started_at)
                        .map(|dt| dt.with_timezone(&Utc))
                    {
                        let run_age_secs = (now - started_at).num_seconds();
                        if run_age_secs > grace_secs {
                            findings.push(LoopWatchdogFinding {
                                loop_id: schedule.id,
                                session_id: schedule.session_id,
                                severity: "warning".to_string(),
                                code: "loop_run_maybe_interrupted".to_string(),
                                message: "Loop has a running run but its backing Cron job is no longer running."
                                    .to_string(),
                                next_run_at: schedule.next_run_at.clone(),
                                overdue_secs: Some(run_age_secs),
                                cron_status: Some(cron_job.status.as_str().to_string()),
                                latest_run_id: Some(run.id.clone()),
                                latest_run_state: Some(run.state.as_str().to_string()),
                            });
                            continue;
                        }
                    }
                }
                if matches!(
                    run.state,
                    LoopRunState::Running | LoopRunState::Queued | LoopRunState::Injected
                ) {
                    continue;
                }
            }

            if cron_job.running_at.is_some() {
                continue;
            }

            let Some(next_run_at) = schedule.next_run_at.as_deref() else {
                continue;
            };
            let Ok(next_run) =
                DateTime::parse_from_rfc3339(next_run_at).map(|dt| dt.with_timezone(&Utc))
            else {
                continue;
            };
            let overdue_secs = (now - next_run).num_seconds();
            if overdue_secs <= grace_secs {
                continue;
            }

            findings.push(LoopWatchdogFinding {
                loop_id: schedule.id,
                session_id: schedule.session_id,
                severity: "warning".to_string(),
                code: "loop_due_not_claimed".to_string(),
                message: "Loop is past its scheduled run time but no active loop run is recorded."
                    .to_string(),
                next_run_at: Some(next_run_at.to_string()),
                overdue_secs: Some(overdue_secs),
                cron_status: Some(cron_job.status.as_str().to_string()),
                latest_run_id: latest_run.as_ref().map(|run| run.id.clone()),
                latest_run_state: latest_run.map(|run| run.state.as_str().to_string()),
            });
        }

        Ok(findings)
    }

    pub fn loop_snapshot(&self, loop_id: &str, run_limit: usize) -> Result<Option<LoopSnapshot>> {
        let Some(schedule) = self.get_loop_schedule(loop_id)? else {
            return Ok(None);
        };
        let runs = self.list_loop_runs(loop_id, run_limit)?;
        let watches = self.list_loop_watches(loop_id)?;
        Ok(Some(LoopSnapshot {
            schedule,
            runs,
            watches,
        }))
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
        let mut runs = collect_rows(rows)?;
        for run in &mut runs {
            run.usage = loop_run_usage_snapshot_with_conn(&conn, run)?;
        }
        Ok(runs)
    }

    pub fn list_loop_watches(&self, loop_id: &str) -> Result<Vec<LoopWatch>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, loop_id, kind, spec_json, active, generation, last_event_at,
                    last_fingerprint, failure_count, last_error, created_at, updated_at,
                    monitor_job_id
             FROM loop_watches
             WHERE loop_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![loop_id], row_to_loop_watch)?;
        collect_rows(rows)
    }

    pub fn upsert_loop_watch(
        &self,
        loop_id: &str,
        kind: LoopWatchKind,
        spec: &Value,
    ) -> Result<LoopWatch> {
        let schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        if schedule.trigger_kind != LoopTriggerKind::Dynamic {
            return Err(anyhow!(
                "loop watches attach only to dynamic loops; {} is {}",
                schedule.id,
                schedule.trigger_kind.as_str()
            ));
        }
        if schedule.state != LoopState::Active {
            return Err(anyhow!(
                "loop schedule {} is {}; only active dynamic loops can add or re-arm watches",
                schedule.id,
                schedule.state.as_str()
            ));
        }
        let spec = normalize_loop_watch_spec(kind, spec)?;
        let previous = self.find_loop_watch_by_spec(loop_id, kind, &spec)?;
        if let Some(previous) = previous.as_ref() {
            stop_loop_monitor(&previous.id);
            if let Some(job_id) = previous.monitor_job_id.as_deref() {
                let _ = crate::async_jobs::JobManager::finish_monitor(
                    job_id,
                    crate::async_jobs::JobStatus::Cancelled,
                    None,
                    Some("Loop monitor re-armed"),
                );
            }
        }
        let spec_json = stable_json(&spec)?;
        let now = now_rfc3339();
        let id = format!("lwatch_{}", uuid::Uuid::new_v4().simple());
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            if previous.as_ref().is_none_or(|watch| !watch.active) {
                ensure_loop_watch_capacity(&conn, &schedule, kind)?;
            }
            conn.execute(
                "INSERT INTO loop_watches (
                    id, loop_id, kind, spec_json, active, generation, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, ?5)
                 ON CONFLICT(loop_id, kind, spec_json) DO UPDATE SET
                    active = 1,
                    generation = loop_watches.generation + 1,
                    failure_count = 0,
                    last_error = NULL,
                    monitor_job_id = NULL,
                    updated_at = excluded.updated_at",
                params![id, loop_id, kind.as_str(), spec_json, now],
            )?;
        }
        let watch = self
            .find_loop_watch_by_spec(loop_id, kind, &spec)?
            .ok_or_else(|| anyhow!("failed to read the created loop watch"))?;
        emit_loop_watch_event("loop:watch_changed", &schedule, &watch);
        Ok(watch)
    }

    pub fn remove_loop_watch(&self, loop_id: &str, watch_id: &str) -> Result<LoopWatch> {
        let schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        let watch = self
            .get_loop_watch(watch_id)?
            .ok_or_else(|| anyhow!("loop watch not found: {watch_id}"))?;
        if watch.loop_id != loop_id {
            return Err(anyhow!("loop watch does not belong to loop {loop_id}"));
        }
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_watches
             SET active = 0, generation = generation + 1,
                 monitor_job_id = NULL, updated_at = ?2
             WHERE id = ?1",
            params![watch_id, now],
        )?;
        drop(conn);
        if let Some(job_id) = watch.monitor_job_id.as_deref() {
            let _ = crate::async_jobs::JobManager::finish_monitor(
                job_id,
                crate::async_jobs::JobStatus::Cancelled,
                None,
                Some("Loop watch removed"),
            );
        }
        let updated = self
            .get_loop_watch(watch_id)?
            .ok_or_else(|| anyhow!("loop watch not found after update"))?;
        stop_loop_monitor(watch_id);
        emit_loop_watch_event("loop:watch_changed", &schedule, &updated);
        Ok(updated)
    }

    fn get_loop_watch(&self, watch_id: &str) -> Result<Option<LoopWatch>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
                "SELECT id, loop_id, kind, spec_json, active, generation, last_event_at,
                        last_fingerprint, failure_count, last_error, created_at, updated_at,
                        monitor_job_id
                 FROM loop_watches WHERE id = ?1",
                params![watch_id],
                row_to_loop_watch,
            )
            .optional()?)
    }

    fn find_loop_watch_by_spec(
        &self,
        loop_id: &str,
        kind: LoopWatchKind,
        spec: &Value,
    ) -> Result<Option<LoopWatch>> {
        let spec_json = stable_json(spec)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        Ok(conn
            .query_row(
                "SELECT id, loop_id, kind, spec_json, active, generation, last_event_at,
                        last_fingerprint, failure_count, last_error, created_at, updated_at,
                        monitor_job_id
                 FROM loop_watches
                 WHERE loop_id = ?1 AND kind = ?2 AND spec_json = ?3",
                params![loop_id, kind.as_str(), spec_json],
                row_to_loop_watch,
            )
            .optional()?)
    }

    fn bind_loop_watch_monitor_job(&self, watch_id: &str, job_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_watches SET monitor_job_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![watch_id, job_id, now_rfc3339()],
        )?;
        Ok(())
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
        for (schedule, watch) in
            self.list_active_event_backed_loop_watches_for_session(session_id)?
        {
            let Some(mut event_match) =
                loop_event_matches_spec(&schedule, &watch.spec, event, &now)?
            else {
                continue;
            };
            event_match.context["watch"] = json!({
                "id": watch.id,
                "kind": watch.kind.as_str(),
                "generation": watch.generation,
            });
            let fingerprint =
                blake3::hash(format!("{}:{}", watch.id, event_match.fingerprint).as_bytes())
                    .to_hex()
                    .to_string();
            if self.insert_loop_event_tick(
                &schedule.id,
                &event.name,
                &fingerprint,
                &event_match.context,
                &now.to_rfc3339(),
            )? {
                self.mark_loop_watch_event(&watch.id, &fingerprint, &now.to_rfc3339())?;
                if !cron_job_ids.contains(&schedule.cron_job_id) {
                    cron_job_ids.push(schedule.cron_job_id);
                }
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
        spawn_loop_monitor_recovery_for_loop(schedule.id.clone());
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

    pub fn record_loop_tool_progress(
        &self,
        loop_id: &str,
        progress_state: LoopProgressState,
        summary: &str,
        reason: Option<&str>,
        metadata: Value,
    ) -> Result<LoopSchedule> {
        let schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        if schedule.state.is_terminal() {
            return Err(anyhow!(
                "loop schedule {} is {}; terminal loops cannot record progress",
                schedule.id,
                schedule.state.as_str()
            ));
        }
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE loop_schedules
                 SET progress_state = ?2,
                     progress_summary = ?3,
                     updated_at = ?4
                 WHERE id = ?1",
                params![loop_id, progress_state.as_str(), summary, now],
            )?;
        }
        if let Some(run_id) = self.latest_running_loop_run_id(loop_id)? {
            self.patch_loop_run_trace(
                &run_id,
                json!({
                    "agentToolProgress": {
                        "source": "tool",
                        "state": progress_state.as_str(),
                        "summary": summary,
                        "reason": reason,
                        "metadata": metadata,
                        "recordedAt": now,
                    }
                }),
                None,
            )?;
        }
        let updated = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        emit_loop_event("loop:changed", &updated);
        Ok(updated)
    }

    pub fn record_loop_tool_reschedule(
        &self,
        cron_db: &CronDB,
        loop_id: &str,
        delay_secs: i64,
        reason: &str,
    ) -> Result<(LoopSchedule, Option<String>)> {
        let schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        if schedule.state != LoopState::Active {
            return Err(anyhow!(
                "loop schedule {} must be active to reschedule; current state is {}",
                schedule.id,
                schedule.state.as_str()
            ));
        }
        if schedule.trigger_kind != LoopTriggerKind::Dynamic {
            return Err(anyhow!(
                "loop schedule {} is {}; loop_reschedule only controls dynamic loops",
                schedule.id,
                schedule.trigger_kind.as_str()
            ));
        }
        let delay_secs = delay_secs.clamp(
            MIN_DYNAMIC_LOOP_RESCHEDULE_SECS,
            MAX_DYNAMIC_LOOP_RESCHEDULE_SECS,
        );
        let next_run_at = cron_db.delay_next_run(&schedule.cron_job_id, delay_secs)?;
        self.set_dynamic_loop_fallback_used(&schedule.id, &schedule.trigger_spec, false)?;
        if let Some(run_id) = self.latest_running_loop_run_id(loop_id)? {
            self.patch_loop_run_trace(
                &run_id,
                json!({
                    "dynamicDecision": {
                        "source": "tool",
                        "action": "reschedule",
                        "delaySecs": delay_secs,
                        "reason": reason,
                    }
                }),
                None,
            )?;
            self.update_loop_run_scheduling_decision(
                &run_id,
                Some(&format!("dynamic_reschedule_{delay_secs}s")),
            )?;
        }
        let mut updated = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        hydrate_loop_schedule_from_cron(cron_db, &mut updated)?;
        emit_loop_event("loop:changed", &updated);
        Ok((updated, next_run_at))
    }

    pub fn record_loop_tool_stop(
        &self,
        cron_db: &CronDB,
        loop_id: &str,
        completed: bool,
        reason: &str,
    ) -> Result<LoopSchedule> {
        let current = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        if current.state.is_terminal() {
            return Err(anyhow!(
                "loop schedule {} is already {}",
                current.id,
                current.state.as_str()
            ));
        }
        let next_state = if completed {
            LoopState::Completed
        } else {
            LoopState::Blocked
        };
        if let Some(run_id) = self.latest_running_loop_run_id(loop_id)? {
            self.patch_loop_run_trace(
                &run_id,
                json!({
                    "dynamicDecision": {
                        "source": "tool",
                        "action": if completed { "stop" } else { "block" },
                        "reason": reason,
                    }
                }),
                None,
            )?;
            self.update_loop_run_scheduling_decision(
                &run_id,
                Some(if completed {
                    "completed_dynamic_stop"
                } else {
                    "blocked_dynamic"
                }),
            )?;
        }
        let mut schedule =
            self.transition_loop_schedule(loop_id, next_state, (!completed).then_some(reason))?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE loop_schedules
                 SET progress_state = ?2,
                     progress_summary = ?3,
                     updated_at = ?4
                 WHERE id = ?1",
                params![
                    loop_id,
                    if completed {
                        LoopProgressState::Progressed.as_str()
                    } else {
                        LoopProgressState::Blocked.as_str()
                    },
                    reason,
                    now_rfc3339(),
                ],
            )?;
        }
        cron_db.toggle_job(&schedule.cron_job_id, false)?;
        if schedule.trigger_kind == LoopTriggerKind::Dynamic {
            self.set_dynamic_loop_fallback_used(&schedule.id, &schedule.trigger_spec, false)?;
        }
        schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        hydrate_loop_schedule_from_cron(cron_db, &mut schedule)?;
        emit_loop_event("loop:changed", &schedule);
        Ok(schedule)
    }

    fn refresh_dynamic_loop_maintenance_prompt(
        &self,
        schedule: &LoopSchedule,
    ) -> Result<Option<LoopMaintenancePromptRefresh>> {
        if schedule.trigger_kind != LoopTriggerKind::Dynamic
            || !dynamic_loop_uses_maintenance_prompt(&schedule.trigger_spec)
        {
            return Ok(None);
        }
        let resolution = resolve_default_loop_prompt_for_session(self, &schedule.session_id);
        let mut trigger_spec = schedule.trigger_spec.clone();
        trigger_spec["maintenancePrompt"] = resolution.metadata.clone();
        let trigger_spec = normalized_trigger_spec(LoopTriggerKind::Dynamic, &trigger_spec)?;

        let prompt_changed = schedule.prompt != resolution.prompt;
        let metadata_changed = dynamic_loop_maintenance_prompt_metadata(&schedule.trigger_spec)
            != dynamic_loop_maintenance_prompt_metadata(&trigger_spec);
        if !prompt_changed && !metadata_changed {
            return Ok(None);
        }

        let now = now_rfc3339();
        let trigger_spec_json = stable_json(&trigger_spec)?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE loop_schedules
                 SET prompt = ?2,
                     trigger_spec_json = ?3,
                     updated_at = ?4
                 WHERE id = ?1",
                params![schedule.id, resolution.prompt, trigger_spec_json, now],
            )?;
        }
        let mut updated = schedule.clone();
        updated.prompt = resolution.prompt.clone();
        updated.trigger_spec = trigger_spec.clone();
        updated.updated_at = now;
        emit_loop_event("loop:changed", &updated);

        Ok(Some(LoopMaintenancePromptRefresh {
            prompt: resolution.prompt,
            trigger_spec,
        }))
    }

    pub fn prepare_loop_cron_run(
        &self,
        cron_job_id: &str,
        session_id: &str,
        started_at: &str,
    ) -> Result<LoopRunDecision> {
        let Some(mut schedule) = self.loop_schedule_for_cron_job(cron_job_id)? else {
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
        if let Some(limit) = loop_runtime_limit_secs(&schedule) {
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

        if let Some(refreshed) = self.refresh_dynamic_loop_maintenance_prompt(&schedule)? {
            schedule.prompt = refreshed.prompt;
            schedule.trigger_spec = refreshed.trigger_spec;
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
            "maintenancePrompt": dynamic_loop_maintenance_prompt_metadata(&schedule.trigger_spec),
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
        let event_context = if matches!(
            schedule.trigger_kind,
            LoopTriggerKind::Event | LoopTriggerKind::Dynamic
        ) {
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
            if let Some(max_runtime) = loop_runtime_limit_secs(&schedule) {
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
            if !next_state.is_terminal() && schedule.trigger_kind == LoopTriggerKind::Dynamic {
                let dynamic_decision = match run_id.as_deref() {
                    Some(run_id) => self
                        .loop_run_dynamic_decision(run_id)?
                        .unwrap_or_else(|| dynamic_loop_decision(result_summary)),
                    None => dynamic_loop_decision(result_summary),
                };
                match dynamic_decision {
                    DynamicLoopDecision::Reschedule { delay_secs, reason } => {
                        backoff_secs = Some(delay_secs);
                        scheduling_decision = Some(format!("dynamic_reschedule_{delay_secs}s"));
                        self.set_dynamic_loop_fallback_used(
                            &schedule.id,
                            &schedule.trigger_spec,
                            false,
                        )?;
                        if let Some(run_id) = run_id.as_deref() {
                            self.patch_loop_run_trace(
                                run_id,
                                json!({
                                    "dynamicDecision": {
                                        "action": "reschedule",
                                        "delaySecs": delay_secs,
                                        "reason": reason,
                                    }
                                }),
                                None,
                            )?;
                        }
                    }
                    DynamicLoopDecision::Stop { reason } => {
                        next_state = LoopState::Completed;
                        pause = true;
                        backoff_secs = None;
                        scheduling_decision = Some("completed_dynamic_stop".to_string());
                        self.set_dynamic_loop_fallback_used(
                            &schedule.id,
                            &schedule.trigger_spec,
                            false,
                        )?;
                        if let Some(run_id) = run_id.as_deref() {
                            self.patch_loop_run_trace(
                                run_id,
                                json!({
                                    "dynamicDecision": {
                                        "action": "stop",
                                        "reason": reason,
                                    }
                                }),
                                None,
                            )?;
                        }
                    }
                    DynamicLoopDecision::Block { reason } => {
                        next_state = LoopState::Blocked;
                        pause = true;
                        backoff_secs = None;
                        blocked_reason = Some(reason.clone());
                        scheduling_decision = Some("blocked_dynamic".to_string());
                        self.set_dynamic_loop_fallback_used(
                            &schedule.id,
                            &schedule.trigger_spec,
                            false,
                        )?;
                        if let Some(run_id) = run_id.as_deref() {
                            self.patch_loop_run_trace(
                                run_id,
                                json!({
                                    "dynamicDecision": {
                                        "action": "block",
                                        "reason": reason,
                                    }
                                }),
                                None,
                            )?;
                        }
                    }
                    DynamicLoopDecision::Missing => {
                        if dynamic_loop_fallback_used(&schedule.trigger_spec) {
                            next_state = LoopState::Blocked;
                            pause = true;
                            backoff_secs = None;
                            blocked_reason = Some(
                                "dynamic loop did not reschedule, stop, or block after its fallback wakeup"
                                    .to_string(),
                            );
                            scheduling_decision =
                                Some("blocked_dynamic_missing_decision".to_string());
                            self.set_dynamic_loop_fallback_used(
                                &schedule.id,
                                &schedule.trigger_spec,
                                false,
                            )?;
                            if let Some(run_id) = run_id.as_deref() {
                                self.patch_loop_run_trace(
                                    run_id,
                                    json!({
                                        "dynamicDecision": {
                                            "action": "missing_after_fallback",
                                        }
                                    }),
                                    None,
                                )?;
                            }
                        } else {
                            let delay_secs = dynamic_loop_fallback_secs(&schedule.trigger_spec);
                            backoff_secs = Some(delay_secs);
                            scheduling_decision = Some(format!("dynamic_fallback_{delay_secs}s"));
                            self.set_dynamic_loop_fallback_used(
                                &schedule.id,
                                &schedule.trigger_spec,
                                true,
                            )?;
                            if let Some(run_id) = run_id.as_deref() {
                                self.patch_loop_run_trace(
                                    run_id,
                                    json!({
                                        "dynamicDecision": {
                                            "action": "fallback",
                                            "delaySecs": delay_secs,
                                            "reason": "model did not provide a dynamic loop decision",
                                        }
                                    }),
                                    None,
                                )?;
                            }
                        }
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

    fn list_active_event_backed_loop_watches_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<(LoopSchedule, LoopWatch)>> {
        let schedules = self.list_loop_schedules_for_session(session_id, 200)?;
        let mut items = Vec::new();
        for schedule in schedules.into_iter().filter(|schedule| {
            schedule.state == LoopState::Active && schedule.trigger_kind == LoopTriggerKind::Dynamic
        }) {
            for watch in self
                .list_loop_watches(&schedule.id)?
                .into_iter()
                .filter(|watch| watch.active && watch.kind.is_event_backed())
            {
                items.push((schedule.clone(), watch));
            }
        }
        Ok(items)
    }

    fn mark_loop_watch_event(
        &self,
        watch_id: &str,
        fingerprint: &str,
        event_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_watches
             SET last_event_at = ?2,
                 last_fingerprint = ?3,
                 failure_count = 0,
                 last_error = NULL,
                 updated_at = ?2
             WHERE id = ?1 AND active = 1",
            params![watch_id, event_at, fingerprint],
        )?;
        Ok(())
    }

    fn record_loop_monitor_event(
        &self,
        watch_id: &str,
        outcome: &str,
        payload: Value,
    ) -> Result<Option<String>> {
        let Some(watch) = self.get_loop_watch(watch_id)? else {
            return Ok(None);
        };
        if !watch.active {
            return Ok(None);
        }
        let Some(schedule) = self.get_loop_schedule(&watch.loop_id)? else {
            return Ok(None);
        };
        if schedule.state != LoopState::Active {
            return Ok(None);
        }
        let now = Utc::now();
        let debounce_secs = watch
            .spec
            .get("debounceSecs")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .clamp(1, MAX_EVENT_DEBOUNCE_SECS);
        let bucket = now.timestamp().div_euclid(debounce_secs);
        let fingerprint_input = stable_json(&json!({
            "watchId": watch.id,
            "generation": watch.generation,
            "outcome": outcome,
            "payload": payload,
            "bucket": bucket,
        }))?;
        let fingerprint = blake3::hash(fingerprint_input.as_bytes())
            .to_hex()
            .to_string();
        let context = json!({
            "eventName": format!("loop:monitor:{}", watch.kind.as_str()),
            "sessionId": schedule.session_id,
            "watch": {
                "id": watch.id,
                "kind": watch.kind.as_str(),
                "generation": watch.generation,
            },
            "outcome": outcome,
            "untrusted": true,
            "payload": payload,
        });
        if !self.insert_loop_event_tick(
            &schedule.id,
            context["eventName"].as_str().unwrap_or("loop:monitor"),
            &fingerprint,
            &context,
            &now.to_rfc3339(),
        )? {
            return Ok(None);
        }
        self.mark_loop_watch_event(&watch.id, &fingerprint, &now.to_rfc3339())?;
        Ok(Some(schedule.cron_job_id))
    }

    pub(crate) fn record_loop_monitor_error(&self, watch_id: &str, error: &str) -> Result<()> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_watches
             SET failure_count = failure_count + 1,
                 last_error = ?2,
                 active = CASE WHEN failure_count + 1 >= ?3 THEN 0 ELSE active END,
                 monitor_job_id = NULL,
                 updated_at = ?4
             WHERE id = ?1",
            params![
                watch_id,
                truncate_utf8(error, 1000),
                LOOP_MONITOR_MAX_FAILURES,
                now,
            ],
        )?;
        Ok(())
    }

    fn settle_loop_monitor_watch(&self, watch_id: &str, generation: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_watches
             SET active = 0, monitor_job_id = NULL, updated_at = ?3
             WHERE id = ?1 AND generation = ?2",
            params![watch_id, generation, now_rfc3339()],
        )?;
        Ok(())
    }

    fn list_active_loop_monitor_watches(&self) -> Result<Vec<(LoopSchedule, LoopWatch)>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT w.id, w.loop_id, w.kind, w.spec_json, w.active, w.generation,
                    w.last_event_at, w.last_fingerprint, w.failure_count, w.last_error,
                    w.created_at, w.updated_at, w.monitor_job_id
             FROM loop_watches w
             JOIN loop_schedules s ON s.id = w.loop_id
             WHERE w.active = 1 AND s.state = 'active'
               AND w.kind IN ('file','websocket')
             ORDER BY w.updated_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_loop_watch)?;
        let watches = collect_rows(rows)?;
        drop(stmt);
        drop(conn);
        let mut items = Vec::new();
        for watch in watches {
            if let Some(schedule) = self.get_loop_schedule(&watch.loop_id)? {
                items.push((schedule, watch));
            }
        }
        Ok(items)
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
        let monitor_watches = if state != LoopState::Active {
            self.list_loop_watches(loop_id)?
                .into_iter()
                .filter(|watch| {
                    watch.active
                        && matches!(watch.kind, LoopWatchKind::File | LoopWatchKind::Websocket)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
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
            if state.is_terminal() {
                conn.execute(
                    "UPDATE loop_watches
                     SET active = 0, generation = generation + 1, updated_at = ?2
                     WHERE loop_id = ?1 AND active = 1",
                    params![loop_id, now],
                )?;
            } else if state != LoopState::Active {
                conn.execute(
                    "UPDATE loop_watches
                     SET monitor_job_id = NULL, updated_at = ?2
                     WHERE loop_id = ?1 AND active = 1
                       AND kind IN ('file','websocket')",
                    params![loop_id, now],
                )?;
            }
        }
        let schedule = self
            .get_loop_schedule(loop_id)?
            .ok_or_else(|| anyhow!("loop schedule not found: {loop_id}"))?;
        for watch in monitor_watches {
            stop_loop_monitor(&watch.id);
            if let Some(job_id) = watch.monitor_job_id.as_deref() {
                let reason = if state.is_terminal() {
                    "Owning Loop reached a terminal state"
                } else {
                    "Owning Loop is not active; monitor suspended"
                };
                let _ = crate::async_jobs::JobManager::finish_monitor(
                    job_id,
                    crate::async_jobs::JobStatus::Cancelled,
                    None,
                    Some(reason),
                );
            }
        }
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

    fn set_dynamic_loop_fallback_used(
        &self,
        loop_id: &str,
        current_spec: &Value,
        fallback_used: bool,
    ) -> Result<()> {
        let mut spec = current_spec.clone();
        if !spec.is_object() {
            spec = json!({});
        }
        if let Some(object) = spec.as_object_mut() {
            object.insert(
                "fallbackSecs".to_string(),
                json!(dynamic_loop_fallback_secs(current_spec)),
            );
            object.insert("fallbackUsed".to_string(), json!(fallback_used));
        }
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE loop_schedules
             SET trigger_spec_json = ?2, updated_at = ?3
             WHERE id = ?1",
            params![loop_id, stable_json(&spec)?, now],
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

    fn loop_run_dynamic_decision(&self, run_id: &str) -> Result<Option<DynamicLoopDecision>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let trace_json: Option<String> = conn
            .query_row(
                "SELECT trace_json FROM loop_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(trace_json) = trace_json else {
            return Ok(None);
        };
        let trace: Value = serde_json::from_str(&trace_json).unwrap_or_else(|_| json!({}));
        Ok(dynamic_loop_decision_from_trace(&trace))
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

pub(crate) fn cancel_loop_monitor_by_job_id(job_id: &str, watch_id: Option<&str>) -> Result<bool> {
    let result = crate::require_session_db()?.deactivate_loop_watch_for_monitor_job(job_id);
    if let Some(watch_id) = watch_id {
        stop_loop_monitor(watch_id);
    }
    result
}

fn loop_runtime_limit_secs(schedule: &LoopSchedule) -> Option<i64> {
    schedule.max_runtime_secs.or_else(|| {
        (schedule.trigger_kind == LoopTriggerKind::Dynamic)
            .then_some(DEFAULT_DYNAMIC_LOOP_MAX_RUNTIME_SECS)
    })
}

fn dynamic_loop_fallback_secs(spec: &Value) -> i64 {
    spec.get("fallbackSecs")
        .or_else(|| spec.get("fallback_secs"))
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_DYNAMIC_LOOP_FALLBACK_SECS)
        .clamp(
            MIN_DYNAMIC_LOOP_RESCHEDULE_SECS,
            MAX_DYNAMIC_LOOP_RESCHEDULE_SECS,
        )
}

fn dynamic_loop_fallback_used(spec: &Value) -> bool {
    spec.get("fallbackUsed")
        .or_else(|| spec.get("fallback_used"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn dynamic_loop_uses_maintenance_prompt(spec: &Value) -> bool {
    spec.get("maintenancePrompt")
        .or_else(|| spec.get("maintenance_prompt"))
        .and_then(|value| value.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn dynamic_loop_maintenance_prompt_metadata(spec: &Value) -> Option<Value> {
    normalize_maintenance_prompt_spec(
        spec.get("maintenancePrompt")
            .or_else(|| spec.get("maintenance_prompt")),
    )
}

fn normalize_maintenance_prompt_spec(value: Option<&Value>) -> Option<Value> {
    let spec = value?;
    let enabled = spec
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    let source = spec
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let mut metadata = json!({
        "enabled": true,
        "source": source,
    });
    if let Some(path) = spec
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata["path"] = json!(path);
    }
    if let Some(hash) = spec
        .get("contentHash")
        .or_else(|| spec.get("content_hash"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata["contentHash"] = json!(hash);
    }
    Some(metadata)
}

fn loop_prompt_candidates(session_db: &SessionDB, sid: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(Some(meta)) = session_db.get_session(sid) {
        if let Some(working_dir) = crate::session::effective_working_dir_for_meta(&meta) {
            roots.push(PathBuf::from(working_dir));
        }
    }
    if let Ok(home) = crate::paths::home_dir() {
        roots.push(home);
    }

    let mut candidates = Vec::new();
    for root in roots {
        candidates.push(root.join("loop.md"));
        candidates.push(root.join(".hope").join("loop.md"));
        candidates.push(root.join(".hope-agent").join("loop.md"));
        candidates.push(root.join(".claude").join("loop.md"));
    }
    candidates
}

fn read_loop_prompt_file(path: &Path) -> Option<DefaultLoopPromptResolution> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((LOOP_MD_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    let truncated = bytes.len() > LOOP_MD_MAX_BYTES;
    if truncated {
        bytes.truncate(LOOP_MD_MAX_BYTES);
    }
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let truncation_note = if truncated {
        "\n\n[The loop.md file was truncated to the first 25KB.]"
    } else {
        ""
    };
    let prompt = format!(
        "Use the following loop.md instructions from `{}` as the standing prompt for this self-paced loop.\n\n{}{}",
        path.display(),
        trimmed,
        truncation_note
    );
    Some(DefaultLoopPromptResolution {
        metadata: json!({
            "enabled": true,
            "source": "loop_md",
            "path": path.to_string_lossy(),
            "contentHash": hash_text(trimmed),
        }),
        prompt,
    })
}

fn hash_text(input: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn dynamic_loop_decision(summary: Option<&str>) -> DynamicLoopDecision {
    let Some(summary) = summary else {
        return DynamicLoopDecision::Missing;
    };
    for line in summary.lines() {
        let line = line.trim();
        if let Some((_, rest)) = line.split_once("LOOP_RESCHEDULE_AFTER:") {
            if let Some((delay_secs, reason)) = parse_dynamic_reschedule(rest) {
                return DynamicLoopDecision::Reschedule { delay_secs, reason };
            }
            return DynamicLoopDecision::Missing;
        }
        if let Some((_, rest)) = line.split_once("LOOP_STOP:") {
            return DynamicLoopDecision::Stop {
                reason: non_empty(rest.trim())
                    .unwrap_or("model stopped the dynamic loop")
                    .to_string(),
            };
        }
        if let Some((_, rest)) = line.split_once("LOOP_BLOCKED:") {
            return DynamicLoopDecision::Block {
                reason: non_empty(rest.trim())
                    .unwrap_or("dynamic loop is blocked")
                    .to_string(),
            };
        }
    }
    DynamicLoopDecision::Missing
}

fn dynamic_loop_decision_from_trace(trace: &Value) -> Option<DynamicLoopDecision> {
    let decision = trace.get("dynamicDecision")?;
    let action = decision.get("action").and_then(Value::as_str)?;
    match action {
        "reschedule" => {
            let delay_secs = decision
                .get("delaySecs")
                .or_else(|| decision.get("delay_secs"))
                .and_then(Value::as_i64)?
                .clamp(
                    MIN_DYNAMIC_LOOP_RESCHEDULE_SECS,
                    MAX_DYNAMIC_LOOP_RESCHEDULE_SECS,
                );
            let reason = decision
                .get("reason")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .unwrap_or("model requested dynamic reschedule")
                .to_string();
            Some(DynamicLoopDecision::Reschedule { delay_secs, reason })
        }
        "stop" => {
            let reason = decision
                .get("reason")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .unwrap_or("model stopped the dynamic loop")
                .to_string();
            Some(DynamicLoopDecision::Stop { reason })
        }
        "block" => {
            let reason = decision
                .get("reason")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .unwrap_or("dynamic loop is blocked")
                .to_string();
            Some(DynamicLoopDecision::Block { reason })
        }
        _ => None,
    }
}

fn parse_dynamic_reschedule(input: &str) -> Option<(i64, String)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let first = parts.next()?;
    let first_clean = first.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '.'));
    let (raw_delay, consumed_len) = if let Some(secs) = parse_loop_duration_secs(first_clean) {
        (secs, first.len())
    } else {
        let second = parts.next()?;
        let second_clean = second.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '.'));
        let secs = parse_loop_duration_secs(&format!("{first_clean}{second_clean}"))?;
        (secs, first.len() + 1 + second.len())
    };
    let delay_secs = raw_delay.clamp(
        MIN_DYNAMIC_LOOP_RESCHEDULE_SECS,
        MAX_DYNAMIC_LOOP_RESCHEDULE_SECS,
    );
    let reason = trimmed
        .get(consumed_len..)
        .unwrap_or("")
        .trim()
        .trim_start_matches(['-', ':', ',', ';'])
        .trim();
    Some((
        delay_secs,
        non_empty(reason)
            .unwrap_or("model requested dynamic reschedule")
            .to_string(),
    ))
}

fn parse_loop_duration_secs(input: &str) -> Option<i64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let split = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (num, unit) = trimmed.split_at(split);
    let n = num.parse::<i64>().ok()?;
    let multiplier = match unit {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    };
    Some(n.saturating_mul(multiplier))
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
        LoopTriggerKind::Dynamic => {
            let secs = dynamic_loop_fallback_secs(&input.trigger_spec);
            Ok(CronSchedule::Every {
                interval_ms: (secs as u64).saturating_mul(1000),
                start_at: None,
            })
        }
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
        LoopTriggerKind::Dynamic => {
            let mut normalized = json!({
                "fallbackSecs": dynamic_loop_fallback_secs(spec),
                "fallbackUsed": spec
                    .get("fallbackUsed")
                    .or_else(|| spec.get("fallback_used"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
            if let Some(maintenance) = normalize_maintenance_prompt_spec(
                spec.get("maintenancePrompt")
                    .or_else(|| spec.get("maintenance_prompt")),
            ) {
                normalized["maintenancePrompt"] = maintenance;
            }
            Ok(normalized)
        }
        LoopTriggerKind::Cron => Ok(spec.clone()),
    }
}

fn normalize_loop_watch_spec(kind: LoopWatchKind, spec: &Value) -> Result<Value> {
    let default_event_name = match kind {
        LoopWatchKind::Job => Some("job:completed"),
        LoopWatchKind::Subagent => Some("subagent_event"),
        LoopWatchKind::AppEvent => None,
        LoopWatchKind::Command => Some("job:completed"),
        LoopWatchKind::File => {
            let path = json_string(spec, "path")
                .ok_or_else(|| anyhow!("file watch requires spec.path"))?;
            let recursive = spec
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let debounce_secs = spec
                .get("debounceSecs")
                .or_else(|| spec.get("debounce_secs"))
                .and_then(Value::as_i64)
                .unwrap_or(2)
                .clamp(1, MAX_EVENT_DEBOUNCE_SECS);
            return Ok(json!({
                "path": path,
                "recursive": recursive,
                "debounceSecs": debounce_secs,
            }));
        }
        LoopWatchKind::Websocket => {
            let url = json_string(spec, "url")
                .ok_or_else(|| anyhow!("websocket watch requires spec.url"))?;
            let parsed =
                url::Url::parse(url).map_err(|err| anyhow!("invalid websocket URL: {err}"))?;
            let scheme = parsed.scheme().to_string();
            if !matches!(scheme.as_str(), "ws" | "wss") {
                return Err(anyhow!("websocket watch URL must use ws:// or wss://"));
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(anyhow!(
                    "websocket watch URL must not contain persisted credentials"
                ));
            }
            const SENSITIVE_QUERY_KEYS: &[&str] = &[
                "access_token",
                "api_key",
                "apikey",
                "auth",
                "authorization",
                "key",
                "password",
                "secret",
                "sig",
                "signature",
                "token",
            ];
            if parsed
                .query_pairs()
                .any(|(key, _)| SENSITIVE_QUERY_KEYS.contains(&key.to_ascii_lowercase().as_str()))
            {
                return Err(anyhow!(
                    "websocket watch URL must not contain secrets in persisted query parameters"
                ));
            }
            let timeout_secs = spec
                .get("timeoutSecs")
                .or_else(|| spec.get("timeout_secs"))
                .and_then(Value::as_i64)
                .unwrap_or(DEFAULT_DYNAMIC_LOOP_FALLBACK_SECS)
                .clamp(30, 24 * 60 * 60);
            let match_text = json_string(spec, "matchText").map(str::to_string);
            return Ok(json!({
                "url": url,
                "timeoutSecs": timeout_secs,
                "matchText": match_text,
            }));
        }
    };
    let event_name = spec
        .get("eventName")
        .or_else(|| spec.get("event_name"))
        .or_else(|| spec.get("event"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(default_event_name)
        .ok_or_else(|| anyhow!("app_event watch requires spec.eventName"))?;
    let valid_for_kind = match kind {
        LoopWatchKind::AppEvent => is_supported_loop_event_name(event_name),
        LoopWatchKind::Job => event_name.starts_with("job:"),
        LoopWatchKind::Subagent => event_name == "subagent_event",
        LoopWatchKind::Command => event_name == "job:completed",
        LoopWatchKind::File | LoopWatchKind::Websocket => false,
    };
    if !valid_for_kind || !is_supported_loop_event_name(event_name) {
        return Err(anyhow!(
            "unsupported {} loop watch event: {event_name}",
            kind.as_str()
        ));
    }
    let filters = spec.get("filters").cloned().unwrap_or_else(|| json!({}));
    if kind == LoopWatchKind::Command
        && filters
            .get("jobId")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(anyhow!(
            "command watch requires spec.filters.jobId from an existing permission-checked background exec job"
        ));
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

fn ensure_loop_watch_capacity(
    conn: &Connection,
    schedule: &LoopSchedule,
    kind: LoopWatchKind,
) -> Result<()> {
    let loop_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM loop_watches WHERE loop_id = ?1 AND active = 1",
        params![schedule.id],
        |row| row.get(0),
    )?;
    if loop_count >= MAX_ACTIVE_WATCHES_PER_LOOP {
        return Err(anyhow!(
            "loop watch limit reached: at most {MAX_ACTIVE_WATCHES_PER_LOOP} active watches per Loop"
        ));
    }
    if !matches!(kind, LoopWatchKind::File | LoopWatchKind::Websocket) {
        return Ok(());
    }

    let session_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM loop_watches w
         JOIN loop_schedules s ON s.id = w.loop_id
         WHERE s.session_id = ?1 AND w.active = 1
           AND w.kind IN ('file','websocket')",
        params![schedule.session_id],
        |row| row.get(0),
    )?;
    if session_count >= MAX_ACTIVE_MONITORS_PER_SESSION {
        return Err(anyhow!(
            "loop monitor limit reached: at most {MAX_ACTIVE_MONITORS_PER_SESSION} active file/WebSocket monitors per session"
        ));
    }
    let global_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM loop_watches
         WHERE active = 1 AND kind IN ('file','websocket')",
        [],
        |row| row.get(0),
    )?;
    if global_count >= MAX_ACTIVE_MONITORS_GLOBAL {
        return Err(anyhow!(
            "global loop monitor limit reached: at most {MAX_ACTIVE_MONITORS_GLOBAL} active file/WebSocket monitors"
        ));
    }
    Ok(())
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

fn row_to_loop_watch(row: &rusqlite::Row<'_>) -> rusqlite::Result<LoopWatch> {
    let kind: String = row.get(2)?;
    let spec_json: String = row.get(3)?;
    Ok(LoopWatch {
        id: row.get(0)?,
        loop_id: row.get(1)?,
        kind: LoopWatchKind::from_str(&kind).unwrap_or(LoopWatchKind::AppEvent),
        spec: serde_json::from_str(&spec_json).unwrap_or_else(|_| json!({})),
        active: row.get::<_, i64>(4)? != 0,
        generation: row.get(5)?,
        last_event_at: row.get(6)?,
        last_fingerprint: row.get(7)?,
        failure_count: row.get(8)?,
        last_error: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        monitor_job_id: row.get(12)?,
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
        usage: empty_loop_run_usage_snapshot("not_loaded"),
        started_at: row.get(15)?,
        finished_at: row.get(16)?,
    })
}

fn empty_loop_run_usage_snapshot(attribution: &str) -> LoopRunUsageSnapshot {
    LoopRunUsageSnapshot {
        message_count: 0,
        user_turns: 0,
        assistant_messages: 0,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        attribution: attribution.to_string(),
        provider_events: 0,
        provider_input_tokens: 0,
        provider_output_tokens: 0,
        provider_cache_creation_input_tokens: 0,
        provider_cache_read_input_tokens: 0,
        provider_total_tokens: 0,
        provider_attribution: "not_loaded".to_string(),
    }
}

fn loop_run_usage_snapshot_with_conn(
    conn: &Connection,
    run: &LoopRun,
) -> Result<LoopRunUsageSnapshot> {
    if let Some(snapshot) = loop_run_usage_from_trigger_message(conn, run)? {
        return Ok(snapshot);
    }

    let window_end = run
        .finished_at
        .clone()
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let lower_bound = i64::MIN;
    let upper_bound = i64::MAX;
    let mut snapshot = conn.query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN role = 'assistant' THEN 1 ELSE 0 END), 0),
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
           AND timestamp >= ?2
           AND timestamp <= ?3
           AND role IN (?4, ?5)",
        params![
            &run.session_id,
            &run.started_at,
            window_end,
            MessageRole::User.as_str(),
            MessageRole::Assistant.as_str(),
        ],
        |row| {
            let input_tokens = row.get::<_, i64>(3)?;
            let output_tokens = row.get::<_, i64>(4)?;
            Ok(LoopRunUsageSnapshot {
                message_count: row.get(0)?,
                user_turns: row.get(1)?,
                assistant_messages: row.get(2)?,
                input_tokens,
                output_tokens,
                total_tokens: input_tokens.saturating_add(output_tokens),
                attribution: "session_messages_between_loop_run_bounds".to_string(),
                provider_events: 0,
                provider_input_tokens: 0,
                provider_output_tokens: 0,
                provider_cache_creation_input_tokens: 0,
                provider_cache_read_input_tokens: 0,
                provider_total_tokens: 0,
                provider_attribution: "not_loaded".to_string(),
            })
        },
    )?;
    hydrate_loop_provider_usage_for_timestamp_range(
        conn,
        &run.session_id,
        &run.started_at,
        &window_end,
        lower_bound,
        upper_bound,
        &mut snapshot,
    )?;
    if run.finished_at.is_none() {
        snapshot.attribution = "session_messages_since_loop_run_start".to_string();
    }
    Ok(snapshot)
}

fn loop_run_usage_from_trigger_message(
    conn: &Connection,
    run: &LoopRun,
) -> Result<Option<LoopRunUsageSnapshot>> {
    let pattern = format!("%\"run_id\":\"{}\"%", run.id);
    let Some(trigger_message_id) = conn
        .query_row(
            "SELECT id
             FROM messages
             WHERE session_id = ?1
               AND role = ?2
               AND attachments_meta LIKE ?3
             ORDER BY id ASC
             LIMIT 1",
            params![&run.session_id, MessageRole::User.as_str(), pattern],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
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
                trigger_message_id
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let upper_bound = next_user_message_id.unwrap_or(i64::MAX);
    let mut snapshot = conn.query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN role = 'assistant' THEN 1 ELSE 0 END), 0),
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
            let input_tokens = row.get::<_, i64>(3)?;
            let output_tokens = row.get::<_, i64>(4)?;
            Ok(LoopRunUsageSnapshot {
                message_count: row.get(0)?,
                user_turns: row.get(1)?,
                assistant_messages: row.get(2)?,
                input_tokens,
                output_tokens,
                total_tokens: input_tokens.saturating_add(output_tokens),
                attribution: "loop_trigger_message_boundary".to_string(),
                provider_events: 0,
                provider_input_tokens: 0,
                provider_output_tokens: 0,
                provider_cache_creation_input_tokens: 0,
                provider_cache_read_input_tokens: 0,
                provider_total_tokens: 0,
                provider_attribution: "not_loaded".to_string(),
            })
        },
    )?;
    hydrate_loop_provider_usage_for_message_id_range(
        conn,
        &run.session_id,
        trigger_message_id,
        upper_bound,
        &mut snapshot,
    )?;
    Ok(Some(snapshot))
}

fn hydrate_loop_provider_usage_for_message_id_range(
    conn: &Connection,
    session_id: &str,
    lower_bound: i64,
    upper_bound: i64,
    snapshot: &mut LoopRunUsageSnapshot,
) -> Result<()> {
    hydrate_loop_provider_usage_with_where(
        conn,
        session_id,
        "m.id >= ?2 AND m.id < ?3",
        params![
            session_id,
            lower_bound,
            upper_bound,
            crate::model_usage::KIND_CHAT,
            MessageRole::Assistant.as_str(),
        ],
        snapshot,
    )
}

fn hydrate_loop_provider_usage_for_timestamp_range(
    conn: &Connection,
    session_id: &str,
    started_at: &str,
    window_end: &str,
    _lower_bound: i64,
    _upper_bound: i64,
    snapshot: &mut LoopRunUsageSnapshot,
) -> Result<()> {
    hydrate_loop_provider_usage_with_where(
        conn,
        session_id,
        "m.timestamp >= ?2 AND m.timestamp <= ?3",
        params![
            session_id,
            started_at,
            window_end,
            crate::model_usage::KIND_CHAT,
            MessageRole::Assistant.as_str(),
        ],
        snapshot,
    )
}

fn hydrate_loop_provider_usage_with_where<P>(
    conn: &Connection,
    session_id: &str,
    message_predicate: &str,
    params: P,
    snapshot: &mut LoopRunUsageSnapshot,
) -> Result<()>
where
    P: rusqlite::Params,
{
    let sql = format!(
        "SELECT
            COUNT(u.id),
            COALESCE(SUM(u.input_tokens), 0),
            COALESCE(SUM(u.output_tokens), 0),
            COALESCE(SUM(u.cache_creation_input_tokens), 0),
            COALESCE(SUM(u.cache_read_input_tokens), 0)
         FROM messages m
         JOIN model_usage_events u ON u.request_key = ('message:' || m.id)
         WHERE m.session_id = ?1
           AND {message_predicate}
           AND u.kind = ?4
           AND m.role = ?5",
    );
    let provider = conn.query_row(&sql, params, |row| {
        let input_tokens = row.get::<_, i64>(1)?;
        let output_tokens = row.get::<_, i64>(2)?;
        Ok((
            row.get::<_, i64>(0)?,
            input_tokens,
            output_tokens,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            input_tokens.saturating_add(output_tokens),
        ))
    })?;
    snapshot.provider_events = provider.0;
    snapshot.provider_input_tokens = provider.1;
    snapshot.provider_output_tokens = provider.2;
    snapshot.provider_cache_creation_input_tokens = provider.3;
    snapshot.provider_cache_read_input_tokens = provider.4;
    snapshot.provider_total_tokens = provider.5;
    snapshot.provider_attribution = if snapshot.provider_events > 0 {
        "model_usage_events.request_key=message_id".to_string()
    } else {
        format!("no_linked_model_usage_events_for_session:{session_id}")
    };
    Ok(())
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
        "workflow:created"
            | "workflow:updated"
            | "workflow:op_updated"
            | "goal:updated"
            | "task_updated"
            | "job:created"
            | "job:updated"
            | "job:progress"
            | "job:completed"
            | "subagent_event"
    )
}

fn is_loop_trigger_event_name(event_name: &str) -> bool {
    is_supported_loop_event_name(event_name)
}

fn validate_loop_event_filters(event_name: &str, filters: &Value) -> Result<()> {
    let allowed = match event_name {
        "workflow:created" | "workflow:updated" => &["workflowState", "workflowId"][..],
        "workflow:op_updated" => &["workflowId", "opState", "opKind"][..],
        "goal:updated" => &["goalState"][..],
        "task_updated" => &["taskStatus"][..],
        "job:created" | "job:updated" | "job:progress" | "job:completed" => {
            &["jobId", "jobKind", "tool", "jobStatus"][..]
        }
        "subagent_event" => &["eventType", "runId", "agentId", "subagentStatus"][..],
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
        .or_else(|| event.payload.get("parentSessionId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn loop_event_matches_schedule(
    schedule: &LoopSchedule,
    event: &AppEvent,
    now: &DateTime<Utc>,
) -> Result<Option<LoopEventMatch>> {
    loop_event_matches_spec(schedule, &schedule.trigger_spec, event, now)
}

fn loop_event_matches_spec(
    schedule: &LoopSchedule,
    spec: &Value,
    event: &AppEvent,
    now: &DateTime<Utc>,
) -> Result<Option<LoopEventMatch>> {
    let event_name = json_string(spec, "eventName")
        .or_else(|| json_string(spec, "event_name"))
        .or_else(|| json_string(spec, "event"));
    if event_name != Some(event.name.as_str()) {
        return Ok(None);
    }
    let filters = spec.get("filters").cloned().unwrap_or_else(|| json!({}));
    validate_loop_event_filters(&event.name, &filters)?;
    let Some(session_id) = loop_event_session_id(event) else {
        return Ok(None);
    };
    if session_id != schedule.session_id {
        return Ok(None);
    }
    let identity = match event.name.as_str() {
        "workflow:created" | "workflow:updated" => {
            let state = json_string(&event.payload, "state").unwrap_or("");
            if !loop_filter_matches(&filters, "workflowState", state) {
                return Ok(None);
            }
            let workflow_id = json_string(&event.payload, "id").unwrap_or("");
            if !loop_filter_matches(&filters, "workflowId", workflow_id) {
                return Ok(None);
            }
            json!({
                "id": workflow_id,
                "state": state,
                "kind": json_string(&event.payload, "kind"),
                "goalId": json_string(&event.payload, "goalId"),
            })
        }
        "workflow:op_updated" => {
            let workflow_id = json_string(&event.payload, "runId")
                .or_else(|| json_string(&event.payload, "workflowRunId"))
                .unwrap_or("");
            let state = json_string(&event.payload, "state").unwrap_or("");
            let kind = json_string(&event.payload, "kind").unwrap_or("");
            if !loop_filter_matches(&filters, "workflowId", workflow_id)
                || !loop_filter_matches(&filters, "opState", state)
                || !loop_filter_matches(&filters, "opKind", kind)
            {
                return Ok(None);
            }
            json!({ "workflowId": workflow_id, "state": state, "kind": kind })
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
        "job:created" | "job:updated" | "job:progress" | "job:completed" => {
            let job_id = json_string(&event.payload, "job_id")
                .or_else(|| json_string(&event.payload, "jobId"))
                .unwrap_or("");
            let kind = json_string(&event.payload, "kind").unwrap_or("");
            let tool = json_string(&event.payload, "tool").unwrap_or("");
            let status = json_string(&event.payload, "status").unwrap_or("");
            if !loop_filter_matches(&filters, "jobId", job_id)
                || !loop_filter_matches(&filters, "jobKind", kind)
                || !loop_filter_matches(&filters, "tool", tool)
                || !loop_filter_matches(&filters, "jobStatus", status)
            {
                return Ok(None);
            }
            json!({ "jobId": job_id, "kind": kind, "tool": tool, "status": status })
        }
        "subagent_event" => {
            let event_type = json_string(&event.payload, "eventType").unwrap_or("");
            let run_id = json_string(&event.payload, "runId").unwrap_or("");
            let agent_id = json_string(&event.payload, "childAgentId").unwrap_or("");
            let status = json_string(&event.payload, "status").unwrap_or("");
            if !loop_filter_matches(&filters, "eventType", event_type)
                || !loop_filter_matches(&filters, "runId", run_id)
                || !loop_filter_matches(&filters, "agentId", agent_id)
                || !loop_filter_matches(&filters, "subagentStatus", status)
            {
                return Ok(None);
            }
            json!({
                "eventType": event_type,
                "runId": run_id,
                "agentId": agent_id,
                "status": status,
            })
        }
        _ => return Ok(None),
    };
    let debounce_secs = spec
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
    let dynamic = if trigger_kind == LoopTriggerKind::Dynamic {
        let fallback_secs = dynamic_loop_fallback_secs(trigger_spec);
        format!(
            "This is a dynamic self-paced loop. At the end of this turn, include exactly one \
             final decision line: `LOOP_RESCHEDULE_AFTER: <duration> - <reason>` to continue, \
             `LOOP_STOP: <reason>` when the recurring objective is complete or no longer useful, \
             or `LOOP_BLOCKED: <reason>` when user input or an external state change is required. \
             Choose a duration between 1 minute and 1 hour. If no decision line appears, the \
             system will schedule one fallback wakeup after {} and then block if the fallback \
             turn also has no decision. When progress depends on a known Workflow, Job, subagent, \
             file, or WebSocket event, prefer `loop_watch` plus this fallback instead of frequent \
             polling. Re-arm a one-shot monitor after each observed event when continued watching \
             is still useful; use `loop_unwatch` when it is no longer relevant.\n",
            format_loop_duration(fallback_secs)
        )
    } else {
        String::new()
    };
    format!(
        "<loop_trigger>\n\
         <loop_id>{}</loop_id>\n\
         <run_id>{}</run_id>\n\
         {}\
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
        dynamic,
        escape_xml(prompt)
    )
}

fn condition_satisfied_marker(summary: Option<&str>) -> bool {
    summary
        .map(|s| s.contains("LOOP_CONDITION_SATISFIED:"))
        .unwrap_or(false)
}

fn format_loop_duration(secs: i64) -> String {
    if secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
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

fn emit_loop_watch_event(event: &str, schedule: &LoopSchedule, watch: &LoopWatch) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            event,
            json!({
                "loopId": schedule.id,
                "sessionId": schedule.session_id,
                "goalId": schedule.goal_id,
                "watchId": watch.id,
                "kind": watch.kind,
                "active": watch.active,
                "generation": watch.generation,
            }),
        );
    }
}

fn spawn_loop_monitor_run(
    session_db: Arc<SessionDB>,
    cron_db: Arc<CronDB>,
    watch_id: String,
    outcome: &'static str,
    payload: Value,
) -> bool {
    let cron_job_id = match session_db.record_loop_monitor_event(&watch_id, outcome, payload) {
        Ok(Some(id)) => id,
        Ok(None) => return false,
        Err(err) => {
            let _ = session_db.record_loop_monitor_error(&watch_id, &err.to_string());
            app_warn!(
                "loop",
                "monitor",
                "failed to record monitor event for {}: {}",
                watch_id,
                err
            );
            return false;
        }
    };
    let job = match cron_db.get_job(&cron_job_id) {
        Ok(Some(job)) => job,
        Ok(None) => {
            app_warn!(
                "loop",
                "monitor",
                "monitor event for {} was recorded but cron job {} is missing",
                watch_id,
                cron_job_id
            );
            return true;
        }
        Err(err) => {
            app_warn!(
                "loop",
                "monitor",
                "failed to load monitor cron job {}: {}",
                cron_job_id,
                err
            );
            return true;
        }
    };
    crate::cron::spawn_job_execution(cron_db, session_db, job);
    true
}

fn start_file_loop_monitor(
    session_db: Arc<SessionDB>,
    cron_db: Arc<CronDB>,
    watch: &LoopWatch,
    monitor_job_id: Option<String>,
) -> Result<()> {
    use notify::Watcher as _;

    let path = watch
        .spec
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("file watch path is missing"))?;
    if !path.exists() {
        return Err(anyhow!(
            "file watch path does not exist: {}",
            path.display()
        ));
    }
    let recursive = watch
        .spec
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let watch_id = watch.id.clone();
    let generation = watch.generation;
    let callback_watch_id = watch_id.clone();
    let callback_db = session_db.clone();
    let callback_cron = cron_db.clone();
    let callback_monitor_job_id = monitor_job_id.clone();
    // The body runs directly on notify's watcher thread: a dedicated OS thread
    // (blocking-work exempt), and — critically — notify delivers a watcher's
    // events sequentially, which serializes the one-shot record→settle
    // check-then-act sequence. Dispatching the body to a pool would let one
    // fs write's multiple notify events (Create + Modify…) race past the
    // `active` check concurrently and emit several durable ticks. Nothing
    // here needs a runtime: the DB calls are sync and `spawn_job_execution`
    // carries its own dedicated runtime handle.
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let session_db = callback_db.clone();
        let cron_db = callback_cron.clone();
        let watch_id = callback_watch_id.clone();
        let monitor_job_id = callback_monitor_job_id.clone();
        match result {
            Ok(event) => {
                let paths: Vec<String> = event
                    .paths
                    .iter()
                    .take(16)
                    .map(|path| truncate_utf8(&path.to_string_lossy(), 500).to_string())
                    .collect();
                let payload = json!({
                    "kind": format!("{:?}", event.kind),
                    "paths": paths,
                    "truncated": event.paths.len() > 16,
                });
                if spawn_loop_monitor_run(
                    session_db.clone(),
                    cron_db,
                    watch_id.clone(),
                    "changed",
                    payload,
                ) {
                    // Terminalize the monitor job BEFORE settling the watch
                    // (same order as the websocket monitor): observers treat
                    // `active == false` as "the monitor job is terminal", and
                    // this thread runs concurrently with them.
                    if let Some(job_id) = monitor_job_id.as_deref() {
                        let _ = crate::async_jobs::JobManager::finish_monitor(
                            job_id,
                            crate::async_jobs::JobStatus::Completed,
                            Some("changed"),
                            None,
                        );
                    }
                    let _ = session_db.settle_loop_monitor_watch(&watch_id, generation);
                    remove_loop_monitor_generation(&watch_id, generation);
                }
            }
            Err(err) => {
                let message = err.to_string();
                let _ = session_db.record_loop_monitor_error(&watch_id, &message);
                if spawn_loop_monitor_run(
                    session_db.clone(),
                    cron_db,
                    watch_id.clone(),
                    "failed",
                    json!({ "error": truncate_utf8(&message, 1000) }),
                ) {
                    if let Some(job_id) = monitor_job_id.as_deref() {
                        let _ = crate::async_jobs::JobManager::finish_monitor(
                            job_id,
                            crate::async_jobs::JobStatus::Failed,
                            None,
                            Some(&message),
                        );
                    }
                    let _ = session_db.settle_loop_monitor_watch(&watch_id, generation);
                    remove_loop_monitor_generation(&watch_id, generation);
                }
            }
        }
    })?;
    watcher.watch(
        &path,
        if recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        },
    )?;
    stop_loop_monitor(&watch_id);
    loop_monitors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            watch_id,
            LoopMonitorHandle::File {
                generation,
                _watcher: watcher,
            },
        );
    Ok(())
}

async fn validate_loop_websocket_url(raw_url: &str) -> Result<()> {
    let mut policy_url = url::Url::parse(raw_url)?;
    let replacement = if policy_url.scheme() == "wss" {
        "https"
    } else {
        "http"
    };
    policy_url
        .set_scheme(replacement)
        .map_err(|_| anyhow!("invalid websocket URL scheme"))?;
    let ssrf = crate::config::cached_config().ssrf.clone();
    crate::security::ssrf::check_url(
        policy_url.as_str(),
        ssrf.default_policy,
        &ssrf.trusted_hosts,
    )
    .await?;
    Ok(())
}

async fn start_websocket_loop_monitor(
    session_db: Arc<SessionDB>,
    cron_db: Arc<CronDB>,
    watch: &LoopWatch,
    monitor_job_id: Option<String>,
) -> Result<()> {
    use futures_util::StreamExt as _;

    let raw_url = watch
        .spec
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("websocket watch URL is missing"))?;
    validate_loop_websocket_url(raw_url).await?;
    let timeout_secs = watch
        .spec
        .get("timeoutSecs")
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_DYNAMIC_LOOP_FALLBACK_SECS)
        .clamp(30, 24 * 60 * 60) as u64;
    let match_text = watch
        .spec
        .get("matchText")
        .and_then(Value::as_str)
        .map(str::to_string);
    let watch_id = watch.id.clone();
    let generation = watch.generation;
    let url = raw_url.to_string();
    let cancel = CancellationToken::new();
    stop_loop_monitor(&watch_id);
    loop_monitors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            watch_id.clone(),
            LoopMonitorHandle::Async {
                generation,
                cancel: cancel.clone(),
            },
        );
    tokio::spawn(async move {
        let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            let (mut stream, _) = tokio_tungstenite::connect_async(&url).await?;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return Ok::<Option<(String, Value)>, anyhow::Error>(None),
                    message = stream.next() => {
                        let Some(message) = message else {
                            return Ok(Some(("closed".to_string(), json!({ "reason": "stream_ended" }))));
                        };
                        let message = message?;
                        if message.is_close() {
                            return Ok(Some(("closed".to_string(), json!({ "reason": "close_frame" }))));
                        }
                        let preview = if message.is_text() {
                            message.into_text()?.to_string()
                        } else if message.is_binary() {
                            format!("<binary:{} bytes>", message.len())
                        } else {
                            continue;
                        };
                        if match_text.as_deref().is_some_and(|needle| !preview.contains(needle)) {
                            continue;
                        }
                        return Ok(Some(("message".to_string(), json!({
                            "preview": truncate_utf8(&preview, LOOP_MONITOR_PAYLOAD_MAX_BYTES),
                            "truncated": preview.len() > LOOP_MONITOR_PAYLOAD_MAX_BYTES,
                        }))));
                    }
                }
            }
        }).await;
        crate::blocking::run_blocking(move || {
            match result {
                Ok(Ok(Some((outcome, payload)))) => {
                    let outcome: &'static str = match outcome.as_str() {
                        "message" => "message",
                        _ => "closed",
                    };
                    spawn_loop_monitor_run(
                        session_db.clone(),
                        cron_db.clone(),
                        watch_id.clone(),
                        outcome,
                        payload,
                    );
                    if let Some(job_id) = monitor_job_id.as_deref() {
                        let _ = crate::async_jobs::JobManager::finish_monitor(
                            job_id,
                            crate::async_jobs::JobStatus::Completed,
                            Some(outcome),
                            None,
                        );
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(err)) => {
                    let message = err.to_string();
                    let _ = session_db.record_loop_monitor_error(&watch_id, &message);
                    spawn_loop_monitor_run(
                        session_db.clone(),
                        cron_db.clone(),
                        watch_id.clone(),
                        "failed",
                        json!({ "error": truncate_utf8(&message, 1000) }),
                    );
                    if let Some(job_id) = monitor_job_id.as_deref() {
                        let _ = crate::async_jobs::JobManager::finish_monitor(
                            job_id,
                            crate::async_jobs::JobStatus::Failed,
                            None,
                            Some(&message),
                        );
                    }
                }
                Err(_) => {
                    let message = format!("websocket monitor timed out after {timeout_secs}s");
                    let _ = session_db.record_loop_monitor_error(&watch_id, &message);
                    spawn_loop_monitor_run(
                        session_db.clone(),
                        cron_db.clone(),
                        watch_id.clone(),
                        "timed_out",
                        json!({ "timeoutSecs": timeout_secs }),
                    );
                    if let Some(job_id) = monitor_job_id.as_deref() {
                        let _ = crate::async_jobs::JobManager::finish_monitor(
                            job_id,
                            crate::async_jobs::JobStatus::TimedOut,
                            None,
                            Some(&message),
                        );
                    }
                }
            }
            let _ = session_db.settle_loop_monitor_watch(&watch_id, generation);
            remove_loop_monitor_generation(&watch_id, generation);
        })
        .await;
    });
    Ok(())
}

pub(crate) async fn start_loop_monitor_adapter(
    session_db: Arc<SessionDB>,
    cron_db: Arc<CronDB>,
    watch: &LoopWatch,
) -> Result<()> {
    if !matches!(watch.kind, LoopWatchKind::File | LoopWatchKind::Websocket) {
        return Ok(());
    }
    let monitor_job_id = {
        let watch = watch.clone();
        session_db
            .run(move |db| -> Result<Option<String>> {
                let schedule = db
                    .get_loop_schedule(&watch.loop_id)?
                    .ok_or_else(|| anyhow!("loop schedule not found: {}", watch.loop_id))?;
                let monitor_job_id = crate::async_jobs::JobManager::register_monitor(
                    &schedule.session_id,
                    &watch.id,
                    watch.kind.as_str(),
                    &watch.spec,
                )?;
                if let Some(job_id) = monitor_job_id.as_deref() {
                    db.bind_loop_watch_monitor_job(&watch.id, job_id)?;
                }
                Ok(monitor_job_id)
            })
            .await?
    };
    let result = match watch.kind {
        LoopWatchKind::File => {
            let watch = watch.clone();
            let monitor_job_id = monitor_job_id.clone();
            crate::blocking::run_blocking(move || {
                start_file_loop_monitor(session_db, cron_db, &watch, monitor_job_id)
            })
            .await
        }
        LoopWatchKind::Websocket => {
            start_websocket_loop_monitor(session_db, cron_db, watch, monitor_job_id.clone()).await
        }
        LoopWatchKind::AppEvent
        | LoopWatchKind::Job
        | LoopWatchKind::Subagent
        | LoopWatchKind::Command => unreachable!(),
    };
    if let Err(err) = result.as_ref() {
        if let Some(job_id) = monitor_job_id.as_deref() {
            let _ = crate::async_jobs::JobManager::finish_monitor(
                job_id,
                crate::async_jobs::JobStatus::Failed,
                None,
                Some(&err.to_string()),
            );
        }
    }
    result
}

fn spawn_loop_monitor_recovery(session_db: Arc<SessionDB>, cron_db: Arc<CronDB>) {
    tokio::spawn(async move {
        let watches = match session_db
            .run(move |db| db.list_active_loop_monitor_watches())
            .await
        {
            Ok(items) => items,
            Err(err) => {
                app_warn!(
                    "loop",
                    "monitor_recovery",
                    "failed to list monitors: {}",
                    err
                );
                return;
            }
        };
        for (_, watch) in watches {
            if let Err(err) =
                start_loop_monitor_adapter(session_db.clone(), cron_db.clone(), &watch).await
            {
                let message = err.to_string();
                {
                    let watch_id = watch.id.clone();
                    let message = message.clone();
                    let _ = session_db
                        .run(move |db| db.record_loop_monitor_error(&watch_id, &message))
                        .await;
                }
                app_warn!(
                    "loop",
                    "monitor_recovery",
                    "failed to recover monitor {}: {}",
                    watch.id,
                    message
                );
            }
        }
    });
}

fn spawn_loop_monitor_recovery_for_loop(loop_id: String) {
    if !crate::runtime_lock::is_primary() {
        return;
    }
    let (Some(session_db), Some(cron_db)) = (
        crate::get_session_db().cloned(),
        crate::get_cron_db().cloned(),
    ) else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        app_warn!(
            "loop",
            "monitor_recovery",
            "Loop {} resumed without an active Tokio runtime; Cron fallback remains active",
            loop_id
        );
        return;
    };
    handle.spawn(async move {
        let listed = {
            let loop_id = loop_id.clone();
            session_db
                .run(move |db| -> Result<Option<(String, Vec<LoopWatch>)>> {
                    let schedule = match db.get_loop_schedule(&loop_id)? {
                        Some(schedule) if schedule.state == LoopState::Active => schedule,
                        _ => return Ok(None),
                    };
                    let watches = db.list_loop_watches(&schedule.id)?;
                    Ok(Some((schedule.id, watches)))
                })
                .await
        };
        let watches = match listed {
            Ok(Some((_, watches))) => watches,
            Ok(None) => return,
            Err(err) => {
                app_warn!(
                    "loop",
                    "monitor_recovery",
                    "failed to list monitors for resumed Loop {}: {}",
                    loop_id,
                    err
                );
                return;
            }
        };
        for watch in watches.into_iter().filter(|watch| {
            watch.active && matches!(watch.kind, LoopWatchKind::File | LoopWatchKind::Websocket)
        }) {
            if let Err(err) =
                start_loop_monitor_adapter(session_db.clone(), cron_db.clone(), &watch).await
            {
                let message = err.to_string();
                {
                    let watch_id = watch.id.clone();
                    let message = message.clone();
                    let _ = session_db
                        .run(move |db| db.record_loop_monitor_error(&watch_id, &message))
                        .await;
                }
                app_warn!(
                    "loop",
                    "monitor_recovery",
                    "failed to recover resumed monitor {}: {}",
                    watch.id,
                    message
                );
            }
        }
    });
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
    spawn_loop_monitor_recovery(session_db.clone(), cron_db.clone());
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let session_db = session_db.clone();
                    let cron_db = cron_db.clone();
                    crate::blocking::run_blocking(move || {
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
                                return;
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
                            crate::cron::spawn_job_execution(
                                cron_db.clone(),
                                session_db.clone(),
                                job,
                            );
                        }
                    })
                    .await;
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
    use std::sync::OnceLock;

    use super::*;
    use crate::domain_eval::{DomainOperationalGateInput, DomainSoakReportInput};
    use crate::goal::{CreateGoalInput, UpdateGoalInput};
    use crate::model_usage::{ModelUsageEvent, KIND_CHAT};
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

    fn ensure_monitor_jobs_db() {
        static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            if crate::async_jobs::get_async_jobs_db().is_some() {
                return;
            }
            let dir = DIR.get_or_init(|| tempfile::tempdir().expect("monitor jobs tempdir"));
            let db = crate::async_jobs::JobsDB::open(&dir.path().join("background_jobs.db"))
                .expect("monitor jobs db");
            crate::async_jobs::set_async_jobs_db(Arc::new(db));
        });
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
    fn dynamic_trigger_message_includes_self_pacing_contract() {
        let msg = build_loop_trigger_message(
            "loop",
            "run",
            None,
            None,
            None,
            LoopTriggerKind::Dynamic,
            &json!({ "fallbackSecs": 1200, "fallbackUsed": false }),
            None,
            "check CI and address review comments",
        );
        assert!(msg.contains("dynamic self-paced loop"));
        assert!(msg.contains("LOOP_RESCHEDULE_AFTER:"));
        assert!(msg.contains("LOOP_STOP:"));
        assert!(msg.contains("LOOP_BLOCKED:"));
        assert!(msg.contains("check CI and address review comments"));
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
    fn dynamic_trigger_uses_clamped_fallback_interval() {
        let input = CreateLoopScheduleInput {
            session_id: "s".into(),
            goal_id: None,
            goal_criterion_id: None,
            prompt: "poll".into(),
            trigger_kind: LoopTriggerKind::Dynamic,
            trigger_spec: json!({ "fallbackSecs": 5, "fallbackUsed": false }),
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
        let schedule = cron_schedule_from_loop(&input).expect("dynamic cron schedule");
        match schedule {
            CronSchedule::Every { interval_ms, .. } => assert_eq!(interval_ms, 60_000),
            other => panic!("expected every schedule, got {other:?}"),
        }
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
    fn dynamic_loop_watch_rearms_dedups_and_carries_event_context() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "continue when the background job settles".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let spec = json!({
            "eventName": "job:completed",
            "filters": { "jobId": "job_1", "jobStatus": "completed" },
            "debounceSecs": 30,
        });
        let first_watch = session_db
            .upsert_loop_watch(&schedule.id, LoopWatchKind::Job, &spec)
            .expect("watch");
        let rearmed = session_db
            .upsert_loop_watch(&schedule.id, LoopWatchKind::Job, &spec)
            .expect("rearm");
        assert_eq!(first_watch.id, rearmed.id);
        assert_eq!(rearmed.generation, first_watch.generation + 1);

        let event = AppEvent {
            name: "job:completed".to_string(),
            payload: json!({
                "job_id": "job_1",
                "session_id": session.id,
                "kind": "tool",
                "tool": "web_search",
                "status": "completed",
            }),
        };
        assert_eq!(
            session_db
                .enqueue_loop_event_triggers(&event)
                .expect("enqueue"),
            vec![schedule.cron_job_id.clone()]
        );
        assert!(session_db
            .enqueue_loop_event_triggers(&event)
            .expect("dedup")
            .is_empty());

        let admission = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare");
        let LoopRunDecision::Admit(admission) = admission else {
            panic!("dynamic event wake should admit");
        };
        let context = admission.event_context.expect("event context");
        assert_eq!(context["eventName"], json!("job:completed"));
        assert_eq!(context["payload"]["watch"]["id"], json!(rearmed.id));
        assert_eq!(
            context["payload"]["watch"]["generation"],
            json!(rearmed.generation)
        );
    }

    #[test]
    fn terminal_loop_deactivates_durable_watches() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "watch workflow".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let watch = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::AppEvent,
                &json!({ "eventName": "workflow:updated", "filters": {} }),
            )
            .expect("watch");
        session_db
            .transition_loop_schedule(&schedule.id, LoopState::Completed, None)
            .expect("complete");
        let watches = session_db.list_loop_watches(&schedule.id).expect("watches");
        assert_eq!(watches.len(), 1);
        assert!(!watches[0].active);
        assert_eq!(watches[0].generation, watch.generation + 1);
    }

    #[test]
    fn file_monitor_terminal_event_is_untrusted_and_wakes_dynamic_loop() {
        let (dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "inspect changed report".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let watch = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::File,
                &json!({ "path": dir.path(), "debounceSecs": 1 }),
            )
            .expect("file watch");
        let cron_job_id = session_db
            .record_loop_monitor_event(
                &watch.id,
                "changed",
                json!({ "paths": [dir.path().join("report.md")] }),
            )
            .expect("record monitor event")
            .expect("wakeup");
        assert_eq!(cron_job_id, schedule.cron_job_id);

        let admission = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare");
        let LoopRunDecision::Admit(admission) = admission else {
            panic!("file monitor wake should admit");
        };
        let context = admission.event_context.expect("event context");
        assert_eq!(context["payload"]["untrusted"], json!(true));
        assert_eq!(context["payload"]["outcome"], json!("changed"));
        assert_eq!(context["payload"]["watch"]["id"], json!(watch.id));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn real_file_monitor_is_one_shot_and_settles_monitor_job() {
        ensure_monitor_jobs_db();
        let (dir, session_db, cron_db) = temp_dbs();
        let session_db = Arc::new(session_db);
        let cron_db = Arc::new(cron_db);
        let session = session_db.create_session("ha-main").expect("session");
        let watched = dir.path().join("watched");
        std::fs::create_dir_all(&watched).expect("create watched dir");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "inspect a changed file".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let watch = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::File,
                &json!({ "path": watched, "debounceSecs": 1 }),
            )
            .expect("file watch");
        start_loop_monitor_adapter(session_db.clone(), cron_db, &watch)
            .await
            .expect("start file monitor");
        let monitor_job_id = session_db
            .get_loop_watch(&watch.id)
            .expect("read armed watch")
            .expect("watch exists")
            .monitor_job_id
            .expect("monitor job id");
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::write(watched.join("result.txt"), "ready").expect("write watched file");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let current = session_db
                    .get_loop_watch(&watch.id)
                    .expect("read watch")
                    .expect("watch exists");
                if !current.active {
                    assert!(
                        current.monitor_job_id.is_none(),
                        "terminal watches must not retain a stale monitor binding"
                    );
                    let job = crate::async_jobs::JobManager::get(&monitor_job_id)
                        .expect("read monitor job")
                        .expect("monitor job exists");
                    assert_eq!(job.status, crate::async_jobs::JobStatus::Completed);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("file monitor should settle");

        let pending_ticks: i64 = {
            let conn = session_db.conn.lock().expect("lock session db");
            conn.query_row(
                "SELECT COUNT(*) FROM loop_event_ticks WHERE loop_id = ?1 AND consumed_at IS NULL",
                params![schedule.id],
                |row| row.get(0),
            )
            .expect("count pending ticks")
        };
        assert_eq!(
            pending_ticks, 1,
            "one watch generation emits one durable tick"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pause_suspends_file_monitor_without_disabling_durable_watch() {
        ensure_monitor_jobs_db();
        let (dir, session_db, cron_db) = temp_dbs();
        let session_db = Arc::new(session_db);
        let cron_db = Arc::new(cron_db);
        let session = session_db.create_session("ha-main").expect("session");
        let watched = dir.path().join("paused-watch");
        std::fs::create_dir_all(&watched).expect("create watched dir");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "wait for a file".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let watch = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::File,
                &json!({ "path": watched }),
            )
            .expect("file watch");
        start_loop_monitor_adapter(session_db.clone(), cron_db.clone(), &watch)
            .await
            .expect("start file monitor");
        let running = session_db
            .get_loop_watch(&watch.id)
            .expect("read watch")
            .expect("watch exists");
        let monitor_job_id = running.monitor_job_id.expect("monitor job id");

        session_db
            .pause_loop_schedule(&cron_db, &schedule.id)
            .expect("pause Loop");

        let paused = session_db
            .get_loop_watch(&watch.id)
            .expect("read paused watch")
            .expect("watch remains durable");
        assert!(paused.active, "resume can re-arm the same durable watch");
        assert!(paused.monitor_job_id.is_none());
        assert!(!loop_monitors()
            .lock()
            .expect("monitor registry")
            .contains_key(&watch.id));
        let job = crate::async_jobs::JobManager::get(&monitor_job_id)
            .expect("read monitor job")
            .expect("monitor job exists");
        assert_eq!(job.status, crate::async_jobs::JobStatus::Cancelled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn monitor_job_cancellation_clears_durable_watch_binding() {
        ensure_monitor_jobs_db();
        let (dir, session_db, cron_db) = temp_dbs();
        let session_db = Arc::new(session_db);
        let cron_db = Arc::new(cron_db);
        let session = session_db.create_session("ha-main").expect("session");
        let watched = dir.path().join("cancelled-watch");
        std::fs::create_dir_all(&watched).expect("create watched dir");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "wait for a file".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let watch = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::File,
                &json!({ "path": watched }),
            )
            .expect("file watch");
        start_loop_monitor_adapter(session_db.clone(), cron_db, &watch)
            .await
            .expect("start monitor");
        let armed = session_db
            .get_loop_watch(&watch.id)
            .expect("read watch")
            .expect("watch exists");
        let monitor_job_id = armed.monitor_job_id.expect("monitor binding");

        assert!(session_db
            .deactivate_loop_watch_for_monitor_job(&monitor_job_id)
            .expect("deactivate durable watch"));
        let cancelled = session_db
            .get_loop_watch(&watch.id)
            .expect("read watch")
            .expect("watch exists");
        assert!(!cancelled.active);
        assert!(cancelled.monitor_job_id.is_none());
        assert!(!loop_monitors()
            .lock()
            .expect("monitor registry")
            .contains_key(&watch.id));
        crate::async_jobs::JobManager::finish_monitor(
            &monitor_job_id,
            crate::async_jobs::JobStatus::Cancelled,
            None,
            Some("test cleanup"),
        )
        .expect("finish monitor job");
    }

    #[test]
    fn noisy_monitor_is_disabled_after_failure_limit_without_stopping_fallback() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "wait safely".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let watch = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::Websocket,
                &json!({ "url": "wss://example.com/events", "timeoutSecs": 30 }),
            )
            .expect("websocket watch");
        session_db
            .bind_loop_watch_monitor_job(&watch.id, "job_failed_monitor")
            .expect("bind failed monitor projection");
        for _ in 0..LOOP_MONITOR_MAX_FAILURES {
            session_db
                .record_loop_monitor_error(&watch.id, "connection failed")
                .expect("record failure");
        }
        let updated = session_db
            .list_loop_watches(&schedule.id)
            .expect("watches")
            .remove(0);
        assert!(!updated.active);
        assert_eq!(updated.failure_count, LOOP_MONITOR_MAX_FAILURES);
        assert!(updated.monitor_job_id.is_none());
        assert_eq!(
            cron_db
                .get_job(&schedule.cron_job_id)
                .expect("cron")
                .expect("cron job")
                .status
                .as_str(),
            "active"
        );
    }

    #[test]
    fn command_monitor_requires_existing_background_job_handle() {
        let err = normalize_loop_watch_spec(
            LoopWatchKind::Command,
            &json!({ "filters": { "jobStatus": "completed" } }),
        )
        .expect_err("command monitor without job id");
        assert!(err.to_string().contains("filters.jobId"));
    }

    #[test]
    fn websocket_monitor_rejects_credentials_in_durable_url() {
        for url in [
            "wss://user:pass@example.com/events",
            "wss://example.com/events?access_token=secret",
            "wss://example.com/events?api_key=secret",
        ] {
            let err = normalize_loop_watch_spec(LoopWatchKind::Websocket, &json!({ "url": url }))
                .expect_err("durable websocket watch must reject embedded credentials");
            assert!(err.to_string().contains("must not contain"));
        }

        normalize_loop_watch_spec(
            LoopWatchKind::Websocket,
            &json!({ "url": "wss://example.com/events?channel=updates" }),
        )
        .expect("non-sensitive query parameters remain supported");
    }

    #[test]
    fn loop_monitor_quota_is_bounded_but_idempotent_rearm_still_works() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id,
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "wait for bounded external events".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        let first_spec = json!({ "url": "wss://example.com/events/0" });
        let first = session_db
            .upsert_loop_watch(&schedule.id, LoopWatchKind::Websocket, &first_spec)
            .expect("first monitor");
        for index in 1..MAX_ACTIVE_MONITORS_PER_SESSION {
            session_db
                .upsert_loop_watch(
                    &schedule.id,
                    LoopWatchKind::Websocket,
                    &json!({ "url": format!("wss://example.com/events/{index}") }),
                )
                .expect("monitor within quota");
        }
        let overflow = session_db
            .upsert_loop_watch(
                &schedule.id,
                LoopWatchKind::Websocket,
                &json!({ "url": "wss://example.com/events/overflow" }),
            )
            .expect_err("session monitor quota must reject overflow");
        assert!(overflow.to_string().contains("monitor limit reached"));

        let rearmed = session_db
            .upsert_loop_watch(&schedule.id, LoopWatchKind::Websocket, &first_spec)
            .expect("rearming an existing active monitor does not consume another slot");
        assert_eq!(rearmed.id, first.id);
        assert_eq!(rearmed.generation, first.generation + 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hundred_file_monitor_start_stop_cycles_leave_no_active_resources() {
        ensure_monitor_jobs_db();
        let (dir, session_db, cron_db) = temp_dbs();
        let session_db = Arc::new(session_db);
        let cron_db = Arc::new(cron_db);
        let session = session_db.create_session("ha-main").expect("session");
        let watched = dir.path().join("monitor-soak");
        std::fs::create_dir_all(&watched).expect("create watched dir");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "exercise monitor lifecycle".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200 }),
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
            .expect("dynamic loop");
        for _ in 0..100 {
            let watch = session_db
                .upsert_loop_watch(
                    &schedule.id,
                    LoopWatchKind::File,
                    &json!({ "path": watched }),
                )
                .expect("arm file monitor");
            start_loop_monitor_adapter(session_db.clone(), cron_db.clone(), &watch)
                .await
                .expect("start file monitor");
            session_db
                .remove_loop_watch(&schedule.id, &watch.id)
                .expect("stop file monitor");
        }

        let watches = session_db
            .list_loop_watches(&schedule.id)
            .expect("list durable watches");
        assert_eq!(watches.len(), 1, "same canonical watch reuses one row");
        assert!(!watches[0].active);
        assert!(watches[0].monitor_job_id.is_none());
        assert!(!loop_monitors()
            .lock()
            .expect("monitor registry")
            .contains_key(&watches[0].id));
        assert!(
            crate::async_jobs::JobManager::list_active_by_session_limited(&session.id, 50)
                .expect("active monitor jobs")
                .is_empty()
        );
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
    fn loop_run_usage_counts_only_messages_within_run_bounds() {
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

        let started_at = "2026-01-01T00:00:10Z";
        let finished_at = "2026-01-01T00:00:20Z";
        let admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, started_at)
            .expect("prepare loop")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };

        let mut before = NewMessage::assistant("old budget");
        before.timestamp = "2026-01-01T00:00:09Z".to_string();
        before.tokens_in_last = Some(500);
        before.tokens_out = Some(500);
        session_db
            .append_message(&session.id, &before)
            .expect("append before");

        let mut user = NewMessage::user("loop tick");
        user.timestamp = "2026-01-01T00:00:12Z".to_string();
        session_db
            .append_message(&session.id, &user)
            .expect("append user");

        let mut assistant_one = NewMessage::assistant("inside one");
        assistant_one.timestamp = "2026-01-01T00:00:15Z".to_string();
        assistant_one.tokens_in = Some(100);
        assistant_one.tokens_in_last = Some(40);
        assistant_one.tokens_out = Some(10);
        session_db
            .append_message(&session.id, &assistant_one)
            .expect("append assistant one");

        let mut assistant_two = NewMessage::assistant("inside two");
        assistant_two.timestamp = "2026-01-01T00:00:18Z".to_string();
        assistant_two.tokens_in = Some(20);
        assistant_two.tokens_out = Some(5);
        session_db
            .append_message(&session.id, &assistant_two)
            .expect("append assistant two");

        let mut after = NewMessage::assistant("future budget");
        after.timestamp = "2026-01-01T00:00:21Z".to_string();
        after.tokens_in_last = Some(900);
        after.tokens_out = Some(900);
        session_db
            .append_message(&session.id, &after)
            .expect("append after");

        session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("done"),
                None,
                finished_at,
            )
            .expect("finish loop");

        let snapshot = session_db
            .loop_snapshot(&schedule.id, 5)
            .expect("snapshot")
            .expect("snapshot exists");
        let usage = &snapshot.runs[0].usage;
        assert_eq!(usage.message_count, 3);
        assert_eq!(usage.user_turns, 1);
        assert_eq!(usage.assistant_messages, 2);
        assert_eq!(usage.input_tokens, 60);
        assert_eq!(usage.output_tokens, 15);
        assert_eq!(usage.total_tokens, 75);
        assert_eq!(
            usage.attribution, "session_messages_between_loop_run_bounds",
            "loop run usage should disclose its window-based attribution"
        );
    }

    #[test]
    fn loop_run_usage_prefers_trigger_message_boundary_over_time_window() {
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
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: default_dynamic_loop_trigger_spec(),
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

        let started_at = "2026-01-01T00:00:10Z";
        let finished_at = "2026-01-01T00:00:30Z";
        let admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, started_at)
            .expect("prepare loop")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };

        let mut unrelated_before = NewMessage::assistant("inside window, wrong turn");
        unrelated_before.timestamp = "2026-01-01T00:00:11Z".to_string();
        unrelated_before.tokens_in_last = Some(999);
        unrelated_before.tokens_out = Some(999);
        let unrelated_before_id = session_db
            .append_message(&session.id, &unrelated_before)
            .expect("append unrelated before");
        let mut unrelated_before_event = ModelUsageEvent::new(KIND_CHAT).with_usage(999, 999, 9, 9);
        unrelated_before_event.request_key = Some(format!("message:{unrelated_before_id}"));
        unrelated_before_event.session_id = Some(session.id.clone());
        session_db
            .insert_model_usage_event(&unrelated_before_event)
            .expect("insert unrelated before usage");

        let mut trigger = NewMessage::user("loop tick");
        trigger.timestamp = "2026-01-01T00:00:12Z".to_string();
        trigger.attachments_meta = Some(
            json!({
                "loop_trigger": {
                    "run_id": &admission.run_id,
                }
            })
            .to_string(),
        );
        session_db
            .append_message(&session.id, &trigger)
            .expect("append trigger");

        let mut assistant = NewMessage::assistant("loop result");
        assistant.timestamp = "2026-01-01T00:00:18Z".to_string();
        assistant.tokens_in = Some(100);
        assistant.tokens_in_last = Some(25);
        assistant.tokens_out = Some(7);
        let assistant_id = session_db
            .append_message(&session.id, &assistant)
            .expect("append assistant");
        let mut assistant_event = ModelUsageEvent::new(KIND_CHAT).with_usage(30, 9, 3, 4);
        assistant_event.request_key = Some(format!("message:{assistant_id}"));
        assistant_event.session_id = Some(session.id.clone());
        session_db
            .insert_model_usage_event(&assistant_event)
            .expect("insert loop assistant usage");

        let mut unrelated_next_turn = NewMessage::user("manual follow-up");
        unrelated_next_turn.timestamp = "2026-01-01T00:00:20Z".to_string();
        session_db
            .append_message(&session.id, &unrelated_next_turn)
            .expect("append unrelated next user");

        let mut unrelated_after = NewMessage::assistant("manual answer");
        unrelated_after.timestamp = "2026-01-01T00:00:22Z".to_string();
        unrelated_after.tokens_in_last = Some(888);
        unrelated_after.tokens_out = Some(888);
        let unrelated_after_id = session_db
            .append_message(&session.id, &unrelated_after)
            .expect("append unrelated after");
        let mut unrelated_after_event = ModelUsageEvent::new(KIND_CHAT).with_usage(888, 888, 8, 8);
        unrelated_after_event.request_key = Some(format!("message:{unrelated_after_id}"));
        unrelated_after_event.session_id = Some(session.id.clone());
        session_db
            .insert_model_usage_event(&unrelated_after_event)
            .expect("insert unrelated after usage");

        session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("done"),
                None,
                finished_at,
            )
            .expect("finish loop");

        let snapshot = session_db
            .loop_snapshot(&schedule.id, 5)
            .expect("snapshot")
            .expect("snapshot exists");
        let usage = &snapshot.runs[0].usage;
        assert_eq!(usage.message_count, 2);
        assert_eq!(usage.user_turns, 1);
        assert_eq!(usage.assistant_messages, 1);
        assert_eq!(usage.input_tokens, 25);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.total_tokens, 32);
        assert_eq!(usage.attribution, "loop_trigger_message_boundary");
        assert_eq!(usage.provider_events, 1);
        assert_eq!(usage.provider_input_tokens, 30);
        assert_eq!(usage.provider_output_tokens, 9);
        assert_eq!(usage.provider_cache_creation_input_tokens, 3);
        assert_eq!(usage.provider_cache_read_input_tokens, 4);
        assert_eq!(usage.provider_total_tokens, 39);
        assert_eq!(
            usage.provider_attribution,
            "model_usage_events.request_key=message_id"
        );
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
    fn dynamic_loop_reschedule_marker_delays_next_run() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "check CI and review comments".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200, "fallbackUsed": false }),
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
            .expect("create dynamic loop");
        let admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare loop")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };
        let action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("CI is still running.\nLOOP_RESCHEDULE_AFTER: 5m - wait for checks"),
                None,
                &now_rfc3339(),
            )
            .expect("finish dynamic loop");
        assert_eq!(action.backoff_secs, Some(300));
        assert!(!action.pause_cron_job);
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load schedule")
            .expect("schedule");
        assert_eq!(updated.state, LoopState::Active);
        assert_eq!(
            updated
                .trigger_spec
                .get("fallbackUsed")
                .and_then(Value::as_bool),
            Some(false)
        );
        let runs = session_db
            .list_loop_runs(&schedule.id, 10)
            .expect("list runs");
        assert_eq!(
            runs[0].scheduling_decision.as_deref(),
            Some("dynamic_reschedule_300s")
        );
        assert_eq!(
            runs[0].trace["dynamicDecision"]["action"],
            json!("reschedule")
        );
    }

    #[test]
    fn dynamic_loop_tool_reschedule_decision_prevents_missing_fallback() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "check CI and review comments".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200, "fallbackUsed": false }),
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
            .expect("create dynamic loop");
        let admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare loop")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };

        let (_updated, next_run_at) = session_db
            .record_loop_tool_reschedule(&cron_db, &schedule.id, 600, "wait for CI to finish")
            .expect("tool reschedule");
        assert!(next_run_at.is_some());

        let action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&admission.run_id),
                None,
                LoopRunState::Succeeded,
                Some("Checked CI; no textual decision marker."),
                None,
                &now_rfc3339(),
            )
            .expect("finish dynamic loop");
        assert_eq!(action.backoff_secs, Some(600));
        assert!(!action.pause_cron_job);
        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load schedule")
            .expect("schedule");
        assert_eq!(updated.state, LoopState::Active);
        assert_eq!(
            updated
                .trigger_spec
                .get("fallbackUsed")
                .and_then(Value::as_bool),
            Some(false)
        );
        let runs = session_db
            .list_loop_runs(&schedule.id, 10)
            .expect("list runs");
        assert_eq!(
            runs[0].scheduling_decision.as_deref(),
            Some("dynamic_reschedule_600s")
        );
        assert_eq!(
            runs[0].trace["dynamicDecision"]["action"],
            json!("reschedule")
        );
    }

    #[test]
    fn dynamic_maintenance_loop_refreshes_loop_md_before_run() {
        let (dir, session_db, cron_db) = temp_dbs();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace dir");
        let loop_md = workspace.join("loop.md");
        std::fs::write(&loop_md, "First maintenance instructions").expect("write loop md");
        let session = session_db.create_session("ha-main").expect("session");
        session_db
            .update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
            .expect("set working dir");
        let resolution = resolve_default_loop_prompt_for_session(&session_db, &session.id);
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: resolution.prompt,
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: dynamic_loop_trigger_spec_with_maintenance_prompt(
                        resolution.metadata,
                    ),
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
            .expect("create dynamic maintenance loop");

        std::fs::write(&loop_md, "Updated maintenance instructions").expect("update loop md");
        let admission = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, "2026-01-01T00:01:00Z")
            .expect("prepare loop run");
        let LoopRunDecision::Admit(admission) = admission else {
            panic!("expected loop run admission");
        };
        assert!(admission
            .prompt
            .contains("Updated maintenance instructions"));
        assert!(!admission.prompt.contains("First maintenance instructions"));
        assert_eq!(
            admission
                .trigger_spec
                .get("maintenancePrompt")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("loop_md")
        );

        let updated = session_db
            .get_loop_schedule(&schedule.id)
            .expect("get loop")
            .expect("loop exists");
        assert!(updated.prompt.contains("Updated maintenance instructions"));
        let snapshot = session_db
            .loop_snapshot(&schedule.id, 5)
            .expect("snapshot")
            .expect("snapshot exists");
        assert_eq!(
            snapshot.runs[0]
                .trace
                .get("maintenancePrompt")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("loop_md")
        );
    }

    #[test]
    fn dynamic_loop_tool_stop_completes_and_pauses_cron() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "check CI and review comments".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200, "fallbackUsed": false }),
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
            .expect("create dynamic loop");
        let _admission = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare loop")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };

        let updated = session_db
            .record_loop_tool_stop(
                &cron_db,
                &schedule.id,
                true,
                "CI is green and no review remains",
            )
            .expect("tool stop");
        assert_eq!(updated.state, LoopState::Completed);
        assert_eq!(
            updated.progress_summary.as_deref(),
            Some("CI is green and no review remains")
        );
        let job = cron_db
            .get_job(&schedule.cron_job_id)
            .expect("read cron job")
            .expect("cron job exists");
        assert_eq!(job.status.as_str(), "paused");
    }

    #[test]
    fn dynamic_loop_fallback_once_then_blocks_without_decision() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: None,
                    goal_criterion_id: None,
                    prompt: "check CI and review comments".into(),
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200, "fallbackUsed": false }),
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
            .expect("create dynamic loop");
        let first = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare first")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };
        let first_action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&first.run_id),
                None,
                LoopRunState::Succeeded,
                Some("Checked CI; no decision marker."),
                None,
                &now_rfc3339(),
            )
            .expect("finish first");
        assert_eq!(first_action.backoff_secs, Some(1200));
        assert!(!first_action.pause_cron_job);
        let after_first = session_db
            .get_loop_schedule(&schedule.id)
            .expect("load first")
            .expect("schedule");
        assert_eq!(
            after_first
                .trigger_spec
                .get("fallbackUsed")
                .and_then(Value::as_bool),
            Some(true)
        );

        let second = match session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &now_rfc3339())
            .expect("prepare second")
        {
            LoopRunDecision::Admit(admission) => admission,
            other => panic!("expected admission, got {other:?}"),
        };
        let second_action = session_db
            .finish_loop_cron_run(
                &schedule.cron_job_id,
                Some(&second.run_id),
                None,
                LoopRunState::Succeeded,
                Some("Checked CI again; still no decision marker."),
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
        assert!(after_second
            .blocked_reason
            .as_deref()
            .unwrap_or("")
            .contains("did not reschedule"));
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

    #[test]
    fn loop_watchdog_reports_due_active_loop_without_active_run() {
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
        let overdue_at = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
        cron_db
            .conn
            .lock()
            .expect("cron lock")
            .execute(
                "UPDATE cron_jobs SET status='active', next_run_at=?1, running_at=NULL WHERE id=?2",
                rusqlite::params![overdue_at, schedule.cron_job_id],
            )
            .expect("set overdue cron");

        let findings = session_db
            .list_loop_watchdog_findings(&cron_db, &session.id, 60)
            .expect("watchdog findings");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].loop_id, schedule.id);
        assert_eq!(findings[0].code, "loop_due_not_claimed");
        assert_eq!(findings[0].cron_status.as_deref(), Some("active"));
        assert!(findings[0].overdue_secs.unwrap_or_default() >= 60);
    }

    #[test]
    fn loop_watchdog_reports_missing_backing_cron_even_without_next_run() {
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
        cron_db
            .delete_job(&schedule.cron_job_id)
            .expect("delete backing cron");

        let findings = session_db
            .list_loop_watchdog_findings(&cron_db, &session.id, 60)
            .expect("watchdog findings");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].loop_id, schedule.id);
        assert_eq!(findings[0].code, "loop_cron_missing");
        assert!(findings[0].next_run_at.is_none());
        assert!(findings[0].cron_status.is_none());
    }

    #[test]
    fn loop_watchdog_reports_stale_running_loop_run_after_cron_recovery() {
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
                    trigger_kind: LoopTriggerKind::Dynamic,
                    trigger_spec: json!({ "fallbackSecs": 1200, "fallbackUsed": false }),
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
        let stale_started_at = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
        let admission = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &stale_started_at)
            .expect("prepare loop");
        let run_id = match admission {
            LoopRunDecision::Admit(admission) => admission.run_id,
            other => panic!("expected admission, got {other:?}"),
        };
        cron_db
            .clear_running(&schedule.cron_job_id)
            .expect("simulate startup cron recovery clearing running marker");

        let findings = session_db
            .list_loop_watchdog_findings(&cron_db, &session.id, 60)
            .expect("watchdog findings");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].loop_id, schedule.id);
        assert_eq!(findings[0].code, "loop_run_maybe_interrupted");
        assert_eq!(findings[0].latest_run_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(findings[0].latest_run_state.as_deref(), Some("running"));
        assert!(findings[0].overdue_secs.unwrap_or_default() >= 60);
    }

    #[test]
    fn loop_watchdog_does_not_flag_cron_job_already_running() {
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
        let overdue_at = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
        let running_at = Utc::now().to_rfc3339();
        cron_db
            .conn
            .lock()
            .expect("cron lock")
            .execute(
                "UPDATE cron_jobs SET status='active', next_run_at=?1, running_at=?2 WHERE id=?3",
                rusqlite::params![overdue_at, running_at, schedule.cron_job_id],
            )
            .expect("set running cron");

        let findings = session_db
            .list_loop_watchdog_findings(&cron_db, &session.id, 60)
            .expect("watchdog findings");

        assert!(findings.is_empty());
    }
}
