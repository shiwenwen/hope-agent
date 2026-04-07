//! ACP Control Plane — Core type definitions.
//!
//! Defines the `AcpRuntime` trait (pluggable backend abstraction) and all
//! shared data structures for managing external ACP agent sessions.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;

// ── AcpRuntime trait ─────────────────────────────────────────────

/// Pluggable ACP runtime backend.
///
/// Each external ACP agent type (Claude Code, Codex CLI, Gemini CLI, …)
/// implements this trait.  The control plane calls these methods to manage
/// the full session lifecycle.
#[async_trait]
pub trait AcpRuntime: Send + Sync {
    /// Unique backend identifier (e.g. "claude-code", "codex-cli").
    fn backend_id(&self) -> &str;

    /// Human-readable name for UI display.
    fn display_name(&self) -> &str;

    /// Check whether the backend binary is installed and executable.
    async fn is_available(&self) -> bool;

    /// Return the backend version string.
    async fn get_version(&self) -> anyhow::Result<String>;

    /// Create (or resume) an ACP session against this backend.
    async fn create_session(&self, params: AcpCreateParams) -> anyhow::Result<AcpExternalSession>;

    /// Send a prompt and stream events back through `event_tx`.
    ///
    /// The implementation MUST check `cancel` periodically and abort
    /// promptly when it is set to `true`.
    async fn run_turn(
        &self,
        session: &AcpExternalSession,
        prompt: &str,
        event_tx: mpsc::Sender<AcpStreamEvent>,
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<AcpTurnResult>;

    /// Cancel the currently running turn for a session.
    async fn cancel_turn(&self, session: &AcpExternalSession) -> anyhow::Result<()>;

    /// Close a session and release all associated resources (kill child process).
    async fn close_session(&self, session: &AcpExternalSession) -> anyhow::Result<()>;

    /// Declared capabilities of this backend.
    fn capabilities(&self) -> AcpRuntimeCapabilities {
        AcpRuntimeCapabilities::default()
    }

    /// Perform a health check and return diagnostic information.
    async fn health_check(&self) -> AcpHealthStatus;
}

// ── Session creation ─────────────────────────────────────────────

/// Parameters for creating an external ACP session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpCreateParams {
    /// Working directory for the external agent.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Optional system prompt override.
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Model override (e.g. "claude-sonnet-4-20250514").
    #[serde(default)]
    pub model: Option<String>,

    /// Timeout in seconds for each turn (default: from config).
    #[serde(default)]
    pub timeout_secs: Option<u64>,

    /// Resume an existing session by its external session ID.
    #[serde(default)]
    pub resume_session_id: Option<String>,
}

/// Handle to an active external ACP session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpExternalSession {
    /// Locally-generated session ID (UUID).
    pub session_id: String,
    /// Backend that owns this session.
    pub backend_id: String,
    /// Session ID on the external agent side (from ACP initialize/session/new).
    #[serde(default)]
    pub external_session_id: Option<String>,
    /// PID of the child process (for process management).
    #[serde(default)]
    pub pid: Option<u32>,
    /// Timestamp (ISO-8601).
    pub created_at: String,
}

// ── Turn result ──────────────────────────────────────────────────

/// Result of a single turn (prompt → response) execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpTurnResult {
    /// Why the turn stopped ("end_turn", "max_tokens", "error", …).
    pub stop_reason: String,
    /// Full accumulated response text.
    pub response_text: String,
    /// Token usage.
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Summary of tool calls made during this turn.
    #[serde(default)]
    pub tool_calls: Vec<AcpToolCallSummary>,
}

/// Brief summary of a tool call (for UI / parent agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpToolCallSummary {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

// ── Stream events ────────────────────────────────────────────────

/// Streaming event emitted during a `run_turn` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpStreamEvent {
    /// Incremental text output from the agent.
    TextDelta { content: String },

    /// Incremental thinking / reasoning output.
    ThinkingDelta { content: String },

    /// A tool call has started or updated.
    ToolCall {
        tool_call_id: String,
        name: String,
        /// "in_progress" | "completed" | "failed"
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        arguments: Option<String>,
    },

    /// Result returned from a tool call.
    ToolResult {
        tool_call_id: String,
        /// "completed" | "failed"
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result_preview: Option<String>,
    },

    /// Token usage update.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },

    /// An error occurred during the turn.
    Error { message: String },

    /// The turn is complete.
    Done { stop_reason: String },
}

// ── Run record (persisted to SQLite) ─────────────────────────────

/// Persistent record of an ACP spawn run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRun {
    /// Unique run identifier (UUID).
    pub run_id: String,
    /// Parent session that spawned this run.
    pub parent_session_id: String,
    /// Backend used.
    pub backend_id: String,
    /// External session ID on the agent side.
    #[serde(default)]
    pub external_session_id: Option<String>,
    /// Task description given to the agent.
    pub task: String,
    /// Current status.
    pub status: AcpRunStatus,
    /// Accumulated result text (truncated).
    #[serde(default)]
    pub result: Option<String>,
    /// Error message (if status is Error).
    #[serde(default)]
    pub error: Option<String>,
    /// Model actually used by the external agent.
    #[serde(default)]
    pub model_used: Option<String>,
    /// ISO-8601 timestamp.
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Optional user-facing label.
    #[serde(default)]
    pub label: Option<String>,
    /// Child process PID.
    #[serde(default)]
    pub pid: Option<u32>,
}

/// Status of an ACP run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpRunStatus {
    Starting,
    Running,
    Completed,
    Error,
    Timeout,
    Killed,
}

impl AcpRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Error | Self::Timeout | Self::Killed
        )
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Killed => "killed",
        }
    }
}

impl std::fmt::Display for AcpRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Capabilities & health ────────────────────────────────────────

/// Declared capabilities of an ACP runtime backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRuntimeCapabilities {
    /// Whether the backend accepts image attachments.
    #[serde(default)]
    pub supports_images: bool,
    /// Whether the backend emits thinking/reasoning events.
    #[serde(default)]
    pub supports_thinking: bool,
    /// Whether the backend has interactive tool approval.
    #[serde(default)]
    pub supports_tool_approval: bool,
    /// Whether sessions can be resumed after close.
    #[serde(default)]
    pub supports_session_resume: bool,
    /// Max context window tokens (informational).
    #[serde(default)]
    pub max_context_window: Option<u64>,
}

/// Health-check result for a single backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpHealthStatus {
    /// Whether the backend is currently available.
    pub available: bool,
    /// Resolved path to the binary.
    #[serde(default)]
    pub binary_path: Option<String>,
    /// Version string (e.g. "1.2.3").
    #[serde(default)]
    pub version: Option<String>,
    /// Error message if unavailable.
    #[serde(default)]
    pub error: Option<String>,
    /// ISO-8601 timestamp of the last check.
    pub last_checked: String,
}

// ── Tauri event payload ──────────────────────────────────────────

/// Payload emitted via Tauri global event `acp_control_event`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpControlEvent {
    /// "spawned" | "text_delta" | "tool_call" | "tool_result" | "completed" | "error" | "killed"
    pub event_type: String,
    pub run_id: String,
    pub parent_session_id: String,
    pub backend_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Event-specific payload.
    pub data: serde_json::Value,
}

// ── Backend info (for UI listing) ────────────────────────────────

/// Summary info about a registered backend (returned by Tauri commands).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpBackendInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub health: AcpHealthStatus,
    pub capabilities: AcpRuntimeCapabilities,
}
