use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::execution_mode::ExecutionMode;
use crate::permission::{SandboxMode, SessionMode};
use crate::plan::PlanModeState;

/// Defaults selected while a chat is still a draft. These values are consumed
/// only when the first turn materializes the Session; legacy top-level chat
/// overrides remain one-turn-only.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDefaultsInput {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemoryPolicyValue {
    #[default]
    Inherit,
    Allow,
    Deny,
}

impl SessionMemoryPolicyValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            _ => Self::Inherit,
        }
    }

    pub fn allows(self) -> bool {
        !matches!(self, Self::Deny)
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMemoryPolicy {
    pub use_memories: SessionMemoryPolicyValue,
    pub contribute_to_memories: SessionMemoryPolicyValue,
}

// Well-known keys for the `messages.attachments_meta` JSON column. Single
// source of truth for both writers (Tauri/HTTP `chat` commands, channel /
// cron / subagent injection) and readers (`chatUtils.ts` mirrors these on
// the frontend).
pub const ATTACHMENT_META_KEY_PLAN_TRIGGER: &str = "plan_trigger";
pub const ATTACHMENT_META_KEY_PLAN_COMMENT: &str = "plan_comment";
pub const ATTACHMENT_META_KEY_GOAL_TRIGGER: &str = "goal_trigger";
pub const ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS: &str = "tool_media_items";
pub const ATTACHMENT_META_KEY_ACTIVE_MEMORY: &str = "active_memory";
pub const ATTACHMENT_META_KEY_USED_MEMORY_REFS: &str = "used_memory_refs";
pub const ATTACHMENT_META_KEY_RETRIEVAL_PLANNER: &str = "retrieval_planner";

/// Resolve the `attachments_meta` value for a user-message coming from the
/// `chat` API surface (Tauri command + HTTP route). Centralizes the
/// plan_trigger > plan_comment > goal_trigger > user_attachments precedence so
/// both shells can't silently drift; if the caller sets both `plan_trigger` and
/// `plan_comment`, plan_trigger wins (a trigger is never also a comment).
pub fn build_chat_user_attachments_meta(
    plan_trigger: bool,
    plan_comment: Option<&Value>,
    goal_trigger: bool,
    user_attachments: Option<String>,
) -> Option<String> {
    if plan_trigger {
        Some(json!({ ATTACHMENT_META_KEY_PLAN_TRIGGER: true }).to_string())
    } else if let Some(payload) = plan_comment {
        Some(merge_user_message_meta(
            json!({ ATTACHMENT_META_KEY_PLAN_COMMENT: payload }),
            user_attachments,
        ))
    } else if goal_trigger {
        Some(merge_user_message_meta(
            json!({ ATTACHMENT_META_KEY_GOAL_TRIGGER: true }),
            user_attachments,
        ))
    } else {
        user_attachments
    }
}

fn merge_user_message_meta(base: Value, user_attachments: Option<String>) -> String {
    let Some(raw) = user_attachments else {
        return base.to_string();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return base.to_string();
    };
    let mut map = match base {
        Value::Object(map) => map,
        other => return other.to_string(),
    };
    match parsed {
        Value::Array(items) if !items.is_empty() => {
            map.insert("user_attachments".to_string(), Value::Array(items));
        }
        Value::Object(existing) => {
            for (key, value) in existing {
                map.entry(key).or_insert(value);
            }
        }
        _ => {}
    }
    Value::Object(map).to_string()
}

/// Persist structured media emitted by a tool result in `attachments_meta`
/// without polluting `tool_result`, which is replayed back into model context.
pub fn build_tool_media_items_attachments_meta(media_items: &Value) -> Option<String> {
    if media_items.as_array().is_none_or(|items| items.is_empty()) {
        return None;
    }
    Some(json!({ ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS: media_items }).to_string())
}

// ── Data Structures ──────────────────────────────────────────────

/// Classifies a session so cross-cutting surfaces can filter it.
///
/// `Regular` is the normal user-facing chat. `Knowledge` is a knowledge-space
/// sidebar conversation — persisted (so history survives) but kept out of the
/// main session sidebar / `/sessions` picker, and driving a trimmed tool set
/// at the chat-engine layer (`ToolScope::Knowledge`). It is NOT a security
/// boundary: KB access is still decided solely by `effective_kb_access`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    #[default]
    Regular,
    Knowledge,
    /// A design-space per-project chat thread — persisted (history survives)
    /// but kept out of the main sidebar / `/sessions` picker / global FTS, and
    /// driving a trimmed tool set (`ToolScope::Design`). Like `Knowledge`, it
    /// is NOT a security boundary.
    Design,
    EvalFixture,
}

impl SessionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionKind::Regular => "regular",
            SessionKind::Knowledge => "knowledge",
            SessionKind::Design => "design",
            SessionKind::EvalFixture => "eval_fixture",
        }
    }

    /// Lenient parse — any unknown value (incl. legacy NULL coerced to "")
    /// falls back to `Regular` so old rows / forward-compat writes are safe.
    pub fn from_db_string(s: &str) -> Self {
        match s {
            "knowledge" => SessionKind::Knowledge,
            "design" => SessionKind::Design,
            "eval_fixture" => SessionKind::EvalFixture,
            _ => SessionKind::Regular,
        }
    }
}

/// Countdown metadata for the sidebar pending-interaction badge: the earliest
/// deadline among a session's pending approvals / ask_user groups that have a
/// timeout configured. Durations are normalized to milliseconds here so the
/// frontend never deals with the approval(ms) vs ask_user(s) unit split.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCountdown {
    /// Earliest auto-resolve instant (server wall clock, epoch ms).
    pub deadline_at_ms: i64,
    /// Full timeout span of that same request (>= 1), for progress ratio.
    pub total_ms: i64,
    /// Server wall clock at enrich time, for client clock-skew correction.
    pub server_now_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub title: Option<String>,
    #[serde(default = "default_title_source")]
    pub title_source: String,
    pub agent_id: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    /// Temperature fixed for this Session. `None` means the provider-native
    /// default was snapshotted, not that the Session should keep inheriting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Per-session Think / reasoning effort override. `None` falls back to
    /// the runtime default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Internal marker distinguishing a snapshotted `None` temperature from
    /// a legacy row that has not captured its defaults yet.
    #[serde(skip)]
    pub runtime_defaults_initialized: bool,
    pub created_at: String,
    pub updated_at: String,
    /// When set, the sidebar sorts this session above unpinned sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
    pub message_count: i64,
    /// Whether this regular desktop conversation is unread, encoded as `0` or
    /// `1` for transport compatibility. Any number of assistant messages after
    /// the read watermark still contributes exactly one unread conversation.
    pub unread_count: i64,
    /// Whether a channel-attached conversation has unread IM activity, encoded
    /// as `0` or `1`. Kept separate from regular desktop unread.
    #[serde(default)]
    pub channel_unread_count: i64,
    /// Whether the latest persisted message is marked as an error.
    /// Used by the sidebar to render a red exclamation indicator.
    #[serde(default)]
    pub has_error: bool,
    /// Number of pending interactions waiting on the user for this session
    /// (sum of pending tool approvals + pending ask_user_question groups).
    /// Populated at the command/route layer, not in `list_sessions_paged`.
    #[serde(default)]
    pub pending_interaction_count: i64,
    /// Earliest auto-resolve deadline among this session's pending
    /// interactions that carry a timeout. `None` when no pending interaction
    /// has a timeout configured. Populated alongside
    /// `pending_interaction_count` at the command/route layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_countdown: Option<PendingCountdown>,
    pub is_cron: bool,
    /// If this session was created by a sub-agent spawn, stores the parent session ID.
    pub parent_session_id: Option<String>,
    /// If this session was forked from another user-facing session, stores the
    /// source session ID. This is intentionally separate from
    /// `parent_session_id`: forked sessions stay first-class sidebar sessions,
    /// while `parent_session_id` marks hidden sub-agent children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_session_id: Option<String>,
    /// Optional source message boundary used when the fork copied only part of
    /// the source transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_message_id: Option<i64>,
    /// Best-effort live title of the source session for UI breadcrumbs. Null
    /// when the source session was deleted or had no title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_session_title: Option<String>,
    /// Plan mode state for this session. Serialized as a snake_case string
    /// (`off` / `planning` / `review` / `executing` / `paused` / `completed`)
    /// matching the frontend's loose `string` type.
    #[serde(default)]
    pub plan_mode: PlanModeState,
    /// Persistent execution mode policy for this session (`off` / `guarded` /
    /// `deep` / `autonomous`). Injected into the system prompt so `/mode`
    /// changes survive refreshes and affect every chat entry point.
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    /// Persistent workflow autonomy mode for this session (`off` / `on` /
    /// `ultracode`). When enabled, the model may call the workflow tool
    /// to create observable durable workflow runs.
    #[serde(default)]
    pub workflow_mode: crate::workflow_mode::WorkflowMode,
    /// Per-session permission mode (`default` / `smart` / `yolo`).
    /// Persisted so the chat title bar's mode switcher is restored when
    /// switching back to a historical session. Serialized as a snake_case
    /// string, matching the frontend `SessionMode` union.
    #[serde(default)]
    pub permission_mode: SessionMode,
    /// Per-session sandbox mode (`off` / `standard` / `isolated` /
    /// `workspace` / `trusted`). This affects where tools execute and may
    /// reduce soft approval prompts without changing YOLO / Smart mode.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,
    /// If this session belongs to a project, stores the project ID.
    /// Project-scoped memories and files are shared across all sessions in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// If this session is linked to an IM channel conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_info: Option<ChannelSessionInfo>,
    /// When true, this session runs in incognito mode: no passive memory or
    /// awareness injection, and no automatic memory extraction.
    #[serde(default)]
    pub incognito: bool,
    /// User-selected working directory for this session. When set, the path
    /// is injected into the system prompt so the model treats it as the
    /// default directory for file operations. On server mode the path refers
    /// to the server machine's filesystem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Session classification (see [`SessionKind`]). `Regular` for normal
    /// chats; `Knowledge` for knowledge-space sidebar conversations and
    /// `EvalFixture` for synthetic eval/smoke runs (both hidden from the main
    /// sidebar / picker).
    #[serde(default)]
    pub kind: SessionKind,
}

/// The next regular unread conversation that the sidebar should reveal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnreadSessionTarget {
    pub session_id: String,
    pub project_id: Option<String>,
    /// Zero-based position inside the target's sidebar session group using the
    /// same pin/update ordering as the list endpoint.
    pub list_offset: u32,
}

fn default_title_source() -> String {
    crate::session_title::TITLE_SOURCE_MANUAL.to_string()
}

impl SessionMeta {
    /// True iff this is a normal user-facing conversation — what the desktop
    /// shell should surface in cross-cutting places like the tray dropdown.
    /// Excludes cron-triggered sessions (autonomous), sub-agent children
    /// (parent owns the UX), IM channel conversations (handled by the IM
    /// worker, not the desktop), and incognito sessions (intentionally
    /// invisible). Project membership is allowed — project chats are still
    /// user chats, just organized inside a project container.
    pub fn is_regular_chat(&self) -> bool {
        !self.is_cron
            && self.parent_session_id.is_none()
            && self.channel_info.is_none()
            && !self.incognito
            && self.kind == SessionKind::Regular
    }
}

/// Lightweight channel info attached to a session for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSessionInfo {
    pub channel_id: String,
    pub account_id: String,
    pub chat_id: String,
    pub chat_type: String,
    pub sender_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Event,
    Tool,
    /// Intermediate text block emitted before tool calls to preserve ordering.
    TextBlock,
    /// Intermediate thinking block emitted before tool calls to preserve multi-round thinking ordering.
    ThinkingBlock,
}

impl MessageRole {
    pub fn as_str(&self) -> &str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Event => "event",
            MessageRole::Tool => "tool",
            MessageRole::TextBlock => "text_block",
            MessageRole::ThinkingBlock => "thinking_block",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "event" => MessageRole::Event,
            "tool" => MessageRole::Tool,
            "text_block" => MessageRole::TextBlock,
            "thinking_block" => MessageRole::ThinkingBlock,
            _ => MessageRole::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    // User message fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments_meta: Option<String>, // see ATTACHMENT_META_KEY_* below for the well-known keys
    // Assistant message fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    // Tool call fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_arguments: Option<String>, // JSON string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Time to first token in milliseconds (from API request to first content token)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<i64>,
    /// Last-round input tokens. See `ChatUsage::last_input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_in_last: Option<i64>,
    /// Cache-creation input tokens (Anthropic prompt cache write).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_cache_creation: Option<i64>,
    /// Cache-read input tokens (Anthropic prompt cache hit / OpenAI
    /// `input_tokens_details.cached_tokens` / `prompt_tokens_details.cached_tokens`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_cache_read: Option<i64>,
    /// Structured tool side-output JSON (e.g. file change before/after
    /// snapshots, line deltas). `None` for non-tool rows or when the tool
    /// produced no metadata. The frontend parses this to render the right
    /// side diff panel + `+N -M` summaries in tool call headers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_metadata: Option<String>,
    /// Streaming persistence state for thinking_block / text_block rows that
    /// were inserted incrementally (placeholder + throttled UPDATE) before the
    /// turn finalized. `streaming` = currently being written; `completed` =
    /// finalized cleanly; `orphaned` = a previous run died mid-stream and still
    /// needs startup finalize; `recovered` = startup finalize already preserved
    /// that interrupted partial. `None` covers legacy rows pre-migration and is
    /// treated as `completed` by readers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_status: Option<String>,
    /// Durable stream run that materialized this row. Exposed additively so a
    /// reload handshake can replace the current run's checkpoint projection
    /// with its authoritative journal snapshot without duplicate rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistence_run_id: Option<String>,
}

// ── NewMessage (for inserting) ───────────────────────────────────

/// A new message to be inserted (without auto-generated id).
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    pub attachments_meta: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub reasoning_effort: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub tool_result: Option<String>,
    pub tool_duration_ms: Option<i64>,
    pub is_error: Option<bool>,
    pub thinking: Option<String>,
    pub ttft_ms: Option<i64>,
    pub tokens_in_last: Option<i64>,
    pub tokens_cache_creation: Option<i64>,
    pub tokens_cache_read: Option<i64>,
    /// JSON string with structured tool side-output (see
    /// [`SessionMessage::tool_metadata`]).
    pub tool_metadata: Option<String>,
    /// Initial stream_status for crash-resilient placeholder rows (see
    /// [`SessionMessage::stream_status`]). Default `None` for normal rows.
    pub stream_status: Option<String>,
    /// Lowercase `ChatSource::as_str()` of the caller that drove the turn
    /// this message belongs to. `None` for legacy rows + helper paths that
    /// have no canonical source; readers fall back to `desktop` so old
    /// unread badges aren't disturbed.
    pub source: Option<String>,
    /// Durable queue id that produced this user row. A partial unique index
    /// makes queue recovery exactly-once across process crashes.
    pub queue_request_id: Option<String>,
    /// Internal durability run which materialized this row. `None` for legacy
    /// and non-streaming writes.
    pub persistence_run_id: Option<String>,
    /// Stable logical block sequence within `persistence_run_id`. Together
    /// with the run id this is an idempotency key for crash replay.
    pub logical_block_seq: Option<i64>,
}

impl NewMessage {
    /// Create a simple user message.
    pub fn user(content: &str) -> Self {
        Self {
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
            source: None,
            queue_request_id: None,
            persistence_run_id: None,
            logical_block_seq: None,
        }
    }

    /// Create a simple assistant message.
    pub fn assistant(content: &str) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
            source: None,
            queue_request_id: None,
            persistence_run_id: None,
            logical_block_seq: None,
        }
    }

    /// Create a tool call/result message.
    ///
    /// Defaults `stream_status` to `'streaming'` so a crash between INSERT
    /// (tool_call) and UPDATE (tool_result, → `'completed'`) leaves the row
    /// recognizable by the startup orphan sweep +
    /// [`crate::chat_engine::context::inject_orphaned_partial_summary`].
    pub fn tool(
        call_id: &str,
        name: &str,
        arguments: &str,
        result: &str,
        duration_ms: Option<i64>,
        is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            content: String::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: Some(call_id.to_string()),
            tool_name: Some(name.to_string()),
            tool_arguments: Some(arguments.to_string()),
            tool_result: Some(result.to_string()),
            tool_duration_ms: duration_ms,
            is_error: Some(is_error),
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: Some("streaming".to_string()),
            source: None,
            queue_request_id: None,
            persistence_run_id: None,
            logical_block_seq: None,
        }
    }

    /// Create a text_block message (intermediate text before tool calls).
    pub fn text_block(content: &str) -> Self {
        Self {
            role: MessageRole::TextBlock,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
            source: None,
            queue_request_id: None,
            persistence_run_id: None,
            logical_block_seq: None,
        }
    }

    /// Create a thinking_block message (intermediate thinking before tool calls).
    pub fn thinking_block(content: &str) -> Self {
        Self::thinking_block_with_duration(content, None)
    }

    /// Create a thinking_block message with an optional duration in milliseconds.
    pub fn thinking_block_with_duration(content: &str, duration_ms: Option<i64>) -> Self {
        Self {
            role: MessageRole::ThinkingBlock,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: duration_ms,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
            source: None,
            queue_request_id: None,
            persistence_run_id: None,
            logical_block_seq: None,
        }
    }

    /// Create an event message (e.g. errors, model fallback notifications).
    pub fn event(content: &str) -> Self {
        Self {
            role: MessageRole::Event,
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
            source: None,
            queue_request_id: None,
            persistence_run_id: None,
            logical_block_seq: None,
        }
    }

    /// Create an event row that should be surfaced as an error marker.
    pub fn error_event(content: &str) -> Self {
        let mut msg = Self::event(content);
        msg.is_error = Some(true);
        msg
    }

    /// Attach a JSON-string `tool_metadata` payload to this message. Returns
    /// `self` for builder chaining; passing `None` is a no-op.
    pub fn with_tool_metadata(mut self, metadata: Option<String>) -> Self {
        self.tool_metadata = metadata;
        self
    }

    /// Tag this message with the [`crate::chat_engine::stream_seq::ChatSource`]
    /// that drove the turn. Builder-style so callers can write
    /// `NewMessage::user("…").with_source(ChatSource::Channel)`.
    pub fn with_source(mut self, source: crate::chat_engine::stream_seq::ChatSource) -> Self {
        self.source = Some(source.as_str().to_string());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            title: None,
            title_source: default_title_source(),
            agent_id: crate::agent_loader::DEFAULT_AGENT_ID.to_string(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            temperature: None,
            reasoning_effort: None,
            runtime_defaults_initialized: false,
            created_at: "2026-05-01T00:00:00Z".to_string(),
            updated_at: "2026-05-01T00:00:00Z".to_string(),
            pinned_at: None,
            message_count: 0,
            unread_count: 0,
            channel_unread_count: 0,
            has_error: false,
            pending_interaction_count: 0,
            pending_countdown: None,
            is_cron: false,
            parent_session_id: None,
            forked_from_session_id: None,
            forked_from_message_id: None,
            forked_from_session_title: None,
            plan_mode: Default::default(),
            execution_mode: Default::default(),
            workflow_mode: Default::default(),
            permission_mode: Default::default(),
            sandbox_mode: Default::default(),
            project_id: None,
            channel_info: None,
            incognito: false,
            working_dir: None,
            kind: SessionKind::Regular,
        }
    }

    #[test]
    fn is_regular_chat_excludes_non_regular_kinds() {
        assert!(meta("a").is_regular_chat());

        let mut cron = meta("b");
        cron.is_cron = true;
        assert!(!cron.is_regular_chat());

        let mut sub = meta("c");
        sub.parent_session_id = Some("parent".to_string());
        assert!(!sub.is_regular_chat());

        // Project membership is allowed — project conversations are still
        // user-facing chats and should surface in the tray etc.
        let mut proj = meta("d");
        proj.project_id = Some("p1".to_string());
        assert!(proj.is_regular_chat());

        let mut im = meta("e");
        im.channel_info = Some(ChannelSessionInfo {
            channel_id: "discord".to_string(),
            account_id: "acc".to_string(),
            chat_id: "ch".to_string(),
            chat_type: "channel".to_string(),
            sender_name: None,
        });
        assert!(!im.is_regular_chat());

        let mut inc = meta("f");
        inc.incognito = true;
        assert!(!inc.is_regular_chat());

        // Knowledge-space sidebar conversations are persisted but never surface
        // in the main session list / picker.
        let mut kb = meta("g");
        kb.kind = SessionKind::Knowledge;
        assert!(!kb.is_regular_chat());

        let mut fixture = meta("h");
        fixture.kind = SessionKind::EvalFixture;
        assert!(!fixture.is_regular_chat());
    }

    #[test]
    fn session_kind_roundtrips_and_defaults() {
        assert_eq!(SessionKind::Regular.as_str(), "regular");
        assert_eq!(SessionKind::Knowledge.as_str(), "knowledge");
        assert_eq!(SessionKind::EvalFixture.as_str(), "eval_fixture");
        assert_eq!(
            SessionKind::from_db_string("knowledge"),
            SessionKind::Knowledge
        );
        assert_eq!(
            SessionKind::from_db_string("eval_fixture"),
            SessionKind::EvalFixture
        );
        // Unknown / legacy NULL coerced to "" → Regular.
        assert_eq!(SessionKind::from_db_string(""), SessionKind::Regular);
        assert_eq!(SessionKind::from_db_string("bogus"), SessionKind::Regular);
        assert_eq!(SessionKind::default(), SessionKind::Regular);

        // serde uses snake_case and round-trips through SessionMeta.
        let mut m = meta("k");
        m.kind = SessionKind::Knowledge;
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"kind\":\"knowledge\""));
        let back: SessionMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, SessionKind::Knowledge);

        m.kind = SessionKind::EvalFixture;
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"kind\":\"eval_fixture\""));
        let back: SessionMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, SessionKind::EvalFixture);
    }
}
