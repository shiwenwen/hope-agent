use anyhow::Result;
use rig::providers::anthropic;
use rig::completion::Prompt;
use rig::client::CompletionClient;
use serde::{Deserialize, Serialize};

const SYSTEM_PROMPT: &str = "You are OpenComputer, a personal AI assistant with deep system integration. \
                             You help users interact with their computer naturally and efficiently.";

const CODEX_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1000;

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
    // gpt-5.1-codex-mini: only medium/high
    if model.contains("5.1-codex-mini") {
        return match effort {
            "low" => Some("medium".to_string()),
            "xhigh" => Some("high".to_string()),
            _ => Some(effort.to_string()),
        };
    }
    // gpt-5.1*: xhigh → high
    if model.contains("5.1") {
        return match effort {
            "xhigh" => Some("high".to_string()),
            _ => Some(effort.to_string()),
        };
    }
    // gpt-5.2/5.3/5.4: all efforts supported
    Some(effort.to_string())
}

/// Supported LLM providers
pub enum LlmProvider {
    Anthropic(anthropic::Client),
    OpenAI { access_token: String, account_id: String, model: String },
}

pub struct AssistantAgent {
    provider: LlmProvider,
}

// ── OpenAI Responses API types ────────────────────────────────────

#[derive(Serialize)]
struct InputMessage {
    role: String,
    content: String,
}

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
    input: Vec<InputMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
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
    // For error events
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize, Debug)]
struct SseResponseObj {
    #[serde(default)]
    output: Option<Vec<OutputItem>>,
    #[serde(default)]
    error: Option<SseResponseError>,
}

#[derive(Deserialize, Debug)]
struct SseResponseError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OutputItem {
    #[serde(rename = "type", default)]
    item_type: Option<String>,
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
    // Sometimes the error comes as "detail" string directly
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

// ── AssistantAgent ────────────────────────────────────────────────

impl AssistantAgent {
    /// Create agent with Anthropic API key
    pub fn new_anthropic(api_key: &str) -> Self {
        let client = anthropic::Client::new(api_key)
            .expect("Failed to create Anthropic client");
        Self {
            provider: LlmProvider::Anthropic(client),
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
            LlmProvider::Anthropic(client) => {
                let agent = client
                    .agent("claude-sonnet-4-6")
                    .preamble(SYSTEM_PROMPT)
                    .build();
                let response = agent.prompt(message).await?;
                on_delta(&response);
                Ok(response)
            }
            LlmProvider::OpenAI { access_token, account_id, model } => {
                self.chat_openai(access_token, account_id, model, message, reasoning_effort, on_delta).await
            }
        }
    }

    /// Call Codex Responses API via chatgpt.com backend with SSE streaming and retry logic
    async fn chat_openai(&self, access_token: &str, account_id: &str, model: &str, message: &str, reasoning_effort: Option<&str>, on_delta: impl Fn(&str) + Send + 'static) -> Result<String> {
        let client = reqwest::Client::new();

        // Build reasoning config with clamping
        let reasoning = reasoning_effort
            .and_then(|e| clamp_reasoning_effort(model, e))
            .map(|effort| ReasoningConfig { effort });

        let request = ResponsesRequest {
            model: model.to_string(),
            store: false,
            stream: true,
            instructions: SYSTEM_PROMPT.to_string(),
            input: vec![
                InputMessage {
                    role: "user".to_string(),
                    content: message.to_string(),
                },
            ],
            reasoning,
        };

        let body_json = serde_json::to_string(&request)?;

        // Build User-Agent header
        let user_agent = format!(
            "OpenComputer ({} {}; {})",
            std::env::consts::OS,
            os_version(),
            std::env::consts::ARCH,
        );

        // Retry loop with exponential backoff
        let mut last_error: Option<String> = None;

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
                        // Parse SSE stream and emit text deltas
                        return self.parse_sse_stream(resp, &on_delta).await;
                    }

                    let status = resp.status().as_u16();
                    let error_text = resp.text().await.unwrap_or_default();

                    // Check if retryable
                    if attempt < MAX_RETRIES && is_retryable_error(status, &error_text) {
                        let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                        log::warn!("Codex API error {} (attempt {}/{}), retrying in {}ms", status, attempt + 1, MAX_RETRIES, delay);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        last_error = Some(error_text);
                        continue;
                    }

                    // Parse friendly error message
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

        Err(anyhow::anyhow!("Codex API failed after {} retries: {}", MAX_RETRIES, last_error.unwrap_or_default()))
    }

    /// Parse SSE stream response, emit deltas via callback, and return full text
    async fn parse_sse_stream(&self, resp: reqwest::Response, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE events (separated by double newline)
            while let Some(idx) = buffer.find("\n\n") {
                let event_block = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                // Extract data lines from the SSE event
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

                // Parse the JSON event
                if let Ok(event) = serde_json::from_str::<SseEvent>(&data) {
                    let event_type = event.event_type.as_deref().unwrap_or("");

                    match event_type {
                        // Emit text deltas in real-time
                        "response.output_text.delta" => {
                            if let Some(delta) = &event.delta {
                                on_delta(delta);
                                collected_text.push_str(delta);
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
                        // response.completed / response.done — extract from full response if no deltas collected
                        "response.completed" | "response.done" => {
                            if collected_text.is_empty() {
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

        if collected_text.is_empty() {
            return Err(anyhow::anyhow!("No content received from Codex API"));
        }

        Ok(collected_text)
    }
}

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
        // Handle "detail" field (simple string error)
        if let Some(detail) = &parsed.detail {
            if let Some(s) = detail.as_str() {
                return format!("Codex API 错误 ({}): {}", status, s);
            }
        }

        if let Some(err) = parsed.error {
            let code = err.code.as_deref()
                .or(err.error_type.as_deref())
                .unwrap_or("");

            // Check for usage limit / rate limit errors
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
