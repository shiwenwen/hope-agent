//! OpenAI Chat Completions API adapter implementing [`StreamingChatAdapter`].
//!
//! Owns body construction (multiple `system` messages for OpenAI's automatic
//! prefix caching), HTTP send, SSE event decoding (delta-based with
//! `tool_calls[]` index accumulation + `<think>` tag filtering), and history
//! persistence in Chat Completions' `tool_calls` + `role=tool` shape.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::FunctionCallItem;
use super::super::config::{apply_thinking_to_chat_body, build_api_url};
use super::super::events::{
    build_openai_chat_tool_result_content, emit_text_delta, emit_thinking_delta,
};
use super::super::streaming_adapter::{
    ExecutedTool, RoundOutcome, RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ChatUsage, ProviderFormat, ThinkTagFilter};
use crate::provider::ThinkingStyle;
use crate::tools::ToolProvider;

pub(crate) struct OpenAIChatStreamingAdapter<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
    pub thinking_style: &'a ThinkingStyle,
}

#[async_trait]
impl<'a> StreamingChatAdapter for OpenAIChatStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::OpenAIChat
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::OpenAI
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_chat(history);
    }

    async fn chat_round(
        &self,
        client: &reqwest::Client,
        req: RoundRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<RoundOutcome> {
        // Build messages array: system (static) + system (awareness) + system
        // (active memory) + history. OpenAI's automatic prefix caching still
        // hits the static prefix when later system messages change between turns.
        let mut api_messages: Vec<Value> =
            vec![json!({ "role": "system", "content": req.system_prompt })];
        if let Some(suffix) = req.awareness_suffix {
            if !suffix.is_empty() {
                api_messages.push(json!({ "role": "system", "content": suffix }));
            }
        }
        if let Some(active_suffix) = req.active_memory_suffix {
            if !active_suffix.is_empty() {
                api_messages.push(json!({ "role": "system", "content": active_suffix }));
            }
        }
        api_messages.extend_from_slice(req.history_for_api);

        // Wrap tool schemas in Chat Completions' `{type, function}` shape.
        let tools_array: Vec<Value> = req
            .tool_schemas
            .iter()
            .map(|t| json!({ "type": "function", "function": t }))
            .collect();

        // Body field order: model, messages, stream, stream_options
        // (then conditional tools / thinking via apply_thinking / temperature).
        // Must match the pre-Phase-2 chat_openai_chat byte-level for prefix cache.
        let mut body = json!({
            "model": self.model,
            "messages": api_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if !req.is_final_round {
            body["tools"] = json!(tools_array);
        }
        apply_thinking_to_chat_body(
            &mut body,
            self.thinking_style,
            req.reasoning_effort,
            req.max_tokens,
        );
        if let Some(temp) = req.temperature {
            body["temperature"] = json!(temp);
        }

        let api_url = build_api_url(self.base_url, "/v1/chat/completions");

        // ── Log API request.
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            let body_size = body_str.len();
            let raw_body = if body_size > 32768 {
                format!(
                    "{}...(truncated, total {}B)",
                    crate::truncate_utf8(&body_str, 32768),
                    body_size
                )
            } else {
                body_str.clone()
            };
            let raw_body = crate::logging::redact_sensitive(&raw_body);
            logger.log(
                "debug",
                "agent",
                "agent::chat_openai_chat::request",
                &format!(
                    "OpenAI Chat API request round {}: {} messages, {} tools, body {}B",
                    req.round,
                    api_messages.len(),
                    tools_array.len(),
                    body_size
                ),
                Some(
                    json!({
                        "round": req.round,
                        "api_url": &api_url,
                        "model": self.model,
                        "message_count": api_messages.len(),
                        "tool_count": tools_array.len(),
                        "body_size_bytes": body_size,
                        "request_body": raw_body,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        // ── Send.
        let mut http_req = client
            .post(&api_url)
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            http_req = http_req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let request_start = std::time::Instant::now();
        let resp = http_req
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("OpenAI Chat API request failed: {}", e))?;

        // ── Log response status.
        if let Some(logger) = crate::get_logger() {
            let status = resp.status().as_u16();
            let headers = resp.headers();
            let request_id = headers
                .get("x-request-id")
                .or_else(|| headers.get("request-id"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .to_string();
            let ttfb_ms = request_start.elapsed().as_millis() as u64;
            let response_headers = json!({
                "x-request-id": request_id,
                "x-ratelimit-limit-requests": headers.get("x-ratelimit-limit-requests").and_then(|v| v.to_str().ok()),
                "x-ratelimit-limit-tokens": headers.get("x-ratelimit-limit-tokens").and_then(|v| v.to_str().ok()),
                "x-ratelimit-remaining-requests": headers.get("x-ratelimit-remaining-requests").and_then(|v| v.to_str().ok()),
                "x-ratelimit-remaining-tokens": headers.get("x-ratelimit-remaining-tokens").and_then(|v| v.to_str().ok()),
                "x-ratelimit-reset-requests": headers.get("x-ratelimit-reset-requests").and_then(|v| v.to_str().ok()),
                "x-ratelimit-reset-tokens": headers.get("x-ratelimit-reset-tokens").and_then(|v| v.to_str().ok()),
                "openai-model": headers.get("openai-model").and_then(|v| v.to_str().ok()),
                "openai-organization": headers.get("openai-organization").and_then(|v| v.to_str().ok()),
                "openai-version": headers.get("openai-version").and_then(|v| v.to_str().ok()),
                "retry-after": headers.get("retry-after").and_then(|v| v.to_str().ok()),
            });
            logger.log(
                "debug",
                "agent",
                "agent::chat_openai_chat::response",
                &format!(
                    "OpenAI Chat API response: status={}, request_id={}, ttfb={}ms",
                    status, request_id, ttfb_ms
                ),
                Some(
                    json!({
                        "status": status,
                        "request_id": request_id,
                        "ttfb_ms": ttfb_ms,
                        "round": req.round,
                        "response_headers": response_headers,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let error_text = resp.text().await.unwrap_or_default();
            if let Some(logger) = crate::get_logger() {
                let error_preview = if error_text.len() > 500 {
                    format!("{}...", crate::truncate_utf8(&error_text, 500))
                } else {
                    error_text.clone()
                };
                logger.log(
                    "error",
                    "agent",
                    "agent::chat_openai_chat::error",
                    &format!("OpenAI Chat API error ({}): {}", status, error_preview),
                    Some(
                        json!({"status": status, "error": error_text, "round": req.round})
                            .to_string(),
                    ),
                    None,
                    None,
                );
            }
            return Err(anyhow::anyhow!(
                "OpenAI Chat API error ({}): {}",
                status,
                error_text
            ));
        }

        // ── Parse SSE.
        let (text, tool_calls, usage, thinking_text, ttft_ms) = parse_chat_completions_sse(
            resp,
            request_start,
            req.reasoning_effort,
            cancel,
            on_delta,
        )
        .await?;

        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_openai_chat::tool_loop",
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
            stop_reason: None, // OpenAI Chat exits via empty tool_calls
            reasoning_items: Vec::new(), // reasoning_content goes inline, no raw items
        })
    }

    fn append_round_to_history(
        &self,
        history: &mut Vec<Value>,
        round: u32,
        outcome: &RoundOutcome,
        executed: &[ExecutedTool],
    ) {
        // Build assistant message: {role, content?, reasoning_content?, tool_calls}
        let tc_json: Vec<Value> = outcome
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.call_id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }
                })
            })
            .collect();

        let mut assistant_msg = json!({ "role": "assistant" });
        if !outcome.text.is_empty() {
            assistant_msg["content"] = json!(outcome.text);
        }
        if !outcome.thinking.is_empty() {
            assistant_msg["reasoning_content"] = json!(outcome.thinking);
        }
        assistant_msg["tool_calls"] = json!(tc_json);
        crate::context_compact::push_and_stamp(history, assistant_msg, round);

        // One {role: tool, tool_call_id, content} message per executed tool.
        for et in executed {
            crate::context_compact::push_and_stamp(
                history,
                json!({
                    "role": "tool",
                    "tool_call_id": et.call_id,
                    "content": build_openai_chat_tool_result_content(&et.clean_result),
                }),
                round,
            );
        }
    }

    fn append_final_assistant(
        &self,
        history: &mut Vec<Value>,
        final_text: &str,
        last_thinking: &str,
    ) {
        if !final_text.is_empty() {
            let mut final_msg = json!({ "role": "assistant", "content": final_text });
            if !last_thinking.is_empty() {
                final_msg["reasoning_content"] = json!(last_thinking);
            }
            history.push(final_msg);
        }
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        outcome.tool_calls.is_empty()
    }
}

/// Parse OpenAI Chat Completions SSE stream.
/// Returns `(collected_text, tool_calls, usage, thinking, ttft_ms)`.
async fn parse_chat_completions_sse(
    resp: reqwest::Response,
    request_start: std::time::Instant,
    reasoning_effort: Option<&str>,
    cancel: &Arc<AtomicBool>,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    Vec<FunctionCallItem>,
    ChatUsage,
    String,
    Option<u64>,
)> {
    use futures_util::StreamExt;

    let mut collected_text = String::new();
    let mut collected_thinking = String::new();
    let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
    let mut pending_calls: std::collections::HashMap<usize, FunctionCallItem> =
        std::collections::HashMap::new();
    let mut usage = ChatUsage::default();
    let mut think_filter = ThinkTagFilter::new();
    let mut first_token_time: Option<u64> = None;

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

                if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                    // Parse usage from stream (when stream_options.include_usage is set).
                    if let Some(u) = chunk.get("usage") {
                        if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                            usage.input_tokens = pt;
                        }
                        if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                            usage.output_tokens = ct;
                        }
                        // Anthropic-style at top level (some gateways forward).
                        if let Some(cr) = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
                            usage.cache_read_input_tokens = cr;
                        }
                        if let Some(cc) = u
                            .get("cache_creation_input_tokens")
                            .and_then(|v| v.as_u64())
                        {
                            usage.cache_creation_input_tokens = cc;
                        }
                        // Fallback: OpenAI prompt_tokens_details.cached_tokens or top-level cached_tokens.
                        if usage.cache_read_input_tokens == 0 {
                            usage.cache_read_input_tokens = u
                                .get("prompt_tokens_details")
                                .and_then(|d| d.get("cached_tokens"))
                                .and_then(|v| v.as_u64())
                                .or_else(|| u.get("cached_tokens").and_then(|v| v.as_u64()))
                                .unwrap_or(0);
                        }
                    }
                    if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
                        for choice in choices {
                            let delta = match choice.get("delta") {
                                Some(d) => d,
                                None => continue,
                            };

                            // Reasoning/thinking content (DeepSeek, OpenAI o-series, etc.)
                            if let Some(reasoning) =
                                delta.get("reasoning_content").and_then(|c| c.as_str())
                            {
                                if !reasoning.is_empty() {
                                    if first_token_time.is_none() {
                                        first_token_time =
                                            Some(request_start.elapsed().as_millis() as u64);
                                    }
                                    emit_thinking_delta(&on_delta, reasoning);
                                    collected_thinking.push_str(reasoning);
                                }
                            }

                            // Text content — filter <think>...</think> tags. Qwen models embed
                            // thinking via <think> tags. With effort=none, discard entirely.
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                let (text_part, think_part) = think_filter.process(content);
                                if !think_part.is_empty() && reasoning_effort != Some("none") {
                                    emit_thinking_delta(&on_delta, &think_part);
                                    collected_thinking.push_str(&think_part);
                                }
                                if !text_part.is_empty() {
                                    if first_token_time.is_none() {
                                        first_token_time =
                                            Some(request_start.elapsed().as_millis() as u64);
                                    }
                                    emit_text_delta(&on_delta, &text_part);
                                    collected_text.push_str(&text_part);
                                }
                            }

                            // Tool calls — accumulated by index (parallel calls supported).
                            if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                for tc_delta in tcs {
                                    let idx = tc_delta
                                        .get("index")
                                        .and_then(|i| i.as_u64())
                                        .unwrap_or(0)
                                        as usize;

                                    if let Some(func) = tc_delta.get("function") {
                                        let entry =
                                            pending_calls.entry(idx).or_insert_with(|| {
                                                FunctionCallItem {
                                                    call_id: tc_delta
                                                        .get("id")
                                                        .and_then(|i| i.as_str())
                                                        .unwrap_or("")
                                                        .to_string(),
                                                    name: String::new(),
                                                    arguments: String::new(),
                                                }
                                            });
                                        if let Some(id) =
                                            tc_delta.get("id").and_then(|i| i.as_str())
                                        {
                                            if !id.is_empty() {
                                                entry.call_id = id.to_string();
                                            }
                                        }
                                        if let Some(name) =
                                            func.get("name").and_then(|n| n.as_str())
                                        {
                                            entry.name.push_str(name);
                                        }
                                        if let Some(args) =
                                            func.get("arguments").and_then(|a| a.as_str())
                                        {
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

    // Move pending calls to final list, ordered by index.
    let mut sorted_keys: Vec<usize> = pending_calls.keys().cloned().collect();
    sorted_keys.sort();
    for key in sorted_keys {
        if let Some(tc) = pending_calls.remove(&key) {
            tool_calls.push(tc);
        }
    }

    if let Some(logger) = crate::get_logger() {
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        logger.log(
            "debug",
            "agent",
            "agent::parse_chat_completions_sse::done",
            &format!(
                "OpenAI Chat SSE done: {}chars text, {} tool_calls",
                collected_text.len(),
                tool_calls.len()
            ),
            Some(
                json!({
                    "text_length": collected_text.len(),
                    "tool_calls": tool_names,
                    "tool_call_count": tool_calls.len(),
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_creation": usage.cache_creation_input_tokens,
                        "cache_read": usage.cache_read_input_tokens,
                    }
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok((
        collected_text,
        tool_calls,
        usage,
        collected_thinking,
        first_token_time,
    ))
}
