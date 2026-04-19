//! OpenAI Responses API adapter implementing [`StreamingChatAdapter`].
//!
//! Owns body construction (using [`ResponsesRequest`] struct with
//! `instructions` + `input` fields), HTTP send, SSE event decoding (with
//! `response.output_text.delta` / `response.function_call_arguments.delta` /
//! reasoning summary events), and history persistence as Responses native
//! items (`function_call` + `function_call_output` + raw `reasoning` items).
//!
//! The SSE parser ([`parse_openai_sse`]) is shared with the Codex adapter
//! since they speak the same protocol — only auth header and endpoint differ.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::{FunctionCallItem, ResponsesRequest, SseEvent};
use super::super::config::build_api_url;
use super::super::events::{build_responses_tool_result, emit_text_delta, emit_thinking_delta};
use super::super::streaming_adapter::{
    ExecutedTool, RoundOutcome, RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ChatUsage, ProviderFormat};
use crate::tools::ToolProvider;

pub(crate) struct OpenAIResponsesStreamingAdapter<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
    /// Resolved Responses `reasoning` config for this turn (built by
    /// [`AssistantAgent::resolve_reasoning_config`] which clamps to model's
    /// supported range). `None` = reasoning disabled.
    pub reasoning: Option<super::super::api_types::ReasoningConfig>,
}

#[async_trait]
impl<'a> StreamingChatAdapter for OpenAIResponsesStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::OpenAIResponses
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
        // Inject awareness suffix (and active memory suffix) as leading
        // system items in the input array. These live OUTSIDE `instructions`
        // so suffix churn never invalidates the static instruction prefix
        // (which OpenAI auto-caches).
        let mut api_input: Vec<Value> = req.history_for_api.to_vec();
        if let Some(active_suffix) = req.active_memory_suffix {
            if !active_suffix.is_empty() {
                api_input.insert(
                    0,
                    json!({
                        "role": "system",
                        "content": active_suffix
                    }),
                );
            }
        }
        if let Some(suffix) = req.awareness_suffix {
            if !suffix.is_empty() {
                api_input.insert(
                    0,
                    json!({
                        "role": "system",
                        "content": suffix
                    }),
                );
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

        let api_url = build_api_url(self.base_url, "/v1/responses");

        // ── Log API request.
        let body_str = serde_json::to_string(&request).unwrap_or_default();
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
                "agent::chat_openai_responses::request",
                &format!(
                    "OpenAI Responses API request round {}: {} input items, {} tools, body {}B",
                    req.round,
                    api_input.len(),
                    req.tool_schemas.len(),
                    body_size
                ),
                Some(
                    json!({
                        "round": req.round,
                        "api_url": &api_url,
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

        // ── Send.
        let mut http_req = client
            .post(&api_url)
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            http_req = http_req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let request_start = std::time::Instant::now();
        let resp = http_req
            .json(&request)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("OpenAI Responses API request failed: {}", e))?;

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
                "agent::chat_openai_responses::response",
                &format!(
                    "OpenAI Responses API response: status={}, request_id={}, ttfb={}ms",
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
                    "agent::chat_openai_responses::error",
                    &format!("OpenAI Responses API error ({}): {}", status, error_preview),
                    Some(
                        json!({"status": status, "error": error_text, "round": req.round})
                            .to_string(),
                    ),
                    None,
                    None,
                );
            }
            return Err(anyhow::anyhow!(
                "OpenAI Responses API error ({}): {}",
                status,
                error_text
            ));
        }

        let (text, tool_calls, usage, thinking_text, ttft_ms, reasoning_items) =
            parse_openai_sse(resp, request_start, cancel, on_delta).await?;

        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_openai_responses::tool_loop",
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
            reasoning_items, // raw items pushed by orchestrator via append_reasoning_items
        })
    }

    fn append_reasoning_items(&self, history: &mut Vec<Value>, outcome: &RoundOutcome) {
        // Push raw reasoning items unchanged so encrypted_content / summary
        // fields are byte-perfect for next-turn input replay. Not stamped
        // with `_oc_round` — they're not Responses tool round artifacts and
        // need to survive compaction-tier round-boundary slicing.
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
        // Per executed tool: function_call item + function_call_output item.
        // If the tool returned __IMAGE_BASE64__ markers, build_responses_tool_result
        // additionally returns image input items (one per image), pushed unstamped
        // because they're orphan user-role messages, not part of a tool-round pair.
        for et in executed {
            let (text_output, image_items) = build_responses_tool_result(&et.clean_result);
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
                    "output": text_output,
                }),
                round,
            );
            for img_item in image_items {
                history.push(img_item);
            }
        }
    }

    fn append_final_assistant(
        &self,
        history: &mut Vec<Value>,
        final_text: &str,
        _last_thinking: &str,
    ) {
        // Responses API final assistant is a `message` item with `output_text`
        // content. Thinking already lives in history as standalone reasoning
        // items pushed by `append_reasoning_items` each round.
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

/// Parse OpenAI SSE stream (Responses API + Codex share this).
/// Returns `(collected_text, tool_calls, usage, thinking, ttft_ms, reasoning_items)`.
///
/// `pub(super)` so [`super::codex_adapter`] can reuse it without duplication.
pub(super) async fn parse_openai_sse(
    resp: reqwest::Response,
    request_start: std::time::Instant,
    cancel: &Arc<AtomicBool>,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    Vec<FunctionCallItem>,
    ChatUsage,
    String,
    Option<u64>,
    Vec<Value>,
)> {
    use futures_util::StreamExt;

    let mut collected_text = String::new();
    let mut collected_thinking = String::new();
    let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
    let mut pending_calls: std::collections::HashMap<String, FunctionCallItem> =
        std::collections::HashMap::new();
    let mut usage = ChatUsage::default();
    let mut first_token_time: Option<u64> = None;
    let mut reasoning_items: Vec<Value> = Vec::new();

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
                            if first_token_time.is_none() {
                                first_token_time = Some(request_start.elapsed().as_millis() as u64);
                            }
                            emit_thinking_delta(&on_delta, delta);
                            collected_thinking.push_str(delta);
                        }
                    }

                    // Reasoning summary part done — paragraph separator between parts.
                    "response.reasoning_summary_part.done" => {
                        collected_thinking.push_str("\n\n");
                        emit_thinking_delta(&on_delta, "\n\n");
                    }

                    // Text deltas
                    "response.output_text.delta" => {
                        if let Some(delta) = &event.delta {
                            if first_token_time.is_none() {
                                first_token_time = Some(request_start.elapsed().as_millis() as u64);
                            }
                            emit_text_delta(&on_delta, delta);
                            collected_text.push_str(delta);
                        }
                    }

                    // Function call started
                    "response.output_item.added" => {
                        if let Some(item) = &event.item {
                            if item.item_type.as_deref() == Some("function_call") {
                                let call_id = item
                                    .id
                                    .clone()
                                    .or_else(|| item.call_id.clone())
                                    .unwrap_or_default();
                                let name = item.name.clone().unwrap_or_default();
                                pending_calls.insert(
                                    call_id.clone(),
                                    FunctionCallItem {
                                        call_id,
                                        name,
                                        arguments: item.arguments.clone().unwrap_or_default(),
                                    },
                                );
                            }
                        }
                    }

                    // Function call arguments delta
                    "response.function_call_arguments.delta" => {
                        if let Some(delta) = &event.delta {
                            if let Some(item) = &event.item {
                                let call_id = item
                                    .id
                                    .clone()
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
                                let call_id = item
                                    .id
                                    .clone()
                                    .or_else(|| item.call_id.clone())
                                    .unwrap_or_default();
                                if let Some(mut tc) = pending_calls.remove(&call_id) {
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
                            // Capture reasoning items raw (preserves encrypted_content).
                            if item.item_type.as_deref() == Some("reasoning") {
                                if let Ok(raw) = serde_json::from_str::<Value>(&data) {
                                    if let Some(raw_item) = raw.get("item") {
                                        reasoning_items.push(raw_item.clone());
                                    }
                                }
                            }
                        }
                    }

                    "error" => {
                        let msg = event
                            .message
                            .as_deref()
                            .or(event.code.as_deref())
                            .unwrap_or("Unknown error");
                        return Err(anyhow::anyhow!("Codex error: {}", msg));
                    }
                    "response.failed" => {
                        let msg = event
                            .response
                            .as_ref()
                            .and_then(|r| r.error.as_ref())
                            .and_then(|e| e.message.as_deref())
                            .unwrap_or("Codex response failed");
                        return Err(anyhow::anyhow!("{}", msg));
                    }

                    // Response completed — extract from full response if no deltas collected.
                    "response.completed" | "response.done" => {
                        if let Some(resp_obj) = &event.response {
                            if let Some(u) = &resp_obj.usage {
                                if let Some(it) = u.input_tokens {
                                    usage.input_tokens = it;
                                }
                                if let Some(ot) = u.output_tokens {
                                    usage.output_tokens = ot;
                                }
                                // Anthropic-style cache token fields.
                                if let Some(cr) = u.cache_read_input_tokens {
                                    usage.cache_read_input_tokens = cr;
                                }
                                if let Some(cc) = u.cache_creation_input_tokens {
                                    usage.cache_creation_input_tokens = cc;
                                }
                                // OpenAI-style fallback.
                                if usage.cache_read_input_tokens == 0 {
                                    usage.cache_read_input_tokens = u
                                        .input_tokens_details
                                        .as_ref()
                                        .and_then(|d| d.cached_tokens)
                                        .or_else(|| {
                                            u.prompt_tokens_details
                                                .as_ref()
                                                .and_then(|d| d.cached_tokens)
                                        })
                                        .unwrap_or(0);
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
                                                    if part.part_type.as_deref()
                                                        == Some("output_text")
                                                    {
                                                        if let Some(text) = &part.text {
                                                            collected_text.push_str(text);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        if item.item_type.as_deref() == Some("function_call") {
                                            let call_id = item
                                                .id
                                                .clone()
                                                .or_else(|| item.call_id.clone())
                                                .unwrap_or_default();
                                            tool_calls.push(FunctionCallItem {
                                                call_id,
                                                name: item.name.clone().unwrap_or_default(),
                                                arguments: item
                                                    .arguments
                                                    .clone()
                                                    .unwrap_or_default(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    // Drain remaining pending calls.
    for (_, tc) in pending_calls {
        tool_calls.push(tc);
    }

    if let Some(logger) = crate::get_logger() {
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        logger.log(
            "debug",
            "agent",
            "agent::parse_openai_sse::done",
            &format!(
                "OpenAI Responses SSE done: {}chars text, {} tool_calls",
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
        reasoning_items,
    ))
}
