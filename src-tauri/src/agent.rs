use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tools::{self, ToolProvider};

const SYSTEM_PROMPT: &str = "You are OpenComputer, a personal AI assistant with deep system integration. \
                             You help users interact with their computer naturally and efficiently. \
                             You have access to tools that let you execute shell commands, read/write files, \
                             and list directories on the user's computer. Use these tools when the user asks \
                             you to interact with their system.";

const CODEX_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1000;
const MAX_TOOL_ROUNDS: u32 = 10;

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

/// Supported LLM providers
pub enum LlmProvider {
    Anthropic { api_key: String },
    OpenAI { access_token: String, account_id: String, model: String },
}

pub struct AssistantAgent {
    provider: LlmProvider,
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
    /// Create agent with Anthropic API key
    pub fn new_anthropic(api_key: &str) -> Self {
        Self {
            provider: LlmProvider::Anthropic {
                api_key: api_key.to_string(),
            },
        }
    }

    /// Create agent with OpenAI-compatible access token (Codex OAuth)
    pub fn new_openai(access_token: &str, account_id: &str, model: &str) -> Self {
        Self {
            provider: LlmProvider::OpenAI {
                access_token: access_token.to_string(),
                account_id: account_id.to_string(),
                model: model.to_string(),
            },
        }
    }

    pub async fn chat(&self, message: &str, reasoning_effort: Option<&str>, on_delta: impl Fn(&str) + Send + 'static) -> Result<String> {
        match &self.provider {
            LlmProvider::Anthropic { api_key } => {
                self.chat_anthropic(api_key, message, &on_delta).await
            }
            LlmProvider::OpenAI { access_token, account_id, model } => {
                self.chat_openai(access_token, account_id, model, message, reasoning_effort, &on_delta).await
            }
        }
    }

    // ── Anthropic Messages API with Tool Loop ─────────────────────

    async fn chat_anthropic(&self, api_key: &str, message: &str, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::new();
        let tool_schemas = tools::get_tools_for_provider(ToolProvider::Anthropic);

        // Build initial messages
        let mut messages = vec![
            json!({ "role": "user", "content": message }),
        ];

        let mut collected_text = String::new();

        for _round in 0..MAX_TOOL_ROUNDS {
            let body = json!({
                "model": ANTHROPIC_MODEL,
                "max_tokens": 8192,
                "system": SYSTEM_PROMPT,
                "tools": tool_schemas,
                "messages": messages,
                "stream": true,
            });

            let resp = client
                .post(ANTHROPIC_API_URL)
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
            let (text, tool_calls, stop_reason) = self.parse_anthropic_sse(resp, on_delta).await?;
            collected_text.push_str(&text);

            // If no tool calls, we're done
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

        if collected_text.is_empty() {
            return Err(anyhow::anyhow!("No content received from Anthropic API"));
        }

        Ok(collected_text)
    }

    /// Parse Anthropic SSE stream. Returns (collected_text, tool_calls, stop_reason)
    async fn parse_anthropic_sse(
        &self,
        resp: reqwest::Response,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>, Option<String>)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
        // Track current content blocks by index
        let mut current_tool: Option<(usize, FunctionCallItem)> = None;
        let mut stop_reason: Option<String> = None;

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
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
                                if block.block_type.as_deref() == Some("tool_use") {
                                    let idx = event.index.unwrap_or(0);
                                    current_tool = Some((idx, FunctionCallItem {
                                        call_id: block.id.clone().unwrap_or_default(),
                                        name: block.name.clone().unwrap_or_default(),
                                        arguments: String::new(),
                                    }));
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = &event.delta {
                                match delta.delta_type.as_deref() {
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
                            if let Some((_, tc)) = current_tool.take() {
                                tool_calls.push(tc);
                            }
                        }
                        "message_delta" => {
                            if let Some(delta) = &event.delta {
                                if let Some(reason) = &delta.stop_reason {
                                    stop_reason = Some(reason.clone());
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

        Ok((collected_text, tool_calls, stop_reason))
    }

    // ── OpenAI Codex Responses API with Tool Loop ─────────────────

    async fn chat_openai(&self, access_token: &str, account_id: &str, model: &str, message: &str, reasoning_effort: Option<&str>, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::new();
        let tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);

        // Build reasoning config with clamping
        let reasoning = reasoning_effort
            .and_then(|e| clamp_reasoning_effort(model, e))
            .map(|effort| ReasoningConfig { effort });

        // Build initial input
        let mut input: Vec<serde_json::Value> = vec![
            json!({ "role": "user", "content": message }),
        ];

        let user_agent = format!(
            "OpenComputer ({} {}; {})",
            std::env::consts::OS,
            os_version(),
            std::env::consts::ARCH,
        );

        let mut collected_text = String::new();

        for _round in 0..MAX_TOOL_ROUNDS {
            let request = ResponsesRequest {
                model: model.to_string(),
                store: false,
                stream: true,
                instructions: SYSTEM_PROMPT.to_string(),
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
            let (text, tool_calls) = self.parse_openai_sse(resp, on_delta).await?;
            collected_text.push_str(&text);

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

        if collected_text.is_empty() {
            return Err(anyhow::anyhow!("No content received from Codex API"));
        }

        Ok(collected_text)
    }

    /// Parse OpenAI SSE stream. Returns (collected_text, tool_calls)
    async fn parse_openai_sse(
        &self,
        resp: reqwest::Response,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
        let mut pending_calls: std::collections::HashMap<String, FunctionCallItem> = std::collections::HashMap::new();

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
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
                                    let call_id = item.call_id.clone()
                                        .or_else(|| item.id.clone())
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
                                    let call_id = item.call_id.clone()
                                        .or_else(|| item.id.clone())
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
                                    let call_id = item.call_id.clone()
                                        .or_else(|| item.id.clone())
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
                                                let call_id = item.call_id.clone()
                                                    .or_else(|| item.id.clone())
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

        Ok((collected_text, tool_calls))
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
