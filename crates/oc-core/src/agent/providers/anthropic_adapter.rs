//! Anthropic Messages API adapter implementing [`StreamingChatAdapter`].
//!
//! Owns body construction (with `cache_control` ephemeral blocks for prompt
//! caching), HTTP send, SSE event decoding (text / thinking / tool_use blocks
//! + stop_reason), and history persistence in Anthropic's content-block shape.
//!
//! Phase 2 of the LLM call unification — the public tool loop lives in
//! [`super::super::streaming_loop`]. See `docs/architecture/side-query.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::{AnthropicSseEvent, FunctionCallItem};
use super::super::config::{
    build_api_url, map_think_anthropic_style, ANTHROPIC_API_VERSION,
};
use super::super::events::{
    build_anthropic_tool_result_content, emit_text_delta, emit_thinking_delta,
};
use super::super::streaming_adapter::{
    ExecutedTool, RoundOutcome, RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ChatUsage, ProviderFormat};
use crate::tools::ToolProvider;

pub(crate) struct AnthropicStreamingAdapter<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
}

#[async_trait]
impl<'a> StreamingChatAdapter for AnthropicStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::Anthropic
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::Anthropic
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_anthropic(history);
    }

    async fn chat_round(
        &self,
        client: &reqwest::Client,
        req: RoundRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<RoundOutcome> {
        // ── Build system blocks with cache_control ephemeral breakpoints.
        // Three independent blocks: static prefix, awareness suffix (if any),
        // active memory suffix (if any). Suffix churn only invalidates the
        // suffix's own cache, not the static prefix.
        let mut system_blocks = vec![json!({
            "type": "text",
            "text": req.system_prompt,
            "cache_control": { "type": "ephemeral" }
        })];
        if let Some(suffix) = req.awareness_suffix {
            if !suffix.is_empty() {
                system_blocks.push(json!({
                    "type": "text",
                    "text": suffix,
                    "cache_control": { "type": "ephemeral" }
                }));
            }
        }
        if let Some(active_suffix) = req.active_memory_suffix {
            if !active_suffix.is_empty() {
                system_blocks.push(json!({
                    "type": "text",
                    "text": active_suffix,
                    "cache_control": { "type": "ephemeral" }
                }));
            }
        }
        let system_with_cache = json!(system_blocks);

        // Tools are static — add cache_control to the last tool definition.
        let mut tools_with_cache: Vec<Value> = req.tool_schemas.to_vec();
        if let Some(last_tool) = tools_with_cache.last_mut() {
            last_tool["cache_control"] = json!({ "type": "ephemeral" });
        }

        let thinking = map_think_anthropic_style(req.reasoning_effort, req.max_tokens);

        // Body field order: model, max_tokens, system, messages, stream
        // (then conditional tools / thinking / temperature). Must match the
        // pre-Phase-2 chat_anthropic byte-level for prompt cache stability.
        let mut body = json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "system": system_with_cache,
            "messages": req.history_for_api,
            "stream": true,
        });
        if !req.is_final_round {
            body["tools"] = json!(tools_with_cache);
        }
        if let Some(ref think_config) = thinking {
            body["thinking"] = think_config.clone();
        }
        if let Some(temp) = req.temperature {
            body["temperature"] = json!(temp);
        }

        let api_url = build_api_url(self.base_url, "/v1/messages");

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
                "agent::chat_anthropic::request",
                &format!(
                    "Anthropic API request round {}: {} messages, {} tools, body {}B",
                    req.round,
                    req.history_for_api.len(),
                    req.tool_schemas.len(),
                    body_size
                ),
                Some(
                    json!({
                        "round": req.round,
                        "api_url": &api_url,
                        "model": self.model,
                        "message_count": req.history_for_api.len(),
                        "tool_count": req.tool_schemas.len(),
                        "body_size_bytes": body_size,
                        "thinking_enabled": thinking.is_some(),
                        "request_body": raw_body,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        // ── Send.
        let request_start = std::time::Instant::now();
        let resp = client
            .post(&api_url)
            .header("x-api-key", self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Anthropic API request failed: {}", e))?;

        // ── Log response status with rate-limit headers for debugging.
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
                "anthropic-model-id": headers.get("anthropic-model-id").and_then(|v| v.to_str().ok()),
                "retry-after": headers.get("retry-after").and_then(|v| v.to_str().ok()),
            });
            logger.log(
                "debug",
                "agent",
                "agent::chat_anthropic::response",
                &format!(
                    "Anthropic API response: status={}, request_id={}, ttfb={}ms",
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
                    "agent::chat_anthropic::error",
                    &format!("Anthropic API error ({}): {}", status, error_preview),
                    Some(
                        json!({"status": status, "error": error_text, "round": req.round})
                            .to_string(),
                    ),
                    None,
                    None,
                );
            }
            return Err(anyhow::anyhow!(
                "Anthropic API error ({}): {}",
                status,
                error_text
            ));
        }

        // ── Parse SSE stream.
        let (text, tool_calls, stop_reason, usage, thinking_text, ttft_ms) =
            parse_anthropic_sse(resp, request_start, cancel, on_delta).await?;

        // Log tool loop progress (moved here from the old chat_anthropic so
        // the orchestrator stays oblivious to which tools were requested).
        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_anthropic::tool_loop",
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
            stop_reason,
            reasoning_items: Vec::new(), // Anthropic puts thinking inline, no raw items
        })
    }

    fn append_round_to_history(
        &self,
        history: &mut Vec<Value>,
        round: u32,
        outcome: &RoundOutcome,
        executed: &[ExecutedTool],
    ) {
        // Build assistant content blocks. Order matters per Anthropic spec:
        //   thinking → text → tool_use
        let mut assistant_content: Vec<Value> = Vec::new();
        if !outcome.thinking.is_empty() {
            assistant_content.push(json!({
                "type": "thinking",
                "thinking": outcome.thinking,
            }));
        }
        if !outcome.text.is_empty() {
            assistant_content.push(json!({
                "type": "text",
                "text": outcome.text,
            }));
        }
        for tc in &outcome.tool_calls {
            let args: Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
            assistant_content.push(json!({
                "type": "tool_use",
                "id": tc.call_id,
                "name": tc.name,
                "input": args,
            }));
        }
        crate::context_compact::push_and_stamp(
            history,
            json!({ "role": "assistant", "content": assistant_content }),
            round,
        );

        // Build user content with tool_result blocks (one per executed tool).
        let mut tool_results: Vec<Value> = Vec::new();
        for et in executed {
            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": et.call_id,
                "content": build_anthropic_tool_result_content(&et.clean_result),
            }));
        }
        if !tool_results.is_empty() {
            crate::context_compact::push_and_stamp(
                history,
                json!({ "role": "user", "content": tool_results }),
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
        let mut final_content: Vec<Value> = Vec::new();
        if !last_thinking.is_empty() {
            final_content.push(json!({
                "type": "thinking",
                "thinking": last_thinking,
            }));
        }
        if !final_text.is_empty() {
            final_content.push(json!({
                "type": "text",
                "text": final_text,
            }));
        }
        if !final_content.is_empty() {
            history.push(json!({ "role": "assistant", "content": final_content }));
        }
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        // Anthropic's terminal signal is `stop_reason != "tool_use"`. If the
        // model picked tools in this round but the stop reason isn't
        // "tool_use" (e.g. "max_tokens"), bail before executing them — sending
        // tool_results back without a tool_use stop would desync the chain.
        outcome.tool_calls.is_empty() || outcome.stop_reason.as_deref() != Some("tool_use")
    }
}

/// Parse Anthropic SSE stream. Returns
/// `(collected_text, tool_calls, stop_reason, usage, thinking, ttft_ms)`.
///
/// Free function (not a `&self` method) because none of the streaming state
/// lives on `AssistantAgent` — only `cancel`, `on_delta`, and accumulators.
async fn parse_anthropic_sse(
    resp: reqwest::Response,
    request_start: std::time::Instant,
    cancel: &Arc<AtomicBool>,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    Vec<FunctionCallItem>,
    Option<String>,
    ChatUsage,
    String,
    Option<u64>,
)> {
    use futures_util::StreamExt;

    let mut collected_text = String::new();
    let mut collected_thinking = String::new();
    let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
    // Single in-flight tool-use block — Anthropic streams them sequentially,
    // not in parallel, so a single slot is sufficient.
    let mut current_tool: Option<(usize, FunctionCallItem)> = None;
    let mut in_thinking_block = false;
    let mut usage = ChatUsage::default();
    let mut stop_reason: Option<String> = None;
    let mut first_token_time: Option<u64> = None;

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

            // SSE event format: "event: <type>\ndata: <json>"
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
                                    current_tool = Some((
                                        idx,
                                        FunctionCallItem {
                                            call_id: block.id.clone().unwrap_or_default(),
                                            name: block.name.clone().unwrap_or_default(),
                                            arguments: String::new(),
                                        },
                                    ));
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
                                        if first_token_time.is_none() {
                                            first_token_time = Some(
                                                request_start.elapsed().as_millis() as u64,
                                            );
                                        }
                                        emit_thinking_delta(&on_delta, text);
                                        collected_thinking.push_str(text);
                                    }
                                }
                                Some("text_delta") => {
                                    if let Some(text) = &delta.text {
                                        if first_token_time.is_none() {
                                            first_token_time = Some(
                                                request_start.elapsed().as_millis() as u64,
                                            );
                                        }
                                        emit_text_delta(&on_delta, text);
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
                        if let Some(u) = &event.usage {
                            if let Some(ot) = u.output_tokens {
                                usage.output_tokens = ot;
                            }
                        }
                    }
                    "error" => {
                        let msg = event
                            .error
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

    if let Some(logger) = crate::get_logger() {
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        logger.log(
            "debug",
            "agent",
            "agent::parse_anthropic_sse::done",
            &format!(
                "Anthropic SSE done: {}chars text, {} tool_calls, stop={:?}",
                collected_text.len(),
                tool_calls.len(),
                stop_reason
            ),
            Some(
                json!({
                    "text_length": collected_text.len(),
                    "tool_calls": tool_names,
                    "tool_call_count": tool_calls.len(),
                    "stop_reason": stop_reason,
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
        stop_reason,
        usage,
        collected_thinking,
        first_token_time,
    ))
}
