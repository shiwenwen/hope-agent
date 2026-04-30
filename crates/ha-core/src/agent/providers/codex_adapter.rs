//! Codex (ChatGPT subscription) adapter implementing [`StreamingChatAdapter`].
//!
//! Same wire protocol as OpenAI Responses (uses [`ResponsesRequest`] body and
//! [`super::openai_responses_adapter::parse_openai_sse`] for streaming) — the
//! difference is the endpoint ([`CODEX_API_URL`]), the auth scheme (OAuth
//! `access_token` + `chatgpt-account-id` header + special user agent), and an
//! internal retry-with-backoff loop for transient 5xx / network errors.
//!
//! The retry loop's `is_retryable_error` predicate is intentionally limited to
//! transient failures (network errors, gateway 5xx). Semantic errors
//! (RateLimit / Auth / Billing / Timeout) bubble up so the upcoming Phase 3
//! `execute_with_failover` executor can rotate auth profiles uniformly across
//! all providers (no double-retry).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::ResponsesRequest;
use super::super::config::{BASE_DELAY_MS, CODEX_API_URL, MAX_RETRIES};
use super::super::errors::{is_retryable_error, os_version, parse_error_response};
use super::super::events::expand_responses_image_markers_for_api;
use super::super::streaming_adapter::{
    ExecutedTool, RoundOutcome, RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ProviderFormat};
use super::openai_responses_adapter::parse_openai_sse;
use crate::tools::ToolProvider;

/// Process-stable User-Agent for Codex requests.
pub(in crate::agent) fn codex_user_agent() -> &'static str {
    static UA: OnceLock<String> = OnceLock::new();
    UA.get_or_init(|| {
        format!(
            "Hope Agent ({} {}; {})",
            std::env::consts::OS,
            os_version(),
            std::env::consts::ARCH,
        )
    })
}

/// Apply Codex's OAuth + SSE headers to a [`reqwest::RequestBuilder`].
/// Shared by streaming chat_round and one-shot side_query.
pub(in crate::agent) fn apply_codex_headers(
    builder: reqwest::RequestBuilder,
    access_token: &str,
    account_id: &str,
    user_agent: &str,
) -> reqwest::RequestBuilder {
    builder
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("chatgpt-account-id", account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .header("originator", "hope-agent")
        .header("User-Agent", user_agent)
        .header("accept", "text/event-stream")
}

pub(crate) struct CodexStreamingAdapter<'a> {
    pub access_token: &'a str,
    pub account_id: &'a str,
    pub model: &'a str,
    pub reasoning: Option<super::super::api_types::ReasoningConfig>,
}

#[async_trait]
impl<'a> StreamingChatAdapter for CodexStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::Codex
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::OpenAI
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_responses(history);
    }

    async fn chat_round(
        &self,
        client: &reqwest::Client,
        req: RoundRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<RoundOutcome> {
        // Inject awareness + active memory as leading system items (same as
        // openai_responses_adapter — keeps `instructions` cache-friendly).
        let mut api_input: Vec<Value> = expand_responses_image_markers_for_api(req.history_for_api);
        if let Some(active_suffix) = req.active_memory_suffix {
            if !active_suffix.is_empty() {
                api_input.insert(0, json!({ "role": "system", "content": active_suffix }));
            }
        }
        if let Some(suffix) = req.awareness_suffix {
            if !suffix.is_empty() {
                api_input.insert(0, json!({ "role": "system", "content": suffix }));
            }
        }

        let request = ResponsesRequest {
            model: self.model.to_string(),
            store: false,
            stream: true,
            instructions: req.system_prompt.to_string(),
            input: api_input.clone(),
            reasoning: self.reasoning.clone(),
            include: if self.reasoning.is_some() {
                Some(vec!["reasoning.encrypted_content".to_string()])
            } else {
                None
            },
            tools: if req.is_final_round {
                None
            } else {
                Some(req.tool_schemas.to_vec())
            },
            temperature: req.temperature,
        };

        let body_json = serde_json::to_string(&request)?;

        if let Some(logger) = crate::get_logger() {
            let body_size = body_json.len();
            let raw_body = if body_size > 32768 {
                format!(
                    "{}...(truncated, total {}B)",
                    crate::truncate_utf8(&body_json, 32768),
                    body_size
                )
            } else {
                body_json.clone()
            };
            let raw_body = crate::logging::redact_sensitive(&raw_body);
            logger.log(
                "debug",
                "agent",
                "agent::chat_codex::request",
                &format!(
                    "Codex API request round {}: {} input items, {} tools, body {}B",
                    req.round,
                    api_input.len(),
                    req.tool_schemas.len(),
                    body_size
                ),
                Some(
                    json!({
                        "round": req.round,
                        "api_url": CODEX_API_URL,
                        "model": self.model,
                        "input_count": api_input.len(),
                        "tool_count": req.tool_schemas.len(),
                        "body_size_bytes": body_size,
                        "reasoning": self.reasoning.as_ref().map(|r| r.effort.as_str()),
                        "request_body": raw_body,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        // ── Send with internal retry loop (transient 5xx + network errors only).
        let mut last_error: Option<String> = None;
        let mut resp_opt: Option<reqwest::Response> = None;
        let request_start = std::time::Instant::now();

        for attempt in 0..=MAX_RETRIES {
            let builder = apply_codex_headers(
                client.post(CODEX_API_URL),
                self.access_token,
                self.account_id,
                codex_user_agent(),
            );
            let response = builder.body(body_json.clone()).send().await;

            match response {
                Ok(resp) => {
                    if resp.status().is_success() {
                        if let Some(logger) = crate::get_logger() {
                            let ttfb_ms = request_start.elapsed().as_millis() as u64;
                            let headers = resp.headers();
                            let request_id = headers
                                .get("x-request-id")
                                .or_else(|| headers.get("request-id"))
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("-")
                                .to_string();
                            let response_headers = json!({
                                "x-request-id": request_id,
                                "x-ratelimit-limit-requests": headers.get("x-ratelimit-limit-requests").and_then(|v| v.to_str().ok()),
                                "x-ratelimit-limit-tokens": headers.get("x-ratelimit-limit-tokens").and_then(|v| v.to_str().ok()),
                                "x-ratelimit-remaining-requests": headers.get("x-ratelimit-remaining-requests").and_then(|v| v.to_str().ok()),
                                "x-ratelimit-remaining-tokens": headers.get("x-ratelimit-remaining-tokens").and_then(|v| v.to_str().ok()),
                                "openai-model": headers.get("openai-model").and_then(|v| v.to_str().ok()),
                                "retry-after": headers.get("retry-after").and_then(|v| v.to_str().ok()),
                            });
                            logger.log("debug", "agent", "agent::chat_codex::response",
                                &format!("Codex API response: status=200, request_id={}, ttfb={}ms, attempt={}", request_id, ttfb_ms, attempt + 1),
                                Some(json!({
                                    "status": 200,
                                    "ttfb_ms": ttfb_ms,
                                    "attempt": attempt + 1,
                                    "round": req.round,
                                    "response_headers": response_headers,
                                }).to_string()),
                                None, None);
                        }
                        resp_opt = Some(resp);
                        break;
                    }

                    let status = resp.status().as_u16();
                    let error_text = resp.text().await.unwrap_or_default();

                    if attempt < MAX_RETRIES && is_retryable_error(status, &error_text) {
                        let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                        app_warn!(
                            "agent",
                            "codex",
                            "Codex API error {} (attempt {}/{}), retrying in {}ms",
                            status,
                            attempt + 1,
                            MAX_RETRIES,
                            delay
                        );
                        if let Some(logger) = crate::get_logger() {
                            logger.log("warn", "agent", "agent::chat_codex::retry",
                                &format!("Codex API error {}, retrying (attempt {}/{})", status, attempt + 1, MAX_RETRIES),
                                Some(json!({"status": status, "attempt": attempt + 1, "delay_ms": delay, "error": &error_text}).to_string()),
                                None, None);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        last_error = Some(error_text);
                        continue;
                    }

                    if let Some(logger) = crate::get_logger() {
                        let error_preview = if error_text.len() > 500 {
                            format!("{}...", crate::truncate_utf8(&error_text, 500))
                        } else {
                            error_text.clone()
                        };
                        logger.log(
                            "error",
                            "agent",
                            "agent::chat_codex::error",
                            &format!("Codex API error ({}): {}", status, error_preview),
                            Some(
                                json!({"status": status, "error": error_text, "round": req.round})
                                    .to_string(),
                            ),
                            None,
                            None,
                        );
                    }
                    let friendly = parse_error_response(status, &error_text);
                    return Err(anyhow::anyhow!("{}", friendly));
                }
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                        app_warn!(
                            "agent",
                            "codex",
                            "Codex API network error (attempt {}/{}): {}, retrying in {}ms",
                            attempt + 1,
                            MAX_RETRIES,
                            e,
                            delay
                        );
                        if let Some(logger) = crate::get_logger() {
                            logger.log("warn", "agent", "agent::chat_codex::retry",
                                &format!("Codex API network error, retrying (attempt {}/{}): {}", attempt + 1, MAX_RETRIES, e),
                                Some(json!({"attempt": attempt + 1, "delay_ms": delay, "error": e.to_string()}).to_string()),
                                None, None);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        last_error = Some(e.to_string());
                        continue;
                    }
                    return Err(anyhow::anyhow!("Codex API request failed: {}", e));
                }
            }
        }

        let resp = resp_opt.ok_or_else(|| {
            anyhow::anyhow!(
                "Codex API failed after {} retries: {}",
                MAX_RETRIES,
                last_error.unwrap_or_default()
            )
        })?;

        // Cancel check before SSE parse begins.
        if cancel.load(Ordering::SeqCst) {
            return Ok(RoundOutcome {
                text: String::new(),
                thinking: String::new(),
                tool_calls: Vec::new(),
                usage: Default::default(),
                ttft_ms: None,
                stop_reason: None,
                reasoning_items: Vec::new(),
            });
        }

        let (text, tool_calls, usage, thinking_text, ttft_ms, reasoning_items) =
            parse_openai_sse(resp, request_start, cancel.as_ref(), on_delta).await?;

        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_codex::tool_loop",
                    &format!(
                        "Tool loop round {}: executing {} tools: {:?}",
                        req.round,
                        tool_calls.len(),
                        tool_names
                    ),
                    Some(
                        json!({
                            "round": req.round,
                            "tool_count": tool_calls.len(),
                            "tools": tool_names,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }
        }

        Ok(RoundOutcome {
            text,
            thinking: thinking_text,
            tool_calls,
            usage,
            ttft_ms,
            stop_reason: None,
            reasoning_items,
        })
    }

    fn append_reasoning_items(&self, history: &mut Vec<Value>, outcome: &RoundOutcome) {
        for ri in &outcome.reasoning_items {
            history.push(ri.clone());
        }
    }

    fn append_round_to_history(
        &self,
        history: &mut Vec<Value>,
        round: u32,
        _outcome: &RoundOutcome,
        executed: &[ExecutedTool],
    ) {
        for et in executed {
            crate::context_compact::push_and_stamp(
                history,
                json!({
                    "type": "function_call",
                    "id": et.call_id,
                    "call_id": et.call_id,
                    "name": et.name,
                    "arguments": et.arguments,
                }),
                round,
            );
            crate::context_compact::push_and_stamp(
                history,
                json!({
                    "type": "function_call_output",
                    "call_id": et.call_id,
                    "output": et.clean_result,
                }),
                round,
            );
        }
    }

    fn append_final_assistant(
        &self,
        history: &mut Vec<Value>,
        final_text: &str,
        _last_thinking: &str,
    ) {
        if !final_text.is_empty() {
            history.push(json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": final_text }],
                "status": "completed"
            }));
        }
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        outcome.tool_calls.is_empty()
    }
}
