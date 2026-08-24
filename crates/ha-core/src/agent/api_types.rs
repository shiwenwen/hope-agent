use serde::{Deserialize, Serialize};

// ── OpenAI Responses API types ────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ReasoningConfig {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub store: bool,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_options: Option<serde_json::Value>,
}

/// Tracks a function_call being accumulated from SSE events
#[derive(Debug, Clone)]
pub struct FunctionCallItem {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

// ── SSE event types for streaming response ────────────────────────

#[derive(Deserialize, Debug)]
pub struct SseEvent {
    #[serde(rename = "type", default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub response: Option<SseResponseObj>,
    #[serde(default)]
    pub item: Option<SseOutputItem>,
    // For error events
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<SseResponseError>,
}

#[derive(Deserialize, Debug)]
pub struct SseResponseObj {
    #[serde(default)]
    pub output: Option<Vec<SseOutputItem>>,
    #[serde(default)]
    pub error: Option<SseResponseError>,
    #[serde(default)]
    pub usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug, Default)]
pub struct SseTokenDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
}

#[derive(Deserialize, Debug, Default)]
pub struct SseUsage {
    #[serde(default, alias = "prompt_tokens")]
    pub input_tokens: Option<u64>,
    #[serde(default, alias = "completion_tokens")]
    pub output_tokens: Option<u64>,
    // Anthropic cache tokens
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    // OpenAI Responses API: input_tokens_details.cached_tokens
    #[serde(default)]
    pub input_tokens_details: Option<SseTokenDetails>,
    // OpenAI Chat Completions / Codex backend: prompt_tokens_details.cached_tokens
    #[serde(default)]
    pub prompt_tokens_details: Option<SseTokenDetails>,
}

#[derive(Deserialize, Debug)]
pub struct SseResponseError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(rename = "type", default)]
    pub error_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct SseOutputItem {
    #[serde(rename = "type", default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
    #[serde(default)]
    pub content: Option<Vec<ContentPart>>,
}

#[derive(Deserialize, Debug)]
pub struct ContentPart {
    #[serde(rename = "type", default)]
    pub part_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

// ── Error parsing types ───────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct ApiErrorResponse {
    #[serde(default)]
    pub error: Option<ApiErrorDetail>,
    #[serde(default)]
    pub detail: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
pub struct ApiErrorDetail {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub resets_at: Option<f64>,
    #[serde(rename = "type", default)]
    pub error_type: Option<String>,
}

// ── Anthropic Messages API types ──────────────────────────────────

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct AnthropicSseEvent {
    #[serde(rename = "type", default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub content_block: Option<AnthropicContentBlock>,
    #[serde(default)]
    pub delta: Option<AnthropicDelta>,
    #[serde(default)]
    pub message: Option<AnthropicMessage>,
    #[serde(default)]
    pub error: Option<AnthropicError>,
    #[serde(default)]
    pub usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type", default)]
    pub block_type: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicDelta {
    #[serde(rename = "type", default)]
    pub delta_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub partial_json: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct AnthropicMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<AnthropicContentBlock>>,
    #[serde(default)]
    pub usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct AnthropicError {
    #[serde(rename = "type", default)]
    pub error_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}
