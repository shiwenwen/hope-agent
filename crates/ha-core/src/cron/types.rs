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
}

/// Input for creating a new job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCronJob {
    pub name: String,
    pub description: Option<String>,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub max_failures: Option<u32>,
    pub notify_on_complete: Option<bool>,
    /// Optional delivery targets. `None` = no delivery, `Some([])` = explicit opt-out,
    /// `Some([...])` = fan-out to the listed channel conversations.
    #[serde(default)]
    pub delivery_targets: Option<Vec<CronDeliveryTarget>>,
}

/// Calendar event for the calendar view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub job_id: String,
    pub job_name: String,
    pub scheduled_at: String,
    pub status: CronJobStatus,
    pub run_log: Option<CronRunLog>,
}
