use serde::{Deserialize, Serialize};

// ── Data Structures ─────────────────────────────────────────────

/// Sub-agent run status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Spawning,
    Running,
    Completed,
    Error,
    Timeout,
    Killed,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Spawning => "spawning",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Killed => "killed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "spawning" => Self::Spawning,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "error" => Self::Error,
            "timeout" => Self::Timeout,
            "killed" => Self::Killed,
            _ => Self::Error,
        }
    }

    /// Whether this status represents a terminal (finished) state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Error | Self::Timeout | Self::Killed)
    }
}

/// A sub-agent run record persisted in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRun {
    pub run_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub child_session_id: String,
    pub task: String,
    pub status: SubagentStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub depth: u32,
    pub model_used: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    /// Optional display label for tracking
    pub label: Option<String>,
    /// Number of file attachments passed to the sub-agent
    pub attachment_count: u32,
    /// Input token usage (if available)
    pub input_tokens: Option<u64>,
    /// Output token usage (if available)
    pub output_tokens: Option<u64>,
}

/// Parameters for spawning a sub-agent.
#[derive(Debug, Clone)]
pub struct SpawnParams {
    pub task: String,
    pub agent_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub depth: u32,
    pub timeout_secs: Option<u64>,
    pub model_override: Option<String>,
    /// Optional display label for tracking
    pub label: Option<String>,
    /// File attachments to pass to the sub-agent
    pub attachments: Vec<crate::agent::Attachment>,
}

/// Event payload for streaming parent agent responses back to frontend.
/// Emitted when a sub-agent completes and the backend auto-injects the result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentAgentStreamEvent {
    pub event_type: String,             // "started" | "delta" | "done" | "error"
    pub parent_session_id: String,
    pub run_id: String,
    pub push_message: Option<String>,   // only for "started"
    pub delta: Option<String>,          // raw JSON delta string, only for "delta"
    pub error: Option<String>,          // only for "error"
}

/// Event payload emitted to the frontend via Tauri events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub event_type: String,
    pub run_id: String,
    pub parent_session_id: String,
    pub child_agent_id: String,
    pub child_session_id: String,
    pub task_preview: String,
    pub status: SubagentStatus,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    /// Optional display label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Input tokens used (available on terminal events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Output tokens used (available on terminal events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Full result text — included only in terminal events for push delivery.
    /// Frontend uses this to auto-inject the result into the parent agent's conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_full: Option<String>,
}

