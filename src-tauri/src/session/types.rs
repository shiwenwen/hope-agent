use serde::{Deserialize, Serialize};

// ── Data Structures ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub title: Option<String>,
    pub agent_id: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
    pub unread_count: i64,
    pub is_cron: bool,
    /// If this session was created by a sub-agent spawn, stores the parent session ID.
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Event,
    Tool,
    /// Intermediate text block emitted before tool calls to preserve ordering.
    TextBlock,
}

impl MessageRole {
    pub fn as_str(&self) -> &str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Event => "event",
            MessageRole::Tool => "tool",
            MessageRole::TextBlock => "text_block",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "event" => MessageRole::Event,
            "tool" => MessageRole::Tool,
            "text_block" => MessageRole::TextBlock,
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
        }
    }

    /// Create a tool call/result message.
    pub fn tool(call_id: &str, name: &str, arguments: &str, result: &str, duration_ms: Option<i64>, is_error: bool) -> Self {
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
        }
    }
}
