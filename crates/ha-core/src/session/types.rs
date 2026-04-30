use serde::{Deserialize, Serialize};

fn default_permission_mode() -> String {
    "default".to_string()
}

// ── Data Structures ──────────────────────────────────────────────

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
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
    pub unread_count: i64,
    /// Number of pending interactions waiting on the user for this session
    /// (sum of pending tool approvals + pending ask_user_question groups).
    /// Populated at the command/route layer, not in `list_sessions_paged`.
    #[serde(default)]
    pub pending_interaction_count: i64,
    pub is_cron: bool,
    /// If this session was created by a sub-agent spawn, stores the parent session ID.
    pub parent_session_id: Option<String>,
    /// Plan mode state for this session: "off" | "planning" | "executing"
    pub plan_mode: String,
    /// Per-session permission mode: "default" | "smart" | "yolo".
    /// Persisted so the chat title bar's mode switcher is restored when
    /// switching back to a historical session.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
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
}

fn default_title_source() -> String {
    crate::session_title::TITLE_SOURCE_MANUAL.to_string()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub attachments_meta: Option<String>, // JSON array of {name, mime_type, size}
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
        }
    }

    /// Create a tool call/result message.
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
        }
    }

    /// Attach a JSON-string `tool_metadata` payload to this message. Returns
    /// `self` for builder chaining; passing `None` is a no-op.
    pub fn with_tool_metadata(mut self, metadata: Option<String>) -> Self {
        self.tool_metadata = metadata;
        self
    }
}
