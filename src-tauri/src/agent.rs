use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::file_extract;
use crate::provider::{ApiType, ProviderConfig, ThinkingStyle};
use crate::tools::{self, ToolProvider};

/// File/image attachment sent alongside a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime_type: String,
    /// Base64-encoded file data (used for images — passed directly through IPC)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Absolute path to the file on disk (used for non-image files)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

impl Attachment {
    /// Get base64-encoded data: use `data` field if present, otherwise read from `file_path`.
    fn get_base64_data(&self) -> Result<String> {
        if let Some(ref data) = self.data {
            return Ok(data.clone());
        }
        if let Some(ref path) = self.file_path {
            return read_and_encode_base64(path);
        }
        Err(anyhow::anyhow!("Attachment '{}' has neither data nor file_path", self.name))
    }
}
/// Read a file from disk and return its contents as a base64-encoded string.
fn read_and_encode_base64(path: &str) -> Result<String> {
    let data = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("Failed to read attachment '{}': {}", path, e))?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&data))
}

/// Process non-image attachments: extract text and images from files (PDF, Word, Excel, PPT, text).
/// Returns (extra_text to append to message, extra_images as base64 tuples).
fn process_file_attachments(attachments: &[Attachment]) -> (String, Vec<file_extract::ExtractedImage>) {
    let mut file_texts = Vec::new();
    let mut extra_images = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/") {
            continue; // Images are handled as multimodal content blocks
        }
        let file_path = match &att.file_path {
            Some(p) => p.as_str(),
            None => continue,
        };

        let content = file_extract::extract(file_path, &att.name, &att.mime_type);

        // Build <file> XML block with path (always present)
        let text_block = match &content.text {
            Some(text) => format!(
                "<file name=\"{}\" path=\"{}\">\n{}\n</file>",
                content.file_name, content.file_path, text
            ),
            None => format!(
                "<file name=\"{}\" path=\"{}\">\n[Binary file. Use tools to inspect if needed.]\n</file>",
                content.file_name, content.file_path
            ),
        };
        file_texts.push(text_block);

        // Collect extracted images (PDF pages, PPT media, etc.)
        extra_images.extend(content.images);
    }

    let extra_text = if file_texts.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", file_texts.join("\n\n"))
    };

    (extra_text, extra_images)
}

/// Build multimodal user content array for Anthropic Messages API.
fn build_user_content_anthropic(message: &str, attachments: &[Attachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    // Check if we have any images (original image attachments + extracted images)
    let has_images = attachments.iter().any(|a| a.mime_type.starts_with("image/"))
        || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    // Original image attachments
    for att in attachments {
        if att.mime_type.starts_with("image/") {
            match att.get_base64_data() {
                Ok(b64) => {
                    parts.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": att.mime_type,
                            "data": b64,
                        }
                    }));
                }
                Err(e) => {
                    log::warn!("Skipping attachment {}: {}", att.name, e);
                }
            }
        }
    }

    // Extracted images (PDF pages, PPT media, etc.)
    for img in &extra_images {
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.mime_type,
                "data": img.data,
            }
        }));
    }

    parts.push(json!({ "type": "text", "text": full_message }));
    json!(parts)
}

/// Build multimodal user content array for OpenAI Chat Completions API.
fn build_user_content_openai_chat(message: &str, attachments: &[Attachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments.iter().any(|a| a.mime_type.starts_with("image/"))
        || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/") {
            match att.get_base64_data() {
                Ok(b64) => {
                    let data_url = format!("data:{};base64,{}", att.mime_type, b64);
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": data_url }
                    }));
                }
                Err(e) => {
                    log::warn!("Skipping attachment {}: {}", att.name, e);
                }
            }
        }
    }

    for img in &extra_images {
        let data_url = format!("data:{};base64,{}", img.mime_type, img.data);
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": data_url }
        }));
    }

    parts.push(json!({ "type": "text", "text": full_message }));
    json!(parts)
}

/// Build multimodal user content array for OpenAI Responses API / Codex.
fn build_user_content_responses(message: &str, attachments: &[Attachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments.iter().any(|a| a.mime_type.starts_with("image/"))
        || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/") {
            match att.get_base64_data() {
                Ok(b64) => {
                    let data_url = format!("data:{};base64,{}", att.mime_type, b64);
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": data_url,
                    }));
                }
                Err(e) => {
                    log::warn!("Skipping attachment {}: {}", att.name, e);
                }
            }
        }
    }

    for img in &extra_images {
        let data_url = format!("data:{};base64,{}", img.mime_type, img.data);
        parts.push(json!({
            "type": "input_image",
            "image_url": data_url,
        }));
    }

    parts.push(json!({ "type": "input_text", "text": full_message }));
    json!(parts)
}

/// Build the full system prompt.
/// Uses the new system_prompt module with AgentDefinition if available,
/// otherwise falls back to legacy behavior for backward compatibility.
fn build_system_prompt() -> String {
    // Try loading the current agent definition
    if let Ok(definition) = crate::agent_loader::load_agent("default") {
        return crate::system_prompt::build(&definition);
    }
    // Fallback: legacy prompt
    crate::system_prompt::build_legacy()
}

const CODEX_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
#[allow(dead_code)]
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// User-Agent header for all outgoing HTTP requests.
/// Some API providers (e.g. DashScope CodingPlan) use WAF rules that filter
/// requests based on User-Agent. Using a recognized coding-tool-style UA
/// ensures compatibility with these services.
pub const USER_AGENT: &str = "OpenComputer/1.0";

/// Smart URL builder: if base_url already ends with a version suffix
/// (e.g. /v1, /v2, /v3), strip the version prefix from path to avoid
/// double-prefixing like /v3/v1/chat/completions.
pub fn build_api_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let version_prefixes = ["/v1", "/v2", "/v3"];

    // Check if base already has any version suffix
    let base_has_version = version_prefixes.iter().any(|p| base.ends_with(p));

    if base_has_version {
        // Strip version prefix from path if present
        for prefix in &version_prefixes {
            if path.starts_with(prefix) {
                return format!("{}{}", base, &path[prefix.len()..]);
            }
        }
    }

    format!("{}{}", base, path)
}
#[allow(dead_code)]
const ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1000;
const DEFAULT_MAX_TOOL_ROUNDS: u32 = 10;

/// Get the configured max tool rounds from the current agent.
/// Returns 0 for unlimited.
fn get_max_tool_rounds() -> u32 {
    crate::agent_loader::load_agent("default")
        .map(|def| def.config.behavior.max_tool_rounds)
        .unwrap_or(DEFAULT_MAX_TOOL_ROUNDS)
}

// ── Codex model definitions ───────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct CodexModel {
    pub id: String,
    pub name: String,
}

pub fn get_codex_models() -> Vec<CodexModel> {
    vec![
        CodexModel { id: "gpt-5.4".into(), name: "GPT-5.4".into() },
        CodexModel { id: "gpt-5.3-codex".into(), name: "GPT-5.3 Codex".into() },
        CodexModel { id: "gpt-5.3-codex-spark".into(), name: "GPT-5.3 Codex Spark".into() },
        CodexModel { id: "gpt-5.2".into(), name: "GPT-5.2".into() },
        CodexModel { id: "gpt-5.2-codex".into(), name: "GPT-5.2 Codex".into() },
        CodexModel { id: "gpt-5.1".into(), name: "GPT-5.1".into() },
        CodexModel { id: "gpt-5.1-codex-max".into(), name: "GPT-5.1 Codex Max".into() },
        CodexModel { id: "gpt-5.1-codex-mini".into(), name: "GPT-5.1 Codex Mini".into() },
    ]
}

/// Clamp reasoning effort to valid range for the given model
pub fn clamp_reasoning_effort(model: &str, effort: &str) -> Option<String> {
    if effort == "none" {
        return None;
    }
    let efforts = ["low", "medium", "high", "xhigh"];
    if !efforts.contains(&effort) {
        return Some("medium".to_string());
    }
    if model.contains("5.1-codex-mini") {
        return match effort {
            "low" => Some("medium".to_string()),
            "xhigh" => Some("high".to_string()),
            _ => Some(effort.to_string()),
        };
    }
    if model.contains("5.1") {
        return match effort {
            "xhigh" => Some("high".to_string()),
            _ => Some(effort.to_string()),
        };
    }
    Some(effort.to_string())
}

/// Map reasoning effort to Anthropic/ZAI thinking parameter.
/// Anthropic/ZAI uses `thinking: { type: "enabled", budget_tokens: N }` format.
/// Returns None if thinking should be disabled.
fn map_think_anthropic_style(effort: Option<&str>, max_tokens: u32) -> Option<serde_json::Value> {
    let effort = effort?;
    if effort == "none" {
        return None;
    }
    // Map effort level to budget_tokens
    let budget: u32 = match effort {
        "low" => 1024,
        "medium" => 4096,
        "high" => 8192,
        "xhigh" => 16384,
        _ => return None,
    };
    // Anthropic requires budget_tokens < max_tokens specified in request
    let capped_budget = budget.min(max_tokens.saturating_sub(1));
    Some(json!({
        "type": "enabled",
        "budget_tokens": capped_budget
    }))
}

/// Map reasoning effort to OpenAI `reasoning_effort` parameter.
/// Chat Completions supports "low", "medium", "high" (no xhigh).
/// Returns None if thinking should be disabled.
fn map_think_openai_style(effort: Option<&str>) -> Option<String> {
    let effort = effort?;
    match effort {
        "none" => None,
        "xhigh" => Some("high".to_string()), // Downgrade xhigh to high
        "low" | "medium" | "high" => Some(effort.to_string()),
        _ => None,
    }
}

/// Map reasoning effort to Qwen `enable_thinking` parameter.
/// Returns None if thinking should be disabled.
fn map_think_qwen_style(effort: Option<&str>) -> Option<bool> {
    let effort = effort?;
    match effort {
        "none" => Some(false),
        "low" | "medium" | "high" | "xhigh" => Some(true),
        _ => None,
    }
}

/// Apply thinking parameters to an OpenAI Chat Completions body based on ThinkingStyle.
fn apply_thinking_to_chat_body(
    body: &mut serde_json::Value,
    thinking_style: &ThinkingStyle,
    reasoning_effort: Option<&str>,
    max_tokens: u32,
) {
    match thinking_style {
        ThinkingStyle::Openai => {
            if let Some(effort) = map_think_openai_style(reasoning_effort) {
                body["reasoning_effort"] = json!(effort);
            }
        }
        ThinkingStyle::Anthropic | ThinkingStyle::Zai => {
            if let Some(think_config) = map_think_anthropic_style(reasoning_effort, max_tokens) {
                body["thinking"] = think_config;
            }
        }
        ThinkingStyle::Qwen => {
            if let Some(enable) = map_think_qwen_style(reasoning_effort) {
                body["enable_thinking"] = json!(enable);
            }
        }
        ThinkingStyle::None => {
            // Do not send any thinking/reasoning parameters
        }
    }
}

/// Supported LLM providers
pub enum LlmProvider {
    /// Anthropic Messages API
    Anthropic { api_key: String, base_url: String, model: String },
    /// OpenAI Chat Completions API (/v1/chat/completions)
    OpenAIChat { api_key: String, base_url: String, model: String },
    /// OpenAI Responses API (/v1/responses)
    OpenAIResponses { api_key: String, base_url: String, model: String },
    /// Built-in Codex OAuth (ChatGPT subscription)
    Codex { access_token: String, account_id: String, model: String },
}

pub struct AssistantAgent {
    provider: LlmProvider,
    /// Custom User-Agent header for API requests
    user_agent: String,
    /// Thinking/reasoning parameter format
    thinking_style: ThinkingStyle,
    /// Conversation history persisted across chat() calls
    conversation_history: std::sync::Mutex<Vec<serde_json::Value>>,
}

// ── Shared Event Types (sent to frontend via on_delta JSON) ───────

/// Emit a JSON event string via the on_delta callback
fn emit_event(on_delta: &(impl Fn(&str) + Send), event: &serde_json::Value) {
    if let Ok(json_str) = serde_json::to_string(event) {
        on_delta(&json_str);
    }
}

fn emit_text_delta(on_delta: &(impl Fn(&str) + Send), text: &str) {
    emit_event(on_delta, &json!({
        "type": "text_delta",
        "content": text,
    }));
}

fn emit_tool_call(on_delta: &(impl Fn(&str) + Send), call_id: &str, name: &str, arguments: &str) {
    emit_event(on_delta, &json!({
        "type": "tool_call",
        "call_id": call_id,
        "name": name,
        "arguments": arguments,
    }));
}

fn emit_tool_result(on_delta: &(impl Fn(&str) + Send), call_id: &str, result: &str) {
    emit_event(on_delta, &json!({
        "type": "tool_result",
        "call_id": call_id,
        "result": result,
    }));
}

fn emit_thinking_delta(on_delta: &(impl Fn(&str) + Send), text: &str) {
    emit_event(on_delta, &json!({
        "type": "thinking_delta",
        "content": text,
    }));
}

/// Token usage accumulated across tool rounds
#[derive(Debug, Clone, Default)]
pub struct ChatUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

fn emit_usage(on_delta: &(impl Fn(&str) + Send), usage: &ChatUsage) {
    emit_event(on_delta, &json!({
        "type": "usage",
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
    }));
}

// ── OpenAI Responses API types ────────────────────────────────────

#[derive(Serialize)]
struct ReasoningConfig {
    effort: String,
}

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    store: bool,
    stream: bool,
    instructions: String,
    input: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

/// Tracks a function_call being accumulated from SSE events
#[derive(Debug, Clone)]
struct FunctionCallItem {
    call_id: String,
    name: String,
    arguments: String,
}

// ── SSE event types for streaming response ────────────────────────

#[derive(Deserialize, Debug)]
struct SseEvent {
    #[serde(rename = "type", default)]
    event_type: Option<String>,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    response: Option<SseResponseObj>,
    #[serde(default)]
    item: Option<SseOutputItem>,
    // For error events
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize, Debug)]
struct SseResponseObj {
    #[serde(default)]
    output: Option<Vec<SseOutputItem>>,
    #[serde(default)]
    error: Option<SseResponseError>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug, Default)]
struct SseUsage {
    #[serde(default, alias = "prompt_tokens")]
    input_tokens: Option<u64>,
    #[serde(default, alias = "completion_tokens")]
    output_tokens: Option<u64>,
    // Anthropic cache tokens
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct SseResponseError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize, Debug)]
struct SseOutputItem {
    #[serde(rename = "type", default)]
    item_type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    content: Option<Vec<ContentPart>>,
}

#[derive(Deserialize, Debug)]
struct ContentPart {
    #[serde(rename = "type", default)]
    part_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

// ── Error parsing types ───────────────────────────────────────────

#[derive(Deserialize, Default)]
struct ApiErrorResponse {
    #[serde(default)]
    error: Option<ApiErrorDetail>,
    #[serde(default)]
    detail: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct ApiErrorDetail {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    resets_at: Option<f64>,
    #[serde(rename = "type", default)]
    error_type: Option<String>,
}

// ── Anthropic Messages API types ──────────────────────────────────

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct AnthropicSseEvent {
    #[serde(rename = "type", default)]
    event_type: Option<String>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content_block: Option<AnthropicContentBlock>,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    #[serde(default)]
    message: Option<AnthropicMessage>,
    #[serde(default)]
    error: Option<AnthropicError>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct AnthropicContentBlock {
    #[serde(rename = "type", default)]
    block_type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct AnthropicDelta {
    #[serde(rename = "type", default)]
    delta_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct AnthropicMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    content: Option<Vec<AnthropicContentBlock>>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct AnthropicError {
    #[serde(rename = "type", default)]
    error_type: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

// ── AssistantAgent ────────────────────────────────────────────────

impl AssistantAgent {
    /// Create agent with Anthropic API key (legacy, uses default base_url and model)
    #[allow(dead_code)]
    pub fn new_anthropic(api_key: &str) -> Self {
        Self {
            provider: LlmProvider::Anthropic {
                api_key: api_key.to_string(),
                base_url: ANTHROPIC_API_URL.trim_end_matches("/v1/messages").to_string(),
                model: ANTHROPIC_MODEL.to_string(),
            },
            user_agent: USER_AGENT.to_string(),
            thinking_style: ThinkingStyle::Anthropic,
            conversation_history: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Create agent with OpenAI-compatible access token (Codex OAuth)
    pub fn new_openai(access_token: &str, account_id: &str, model: &str) -> Self {
        Self {
            provider: LlmProvider::Codex {
                access_token: access_token.to_string(),
                account_id: account_id.to_string(),
                model: model.to_string(),
            },
            user_agent: USER_AGENT.to_string(),
            thinking_style: ThinkingStyle::Openai,
            conversation_history: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Create agent from a ProviderConfig and a specific model ID
    pub fn new_from_provider(config: &ProviderConfig, model_id: &str) -> Self {
        let provider = match config.api_type {
            ApiType::Anthropic => LlmProvider::Anthropic {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiChat => LlmProvider::OpenAIChat {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiResponses => LlmProvider::OpenAIResponses {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::Codex => LlmProvider::Codex {
                access_token: config.api_key.clone(),
                account_id: String::new(),
                model: model_id.to_string(),
            },
        };
        Self {
            provider,
            user_agent: config.user_agent.clone(),
            thinking_style: config.thinking_style.clone(),
            conversation_history: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Replace the conversation history (used to restore context from DB).
    pub fn set_conversation_history(&self, history: Vec<serde_json::Value>) {
        *self.conversation_history.lock().unwrap() = history;
    }

    /// Get a clone of the current conversation history (used to persist context to DB).
    pub fn get_conversation_history(&self) -> Vec<serde_json::Value> {
        self.conversation_history.lock().unwrap().clone()
    }

    /// Push a user message, merging with the last message if it's also a user message.
    /// This avoids consecutive user messages which Anthropic API rejects.
    fn push_user_message(messages: &mut Vec<serde_json::Value>, new_content: serde_json::Value) {
        if let Some(last) = messages.last_mut() {
            if last.get("role").and_then(|r| r.as_str()) == Some("user") {
                // Merge into existing user message
                let old_content = last.get("content").cloned();
                let merged = match (old_content, &new_content) {
                    (Some(serde_json::Value::String(old)), serde_json::Value::String(new)) => {
                        serde_json::Value::String(format!("{}\n\n{}", old, new))
                    }
                    (Some(serde_json::Value::Array(mut old_arr)), serde_json::Value::Array(new_arr)) => {
                        old_arr.extend(new_arr.iter().cloned());
                        serde_json::Value::Array(old_arr)
                    }
                    (Some(serde_json::Value::Array(mut old_arr)), serde_json::Value::String(s)) => {
                        old_arr.push(json!({"type": "text", "text": s}));
                        serde_json::Value::Array(old_arr)
                    }
                    (Some(serde_json::Value::String(old)), serde_json::Value::Array(new_arr)) => {
                        let mut arr = vec![json!({"type": "text", "text": old})];
                        arr.extend(new_arr.iter().cloned());
                        serde_json::Value::Array(arr)
                    }
                    (_, _) => new_content.clone(),
                };
                last["content"] = merged;
                return;
            }
        }
        messages.push(json!({ "role": "user", "content": new_content }));
    }

    pub async fn chat(&self, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: Arc<AtomicBool>, on_delta: impl Fn(&str) + Send + 'static) -> Result<String> {
        match &self.provider {
            LlmProvider::Anthropic { api_key, base_url, model } => {
                self.chat_anthropic(api_key, base_url, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
            LlmProvider::OpenAIChat { api_key, base_url, model } => {
                self.chat_openai_chat(api_key, base_url, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
            LlmProvider::OpenAIResponses { api_key, base_url, model } => {
                self.chat_openai_responses(api_key, base_url, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
            LlmProvider::Codex { access_token, account_id, model } => {
                self.chat_openai(access_token, account_id, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
        }
    }

    // ── Anthropic Messages API with Tool Loop ─────────────────────

    async fn chat_anthropic(&self, api_key: &str, base_url: &str, model: &str, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let tool_schemas = tools::get_tools_for_provider(ToolProvider::Anthropic);

        // Build messages from conversation history + new user message (with optional image attachments)
        let mut messages = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_anthropic(message, attachments);
        Self::push_user_message(&mut messages, user_content);

        let mut collected_text = String::new();
        let mut total_usage = ChatUsage::default();

        let api_url = build_api_url(base_url, "/v1/messages");
        let system_prompt = build_system_prompt();

        // Map thinking effort for Anthropic
        let max_tokens: u32 = 16384;
        let thinking = map_think_anthropic_style(reasoning_effort, max_tokens);

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 { u32::MAX } else { max_rounds };
        for _round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) { break; }

            let mut body = json!({
                "model": model,
                "max_tokens": max_tokens,
                "system": system_prompt,
                "tools": tool_schemas,
                "messages": messages,
                "stream": true,
            });

            // Add thinking parameter if enabled
            if let Some(ref think_config) = thinking {
                body["thinking"] = think_config.clone();
            }

            let resp = client
                .post(&api_url)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_API_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Anthropic API request failed: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let error_text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("Anthropic API error ({}): {}", status, error_text));
            }

            // Parse SSE stream
            let (text, tool_calls, stop_reason, round_usage) = self.parse_anthropic_sse(resp, cancel, on_delta).await?;
            collected_text.push_str(&text);
            total_usage.input_tokens += round_usage.input_tokens;
            total_usage.output_tokens += round_usage.output_tokens;
            total_usage.cache_creation_input_tokens += round_usage.cache_creation_input_tokens;
            total_usage.cache_read_input_tokens += round_usage.cache_read_input_tokens;

            // If cancelled, no tool calls, or not tool_use stop reason — done
            if tool_calls.is_empty() || stop_reason.as_deref() != Some("tool_use") {
                break;
            }

            // Build assistant message with all content blocks
            let mut assistant_content: Vec<serde_json::Value> = Vec::new();
            if !text.is_empty() {
                assistant_content.push(json!({ "type": "text", "text": text }));
            }
            for tc in &tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                assistant_content.push(json!({
                    "type": "tool_use",
                    "id": tc.call_id,
                    "name": tc.name,
                    "input": args,
                }));
            }
            messages.push(json!({ "role": "assistant", "content": assistant_content }));

            // Execute tools and build tool_result messages
            let mut tool_results: Vec<serde_json::Value> = Vec::new();
            for tc in &tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);

                let result = match tools::execute_tool(&tc.name, &args).await {
                    Ok(r) => r,
                    Err(e) => format!("Tool error: {}", e),
                };

                emit_tool_result(on_delta, &tc.call_id, &result);

                tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tc.call_id,
                    "content": result,
                }));
            }
            messages.push(json!({ "role": "user", "content": tool_results }));
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from Anthropic API"));
        }

        // Persist conversation history (including partial response if cancelled)
        if !collected_text.is_empty() {
            messages.push(json!({ "role": "assistant", "content": collected_text }));
        }
        *self.conversation_history.lock().unwrap() = messages;

        // Emit accumulated usage
        emit_usage(on_delta, &total_usage);

        Ok(collected_text)
    }

    /// Parse Anthropic SSE stream. Returns (collected_text, tool_calls, stop_reason, usage)
    async fn parse_anthropic_sse(
        &self,
        resp: reqwest::Response,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>, Option<String>, ChatUsage)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
        // Track current content blocks by index
        let mut current_tool: Option<(usize, FunctionCallItem)> = None;
        let mut in_thinking_block = false;
        let mut usage = ChatUsage::default();
        let mut stop_reason: Option<String> = None;

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                stop_reason = Some("cancelled".to_string());
                break;
            }
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(idx) = buffer.find("\n\n") {
                let event_block = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                // Parse SSE event format: "event: <type>\ndata: <json>"
                let mut event_name = String::new();
                let mut data_lines = Vec::new();

                for line in event_block.lines() {
                    if let Some(ev) = line.strip_prefix("event:") {
                        event_name = ev.trim().to_string();
                    } else if let Some(d) = line.strip_prefix("data:") {
                        data_lines.push(d.trim().to_string());
                    }
                }

                if data_lines.is_empty() {
                    continue;
                }

                let data = data_lines.join("\n");
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }

                if let Ok(event) = serde_json::from_str::<AnthropicSseEvent>(&data) {
                    match event_name.as_str() {
                        "content_block_start" => {
                            if let Some(block) = &event.content_block {
                                match block.block_type.as_deref() {
                                    Some("tool_use") => {
                                        let idx = event.index.unwrap_or(0);
                                        current_tool = Some((idx, FunctionCallItem {
                                            call_id: block.id.clone().unwrap_or_default(),
                                            name: block.name.clone().unwrap_or_default(),
                                            arguments: String::new(),
                                        }));
                                    }
                                    Some("thinking") => {
                                        in_thinking_block = true;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = &event.delta {
                                match delta.delta_type.as_deref() {
                                    Some("thinking_delta") => {
                                        if let Some(text) = &delta.text {
                                            emit_thinking_delta(on_delta, text);
                                        }
                                    }
                                    Some("text_delta") => {
                                        if let Some(text) = &delta.text {
                                            emit_text_delta(on_delta, text);
                                            collected_text.push_str(text);
                                        }
                                    }
                                    Some("input_json_delta") => {
                                        if let Some(partial) = &delta.partial_json {
                                            if let Some((_, ref mut tc)) = current_tool {
                                                tc.arguments.push_str(partial);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "content_block_stop" => {
                            if in_thinking_block {
                                in_thinking_block = false;
                            }
                            if let Some((_, tc)) = current_tool.take() {
                                tool_calls.push(tc);
                            }
                        }
                        "message_start" => {
                            // Extract input_tokens + cache tokens from message.usage
                            if let Some(msg) = &event.message {
                                if let Some(u) = &msg.usage {
                                    if let Some(it) = u.input_tokens {
                                        usage.input_tokens = it;
                                    }
                                    if let Some(ct) = u.cache_creation_input_tokens {
                                        usage.cache_creation_input_tokens = ct;
                                    }
                                    if let Some(cr) = u.cache_read_input_tokens {
                                        usage.cache_read_input_tokens = cr;
                                    }
                                }
                            }
                        }
                        "message_delta" => {
                            if let Some(delta) = &event.delta {
                                if let Some(reason) = &delta.stop_reason {
                                    stop_reason = Some(reason.clone());
                                }
                            }
                            // Extract output_tokens from usage
                            if let Some(u) = &event.usage {
                                if let Some(ot) = u.output_tokens {
                                    usage.output_tokens = ot;
                                }
                            }
                        }
                        "error" => {
                            let msg = event.error
                                .as_ref()
                                .and_then(|e| e.message.as_deref())
                                .unwrap_or("Unknown Anthropic error");
                            return Err(anyhow::anyhow!("Anthropic error: {}", msg));
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok((collected_text, tool_calls, stop_reason, usage))
    }

    // ── OpenAI Chat Completions API with Tool Loop ───────────────

    async fn chat_openai_chat(&self, api_key: &str, base_url: &str, model: &str, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);

        let mut messages = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_openai_chat(message, attachments);
        Self::push_user_message(&mut messages, user_content);

        let api_url = build_api_url(base_url, "/v1/chat/completions");
        let mut collected_text = String::new();
        let mut total_usage = ChatUsage::default();
        let system_prompt = build_system_prompt();

        // Apply thinking parameters based on ThinkingStyle

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 { u32::MAX } else { max_rounds };
        for _round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) { break; }

            // Build messages array: system + conversation
            let mut api_messages = vec![json!({ "role": "system", "content": &system_prompt })];
            api_messages.extend(messages.iter().cloned());

            // Build tools array in Chat Completions format
            let tools_array: Vec<serde_json::Value> = tool_schemas.iter().map(|t| {
                json!({ "type": "function", "function": t })
            }).collect();

            let mut body = json!({
                "model": model,
                "messages": api_messages,
                "tools": tools_array,
                "stream": true,
                "stream_options": { "include_usage": true },
            });

            // Apply thinking parameters based on provider's ThinkingStyle
            apply_thinking_to_chat_body(&mut body, &self.thinking_style, reasoning_effort, 16384);

            let mut req = client
                .post(&api_url)
                .header("Content-Type", "application/json");
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }
            let resp = req
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("OpenAI Chat API request failed: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let error_text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("OpenAI Chat API error ({}): {}", status, error_text));
            }

            // Parse SSE stream for Chat Completions format
            let (text, tool_calls, round_usage) = self.parse_chat_completions_sse(resp, cancel, on_delta).await?;
            collected_text.push_str(&text);
            total_usage.input_tokens += round_usage.input_tokens;
            total_usage.output_tokens += round_usage.output_tokens;
            total_usage.cache_creation_input_tokens += round_usage.cache_creation_input_tokens;
            total_usage.cache_read_input_tokens += round_usage.cache_read_input_tokens;

            if tool_calls.is_empty() {
                break;
            }

            // Build assistant message with tool_calls
            let tc_json: Vec<serde_json::Value> = tool_calls.iter().map(|tc| {
                json!({
                    "id": tc.call_id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }
                })
            }).collect();

            let mut assistant_msg = json!({ "role": "assistant" });
            if !text.is_empty() {
                assistant_msg["content"] = json!(text);
            }
            assistant_msg["tool_calls"] = json!(tc_json);
            messages.push(assistant_msg);

            // Execute tools
            for tc in &tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);

                let result = match tools::execute_tool(&tc.name, &args).await {
                    Ok(r) => r,
                    Err(e) => format!("Tool error: {}", e),
                };

                emit_tool_result(on_delta, &tc.call_id, &result);

                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc.call_id,
                    "content": result,
                }));
            }
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from OpenAI Chat API"));
        }

        if !collected_text.is_empty() {
            messages.push(json!({ "role": "assistant", "content": collected_text }));
        }
        *self.conversation_history.lock().unwrap() = messages;

        // Emit accumulated usage
        emit_usage(on_delta, &total_usage);

        Ok(collected_text)
    }

    /// Parse OpenAI Chat Completions SSE stream
    async fn parse_chat_completions_sse(
        &self,
        resp: reqwest::Response,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>, ChatUsage)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
        // Track tool calls by index
        let mut pending_calls: std::collections::HashMap<usize, FunctionCallItem> = std::collections::HashMap::new();
        let mut usage = ChatUsage::default();

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(idx) = buffer.find("\n\n") {
                let event_block = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                for line in event_block.lines() {
                    let data = if let Some(d) = line.strip_prefix("data:") {
                        d.trim()
                    } else {
                        continue;
                    };

                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }

                    if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                        // Parse usage from stream (when stream_options.include_usage is set)
                        if let Some(u) = chunk.get("usage") {
                            if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                                usage.input_tokens = pt;
                            }
                            if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                                usage.output_tokens = ct;
                            }
                            // OpenAI: prompt_tokens_details.cached_tokens
                            if let Some(details) = u.get("prompt_tokens_details") {
                                if let Some(cached) = details.get("cached_tokens").and_then(|v| v.as_u64()) {
                                    usage.cache_read_input_tokens = cached;
                                }
                            }
                            // Moonshot/Kimi: cached_tokens at top level
                            if let Some(cached) = u.get("cached_tokens").and_then(|v| v.as_u64()) {
                                if usage.cache_read_input_tokens == 0 {
                                    usage.cache_read_input_tokens = cached;
                                }
                            }
                        }
                        if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
                            for choice in choices {
                                let delta = match choice.get("delta") {
                                    Some(d) => d,
                                    None => continue,
                                };

                                // Reasoning/thinking content (DeepSeek, OpenAI o-series, etc.)
                                if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                                    if !reasoning.is_empty() {
                                        emit_thinking_delta(on_delta, reasoning);
                                    }
                                }

                                // Text content
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                    emit_text_delta(on_delta, content);
                                    collected_text.push_str(content);
                                }

                                // Tool calls
                                if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                    for tc_delta in tcs {
                                        let idx = tc_delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;

                                        if let Some(func) = tc_delta.get("function") {
                                            let entry = pending_calls.entry(idx).or_insert_with(|| {
                                                FunctionCallItem {
                                                    call_id: tc_delta.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                                                    name: String::new(),
                                                    arguments: String::new(),
                                                }
                                            });

                                            if let Some(id) = tc_delta.get("id").and_then(|i| i.as_str()) {
                                                if !id.is_empty() {
                                                    entry.call_id = id.to_string();
                                                }
                                            }
                                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                                entry.name.push_str(name);
                                            }
                                            if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                                entry.arguments.push_str(args);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Move pending calls to final list
        let mut sorted_keys: Vec<usize> = pending_calls.keys().cloned().collect();
        sorted_keys.sort();
        for key in sorted_keys {
            if let Some(tc) = pending_calls.remove(&key) {
                tool_calls.push(tc);
            }
        }

        Ok((collected_text, tool_calls, usage))
    }

    // ── OpenAI Responses API (custom base_url) ────────────────────

    async fn chat_openai_responses(&self, api_key: &str, base_url: &str, model: &str, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);

        let reasoning = reasoning_effort
            .and_then(|e| clamp_reasoning_effort(model, e))
            .map(|effort| ReasoningConfig { effort });

        let mut input: Vec<serde_json::Value> = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_responses(message, attachments);
        Self::push_user_message(&mut input, user_content);

        let api_url = build_api_url(base_url, "/v1/responses");
        let mut collected_text = String::new();
        let mut total_usage = ChatUsage::default();
        let system_prompt = build_system_prompt();

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 { u32::MAX } else { max_rounds };
        for _round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) { break; }

            let request = ResponsesRequest {
                model: model.to_string(),
                store: false,
                stream: true,
                instructions: system_prompt.clone(),
                input: input.clone(),
                reasoning: reasoning.as_ref().map(|r| ReasoningConfig { effort: r.effort.clone() }),
                tools: Some(tool_schemas.clone()),
            };

            let mut req = client
                .post(&api_url)
                .header("Content-Type", "application/json");
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }
            let resp = req
                .json(&request)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("OpenAI Responses API request failed: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let error_text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("OpenAI Responses API error ({}): {}", status, error_text));
            }

            let (text, tool_calls, round_usage) = self.parse_openai_sse(resp, cancel, on_delta).await?;
            collected_text.push_str(&text);
            total_usage.input_tokens += round_usage.input_tokens;
            total_usage.output_tokens += round_usage.output_tokens;
            total_usage.cache_creation_input_tokens += round_usage.cache_creation_input_tokens;
            total_usage.cache_read_input_tokens += round_usage.cache_read_input_tokens;

            if tool_calls.is_empty() {
                break;
            }

            for tc in &tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);

                let result = match tools::execute_tool(&tc.name, &args).await {
                    Ok(r) => r,
                    Err(e) => format!("Tool error: {}", e),
                };

                emit_tool_result(on_delta, &tc.call_id, &result);

                input.push(json!({
                    "type": "function_call",
                    "id": tc.call_id,
                    "call_id": tc.call_id,
                    "name": tc.name,
                    "arguments": tc.arguments,
                }));
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tc.call_id,
                    "output": result,
                }));
            }
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from OpenAI Responses API"));
        }

        if !collected_text.is_empty() {
            input.push(json!({ "role": "assistant", "content": collected_text }));
        }
        *self.conversation_history.lock().unwrap() = input;

        // Emit accumulated usage
        emit_usage(on_delta, &total_usage);

        Ok(collected_text)
    }

    // ── OpenAI Codex Responses API with Tool Loop ─────────────────

    async fn chat_openai(&self, access_token: &str, account_id: &str, model: &str, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::new();
        let tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);

        // Build reasoning config with clamping
        let reasoning = reasoning_effort
            .and_then(|e| clamp_reasoning_effort(model, e))
            .map(|effort| ReasoningConfig { effort });

        // Build input from conversation history + new user message (with optional image attachments)
        let mut input: Vec<serde_json::Value> = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_responses(message, attachments);
        Self::push_user_message(&mut input, user_content);

        let user_agent = format!(
            "OpenComputer ({} {}; {})",
            std::env::consts::OS,
            os_version(),
            std::env::consts::ARCH,
        );

        let mut collected_text = String::new();
        let mut total_usage = ChatUsage::default();
        let system_prompt = build_system_prompt();

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 { u32::MAX } else { max_rounds };
        for _round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) { break; }

            let request = ResponsesRequest {
                model: model.to_string(),
                store: false,
                stream: true,
                instructions: system_prompt.clone(),
                input: input.clone(),
                reasoning: reasoning.as_ref().map(|r| ReasoningConfig { effort: r.effort.clone() }),
                tools: Some(tool_schemas.clone()),
            };

            let body_json = serde_json::to_string(&request)?;

            // Retry loop with exponential backoff
            let mut last_error: Option<String> = None;
            let mut resp_opt: Option<reqwest::Response> = None;

            for attempt in 0..=MAX_RETRIES {
                let response = client
                    .post(CODEX_API_URL)
                    .header("Authorization", format!("Bearer {}", access_token))
                    .header("Content-Type", "application/json")
                    .header("chatgpt-account-id", account_id)
                    .header("OpenAI-Beta", "responses=experimental")
                    .header("originator", "opencomputer")
                    .header("User-Agent", &user_agent)
                    .header("accept", "text/event-stream")
                    .body(body_json.clone())
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            resp_opt = Some(resp);
                            break;
                        }

                        let status = resp.status().as_u16();
                        let error_text = resp.text().await.unwrap_or_default();

                        if attempt < MAX_RETRIES && is_retryable_error(status, &error_text) {
                            let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                            log::warn!("Codex API error {} (attempt {}/{}), retrying in {}ms", status, attempt + 1, MAX_RETRIES, delay);
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            last_error = Some(error_text);
                            continue;
                        }

                        let friendly = parse_error_response(status, &error_text);
                        return Err(anyhow::anyhow!("{}", friendly));
                    }
                    Err(e) => {
                        if attempt < MAX_RETRIES {
                            let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                            log::warn!("Codex API network error (attempt {}/{}): {}, retrying in {}ms", attempt + 1, MAX_RETRIES, e, delay);
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            last_error = Some(e.to_string());
                            continue;
                        }
                        return Err(anyhow::anyhow!("Codex API request failed: {}", e));
                    }
                }
            }

            let resp = resp_opt.ok_or_else(|| {
                anyhow::anyhow!("Codex API failed after {} retries: {}", MAX_RETRIES, last_error.unwrap_or_default())
            })?;

            // Parse SSE stream
            let (text, tool_calls, round_usage) = self.parse_openai_sse(resp, cancel, on_delta).await?;
            collected_text.push_str(&text);
            total_usage.input_tokens += round_usage.input_tokens;
            total_usage.output_tokens += round_usage.output_tokens;
            total_usage.cache_creation_input_tokens += round_usage.cache_creation_input_tokens;
            total_usage.cache_read_input_tokens += round_usage.cache_read_input_tokens;

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                break;
            }

            // Execute tools and append results to input
            for tc in &tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);

                let result = match tools::execute_tool(&tc.name, &args).await {
                    Ok(r) => r,
                    Err(e) => format!("Tool error: {}", e),
                };

                emit_tool_result(on_delta, &tc.call_id, &result);

                // Append function_call item to input
                input.push(json!({
                    "type": "function_call",
                    "id": tc.call_id,
                    "call_id": tc.call_id,
                    "name": tc.name,
                    "arguments": tc.arguments,
                }));

                // Append function_call_output to input
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tc.call_id,
                    "output": result,
                }));
            }
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from Codex API"));
        }

        // Persist conversation history
        if !collected_text.is_empty() {
            input.push(json!({ "role": "assistant", "content": collected_text }));
        }
        *self.conversation_history.lock().unwrap() = input;

        // Emit accumulated usage
        emit_usage(on_delta, &total_usage);

        Ok(collected_text)
    }

    /// Parse OpenAI SSE stream. Returns (collected_text, tool_calls, usage)
    async fn parse_openai_sse(
        &self,
        resp: reqwest::Response,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>, ChatUsage)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
        let mut pending_calls: std::collections::HashMap<String, FunctionCallItem> = std::collections::HashMap::new();
        let mut usage = ChatUsage::default();

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(idx) = buffer.find("\n\n") {
                let event_block = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                let data_lines: Vec<&str> = event_block
                    .lines()
                    .filter(|l| l.starts_with("data:"))
                    .map(|l| l[5..].trim())
                    .collect();

                if data_lines.is_empty() {
                    continue;
                }

                let data = data_lines.join("\n").trim().to_string();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }

                if let Ok(event) = serde_json::from_str::<SseEvent>(&data) {
                    let event_type = event.event_type.as_deref().unwrap_or("");

                    match event_type {
                        // Reasoning summary deltas
                        "response.reasoning_summary_text.delta" => {
                            if let Some(delta) = &event.delta {
                                emit_thinking_delta(on_delta, delta);
                            }
                        }

                        // Text deltas
                        "response.output_text.delta" => {
                            if let Some(delta) = &event.delta {
                                emit_text_delta(on_delta, delta);
                                collected_text.push_str(delta);
                            }
                        }

                        // Function call started
                        "response.output_item.added" => {
                            if let Some(item) = &event.item {
                                if item.item_type.as_deref() == Some("function_call") {
                                    let call_id = item.id.clone()
                                        .or_else(|| item.call_id.clone())
                                        .unwrap_or_default();
                                    let name = item.name.clone().unwrap_or_default();
                                    pending_calls.insert(call_id.clone(), FunctionCallItem {
                                        call_id,
                                        name,
                                        arguments: item.arguments.clone().unwrap_or_default(),
                                    });
                                }
                            }
                        }

                        // Function call arguments delta
                        "response.function_call_arguments.delta" => {
                            if let Some(delta) = &event.delta {
                                // Find the pending call to append args to
                                // The event doesn't always include item_id, try all pending
                                if let Some(item) = &event.item {
                                    let call_id = item.id.clone()
                                        .or_else(|| item.call_id.clone())
                                        .unwrap_or_default();
                                    if let Some(tc) = pending_calls.get_mut(&call_id) {
                                        tc.arguments.push_str(delta);
                                    }
                                } else {
                                    // Fallback: append to last pending call
                                    if let Some(tc) = pending_calls.values_mut().last() {
                                        tc.arguments.push_str(delta);
                                    }
                                }
                            }
                        }

                        // Function call done or output item done
                        "response.function_call_arguments.done" | "response.output_item.done" => {
                            if let Some(item) = &event.item {
                                if item.item_type.as_deref() == Some("function_call") {
                                    let call_id = item.id.clone()
                                        .or_else(|| item.call_id.clone())
                                        .unwrap_or_default();
                                    if let Some(mut tc) = pending_calls.remove(&call_id) {
                                        // Use final arguments from the event if available
                                        if let Some(args) = &item.arguments {
                                            if !args.is_empty() {
                                                tc.arguments = args.clone();
                                            }
                                        }
                                        if item.name.is_some() {
                                            tc.name = item.name.clone().unwrap_or_default();
                                        }
                                        tool_calls.push(tc);
                                    }
                                }
                            }
                        }

                        // Handle errors
                        "error" => {
                            let msg = event.message.as_deref()
                                .or(event.code.as_deref())
                                .unwrap_or("Unknown error");
                            return Err(anyhow::anyhow!("Codex error: {}", msg));
                        }
                        "response.failed" => {
                            let msg = event.response
                                .as_ref()
                                .and_then(|r| r.error.as_ref())
                                .and_then(|e| e.message.as_deref())
                                .unwrap_or("Codex response failed");
                            return Err(anyhow::anyhow!("{}", msg));
                        }

                        // Response completed — extract from full response if no deltas collected
                        "response.completed" | "response.done" => {
                            // Extract usage from response
                            if let Some(resp_obj) = &event.response {
                                if let Some(u) = &resp_obj.usage {
                                    if let Some(it) = u.input_tokens {
                                        usage.input_tokens = it;
                                    }
                                    if let Some(ot) = u.output_tokens {
                                        usage.output_tokens = ot;
                                    }
                                    // Responses API cache tokens
                                    if let Some(cr) = u.cache_read_input_tokens {
                                        usage.cache_read_input_tokens = cr;
                                    }
                                    if let Some(cc) = u.cache_creation_input_tokens {
                                        usage.cache_creation_input_tokens = cc;
                                    }
                                }
                            }
                            if collected_text.is_empty() && tool_calls.is_empty() {
                                if let Some(resp_obj) = &event.response {
                                    if let Some(outputs) = &resp_obj.output {
                                        for item in outputs {
                                            if item.item_type.as_deref() == Some("message") {
                                                if let Some(parts) = &item.content {
                                                    for part in parts {
                                                        if part.part_type.as_deref() == Some("output_text") {
                                                            if let Some(text) = &part.text {
                                                                collected_text.push_str(text);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            // Also pick up function_call items from completed response
                                            if item.item_type.as_deref() == Some("function_call") {
                                                let call_id = item.id.clone()
                                                    .or_else(|| item.call_id.clone())
                                                    .unwrap_or_default();
                                                tool_calls.push(FunctionCallItem {
                                                    call_id,
                                                    name: item.name.clone().unwrap_or_default(),
                                                    arguments: item.arguments.clone().unwrap_or_default(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        _ => {} // Ignore other event types
                    }
                }
            }
        }

        // Drain any remaining pending calls
        for (_, tc) in pending_calls {
            tool_calls.push(tc);
        }

        Ok((collected_text, tool_calls, usage))
    }
}

// ── Helper functions ──────────────────────────────────────────────

/// Check if an HTTP error is retryable (rate limit or server error)
fn is_retryable_error(status: u16, error_text: &str) -> bool {
    if matches!(status, 429 | 500 | 502 | 503 | 504) {
        return true;
    }
    let lower = error_text.to_lowercase();
    lower.contains("rate") && lower.contains("limit")
        || lower.contains("overloaded")
        || lower.contains("service unavailable")
        || lower.contains("upstream connect")
        || lower.contains("connection refused")
}

/// Parse error response and return a user-friendly message
fn parse_error_response(status: u16, raw: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<ApiErrorResponse>(raw) {
        if let Some(detail) = &parsed.detail {
            if let Some(s) = detail.as_str() {
                return format!("Codex API 错误 ({}): {}", status, s);
            }
        }

        if let Some(err) = parsed.error {
            let code = err.code.as_deref()
                .or(err.error_type.as_deref())
                .unwrap_or("");

            if code.contains("usage_limit_reached")
                || code.contains("usage_not_included")
                || code.contains("rate_limit_exceeded")
                || status == 429
            {
                let plan = err.plan_type
                    .as_ref()
                    .map(|p| format!(" ({} plan)", p.to_lowercase()))
                    .unwrap_or_default();

                let when = if let Some(resets_at) = err.resets_at {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as f64;
                    let mins = ((resets_at - now_secs) / 60.0).max(0.0).round() as u64;
                    format!(" 大约 {} 分钟后可重试。", mins)
                } else {
                    String::new()
                };

                return format!("您已达到 ChatGPT 使用限额{}。{}", plan, when);
            }

            if let Some(msg) = err.message {
                return format!("Codex API 错误 ({}): {}", status, msg);
            }
        }
    }

    format!("Codex API 错误 ({}): {}", status, raw)
}

/// Get OS version string
fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        "unknown".to_string()
    }
}
