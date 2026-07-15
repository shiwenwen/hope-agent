use serde::{Deserialize, Serialize};

// ── Data Structures ─────────────────────────────────────────────

/// Schedule types: one-shot, fixed interval, or cron expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CronSchedule {
    /// Fire once at a specific timestamp
    At { timestamp: String },
    /// Fire every N milliseconds
    Every {
        interval_ms: u64,
        /// The first scheduled fire time for this interval job.
        /// Backfilled for legacy rows so calendar expansion does not start at
        /// the query window boundary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_at: Option<String>,
    },
    /// Cron expression with optional timezone (default UTC)
    Cron {
        expression: String,
        timezone: Option<String>,
    },
}

/// What the job does when triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CronPayload {
    /// Run an agent turn with the given prompt
    AgentTurn {
        prompt: String,
        agent_id: Option<String>,
    },
    /// Fire a managed `/loop` trigger back into an existing parent session.
    ///
    /// This reuses cron's durable scheduling and recovery, but executes through
    /// the parent-session injection pipeline so the loop preserves conversation
    /// context, Goal linkage, permissions, Project/KB access, and idle gating.
    SessionLoop {
        loop_id: String,
        session_id: String,
        prompt: String,
        agent_id: Option<String>,
        goal_id: Option<String>,
    },
}

/// Stable discriminator exposed by cron summary DTOs that do not need the
/// payload's full prompt/session data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CronPayloadType {
    AgentTurn,
    SessionLoop,
}

impl From<&CronPayload> for CronPayloadType {
    fn from(payload: &CronPayload) -> Self {
        match payload {
            CronPayload::AgentTurn { .. } => Self::AgentTurn,
            CronPayload::SessionLoop { .. } => Self::SessionLoop,
        }
    }
}

/// Job status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CronJobStatus {
    Active,
    Paused,
    Disabled,
    Completed,
    Missed,
}

impl CronJobStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
            Self::Completed => "completed",
            Self::Missed => "missed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "disabled" => Self::Disabled,
            "completed" => Self::Completed,
            "missed" => Self::Missed,
            _ => Self::Active,
        }
    }
}

/// A scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Optional Project context to attach each isolated cron run session to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub status: CronJobStatus,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    /// Set when the job is currently executing; cleared on completion.
    pub running_at: Option<String>,
    pub consecutive_failures: u32,
    pub max_failures: u32,
    pub created_at: String,
    pub updated_at: String,
    /// Whether to send a desktop notification when this job completes.
    #[serde(default = "crate::default_true")]
    pub notify_on_complete: bool,
    /// IM channel conversations to fan-out the job's final output to.
    /// Empty = no delivery (job result only lands in the isolated session).
    #[serde(default)]
    pub delivery_targets: Vec<CronDeliveryTarget>,
    /// §8: when true, a *successful* delivery is prefixed with `[Cron] {name}`
    /// so multiple jobs fanning out to the same chat are distinguishable
    /// (failure deliveries already carry `⚠️ [Cron] {name} failed:`). Opt-in
    /// per job; default off keeps the raw agent reply.
    #[serde(default)]
    pub prefix_delivery_with_name: bool,
    /// C19: optional per-job override of the global `CronConfig.job_timeout_secs`
    /// per-run wall-clock budget (clamped to `[30, 7200]s` at use). `None` = use
    /// the global default. Lets a legitimately long-running task declare a higher
    /// budget without raising the global cap (which would let a wedged job burn a
    /// bigger budget every run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_timeout_secs: Option<u64>,
    /// Per-job override of the cron run session's permission mode. `None` =
    /// inherit the agent's `default_session_permission_mode` (current behavior).
    /// Only changes whether approval-needing tools auto-deny (default/smart) or
    /// bypass (yolo) under the unattended cron surface — the fail-closed surface
    /// logic and strict gates are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode_override: Option<crate::permission::SessionMode>,
    /// Per-job override of the cron run session's sandbox mode. `None` = inherit
    /// the agent's `effective_default_sandbox_mode()`. Confines the run's blast
    /// radius (off/standard/isolated/workspace/trusted) so an unattended task can
    /// act autonomously without putting the host at risk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_mode_override: Option<crate::permission::SandboxMode>,
}

/// A cron job execution lease. Constructed only after the DB atomically marks
/// a job as running, so executors do not need to claim it again.
#[derive(Debug, Clone)]
pub struct ClaimedCronJob {
    pub job: CronJob,
    pub claimed_at: String,
    /// C12a: true for a manual `run now` (claim_immediate). A run-now is a one-off
    /// test orthogonal to the schedule — its terminal handling records the run +
    /// delivers but must NOT mutate the job's status / schedule / failure count
    /// (no reviving a disabled job on success, no auto-disable on a test failure,
    /// no rescheduling the next occurrence).
    pub immediate: bool,
}

/// A single IM channel conversation target for cron result delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronDeliveryTarget {
    /// Channel plugin id, e.g. "telegram" / "feishu" / "slack".
    pub channel_id: String,
    /// `ChannelAccountConfig.id` of the sending account.
    pub account_id: String,
    /// Destination `ChannelConversation.chat_id`.
    pub chat_id: String,
    /// Optional thread/topic id (Feishu topic, Slack thread, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Cached human-readable label for UI display (not used at send time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// §8: set when this target's sending account has been deleted (detected at
    /// delivery time or eagerly when the account is removed). A stale target is
    /// surfaced in the GUI (marked red) and skipped at send time. Cleared again
    /// if the account ever resolves on a later run.
    #[serde(default)]
    pub stale: bool,
}

/// A single run log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunLog {
    pub id: i64,
    pub job_id: String,
    pub session_id: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    /// §8: outcome of fanning this run's result to the job's `delivery_targets`.
    /// `None` = the job has no delivery targets (nothing to fan out). Otherwise
    /// one of `"delivered"` (all targets ok), `"partial"` (some failed/skipped),
    /// `"failed"` (no target received it). Surfaced in the GUI run-log list.
    #[serde(default)]
    pub delivery_status: Option<String>,
}

/// One row of the global cron-run timeline (a single run of any job), surfaced
/// in the cron panel's "conversations" view. The run rows come from `CronDB`
/// (`cron_run_logs` + `cron_jobs`); `title` / `unread_count` are hydrated by the
/// assembling layer from `SessionDB` (a separate database — cannot be SQL-joined).
/// `title` falls back to `job_name` and `unread_count` to `0` when the run's
/// session row is missing (e.g. purged).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronTimelineRow {
    /// Stable identity of this run row. Loop runs share their parent session,
    /// so `session_id` cannot be used as the list key or selection identity.
    pub run_log_id: i64,
    pub session_id: String,
    pub job_id: String,
    pub job_name: String,
    /// Payload discriminator from the owning job. `None` is reserved for
    /// orphaned legacy run rows whose job record no longer exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_type: Option<CronPayloadType>,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub result_preview: Option<String>,
    /// Session title from `SessionDB`; defaults to `job_name` when absent.
    #[serde(default)]
    pub title: Option<String>,
    /// Unread-session marker (`0` or `1`) for this run (from `SessionDB`).
    #[serde(default)]
    pub unread_count: i64,
}

/// Input for creating a new job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCronJob {
    pub name: String,
    pub description: Option<String>,
    /// Optional Project context to attach each isolated cron run session to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub max_failures: Option<u32>,
    pub notify_on_complete: Option<bool>,
    /// Optional delivery targets. `None` = no delivery, `Some([])` = explicit opt-out,
    /// `Some([...])` = fan-out to the listed channel conversations.
    #[serde(default)]
    pub delivery_targets: Option<Vec<CronDeliveryTarget>>,
    /// §8: opt-in `[Cron] {name}` prefix on successful deliveries (see `CronJob`).
    #[serde(default)]
    pub prefix_delivery_with_name: Option<bool>,
    /// C19: optional per-job run timeout override (seconds); `None` = global default.
    #[serde(default)]
    pub job_timeout_secs: Option<u64>,
    /// Per-job permission-mode override; `None` = follow the agent default.
    #[serde(default)]
    pub permission_mode_override: Option<crate::permission::SessionMode>,
    /// Per-job sandbox-mode override; `None` = follow the agent default.
    #[serde(default)]
    pub sandbox_mode_override: Option<crate::permission::SandboxMode>,
}

/// §8: a cron job that references a given channel account in its delivery
/// targets. Returned to the channel-account delete confirmation so the user
/// sees which scheduled tasks fan out to the account they're about to remove.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronAccountRef {
    pub job_id: String,
    pub job_name: String,
    /// Number of delivery targets in this job pointing at the account.
    pub target_count: usize,
}

/// Calendar event for the calendar view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub job_id: String,
    pub job_name: String,
    pub payload_type: CronPayloadType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub scheduled_at: String,
    pub status: CronJobStatus,
    pub run_log: Option<CronRunLog>,
}
