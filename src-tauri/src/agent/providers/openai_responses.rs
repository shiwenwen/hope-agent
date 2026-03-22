use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::json;

use crate::tools::{self, ToolProvider};
use super::super::api_types::{FunctionCallItem, ReasoningConfig, ResponsesRequest, SseEvent};
use super::super::config::{build_api_url, clamp_reasoning_effort, get_max_tool_rounds};
use super::super::content::build_user_content_responses;
use super::super::events::{
    emit_text_delta, emit_thinking_delta, emit_tool_call, emit_tool_result, emit_usage,
};
use super::super::types::{AssistantAgent, Attachment, ChatUsage};

impl AssistantAgent {
    // ── OpenAI Responses API (custom base_url) ────────────────────

    pub(crate) async fn chat_openai_responses(&self, api_key: &str, base_url: &str, model: &str, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<String> {
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let mut tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);
        if self.notification_enabled {
            tool_schemas.push(tools::get_notification_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        if self.subagent_tool_enabled() {
            tool_schemas.push(tools::get_subagent_tool().to_provider_schema(ToolProvider::OpenAI));
        }

        let reasoning = reasoning_effort
            .and_then(|e| clamp_reasoning_effort(model, e))
            .map(|effort| ReasoningConfig { effort, summary: "auto".to_string() });

        let mut input: Vec<serde_json::Value> = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_responses(message, attachments);
        Self::push_user_message(&mut input, user_content);

        let api_url = build_api_url(base_url, "/v1/responses");
        let mut collected_text = String::new();
        let mut total_usage = ChatUsage::default();
        let system_prompt = self.build_full_system_prompt(model, "OpenAIResponses");

        // Run context compaction (Tier 1-3) before API call
        self.run_compaction(&mut input, &system_prompt, 16384, on_delta).await;

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 { u32::MAX } else { max_rounds };
        let mut round_count: u32 = 0;
        for round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) { break; }
            round_count = round + 1;

            let request = ResponsesRequest {
                model: model.to_string(),
                store: false,
                stream: true,
                instructions: system_prompt.clone(),
                input: input.clone(),
                reasoning: reasoning.as_ref().map(|r| ReasoningConfig { effort: r.effort.clone(), summary: "auto".to_string() }),
                tools: Some(tool_schemas.clone()),
            };

            // Log API request details
            if let Some(logger) = crate::get_logger() {
                let body_size = serde_json::to_string(&request).map(|s| s.len()).unwrap_or(0);
                logger.log("debug", "agent", "agent::chat_openai_responses::request",
                    &format!("OpenAI Responses API request round {}: {} input items, {} tools, body {}B",
                        round, input.len(), tool_schemas.len(), body_size),
                    Some(json!({
                        "round": round,
                        "api_url": &api_url,
                        "model": model,
                        "input_count": input.len(),
                        "tool_count": tool_schemas.len(),
                        "body_size_bytes": body_size,
                        "reasoning": reasoning.as_ref().map(|r| r.effort.as_str()),
                    }).to_string()),
                    None, None);
            }

            let mut req = client
                .post(&api_url)
                .header("Content-Type", "application/json");
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }
            let request_start = std::time::Instant::now();
            let resp = req
                .json(&request)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("OpenAI Responses API request failed: {}", e))?;

            // Log API response status
            if let Some(logger) = crate::get_logger() {
                let status = resp.status().as_u16();
                let request_id = resp.headers().get("x-request-id")
                    .or_else(|| resp.headers().get("request-id"))
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string();
                let ttfb_ms = request_start.elapsed().as_millis() as u64;
                logger.log("debug", "agent", "agent::chat_openai_responses::response",
                    &format!("OpenAI Responses API response: status={}, request_id={}, ttfb={}ms", status, request_id, ttfb_ms),
                    Some(json!({
                        "status": status,
                        "request_id": request_id,
                        "ttfb_ms": ttfb_ms,
                        "round": round,
                    }).to_string()),
                    None, None);
            }

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let error_text = resp.text().await.unwrap_or_default();
                if let Some(logger) = crate::get_logger() {
                    let error_preview = if error_text.len() > 500 { format!("{}...", crate::truncate_utf8(&error_text, 500)) } else { error_text.clone() };
                    logger.log("error", "agent", "agent::chat_openai_responses::error",
                        &format!("OpenAI Responses API error ({}): {}", status, error_preview),
                        Some(json!({"status": status, "error": error_text, "round": round}).to_string()),
                        None, None);
                }
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

            // Log tool loop progress
            if let Some(logger) = crate::get_logger() {
                let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                logger.log("info", "agent", "agent::chat_openai_responses::tool_loop",
                    &format!("Tool loop round {}: executing {} tools: {:?}", round, tool_calls.len(), tool_names),
                    Some(json!({
                        "round": round,
                        "tool_count": tool_calls.len(),
                        "tools": tool_names,
                    }).to_string()),
                    None, None);
            }

            for tc in &tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);

                let result = match tools::execute_tool_with_context(&tc.name, &args, &self.tool_context()).await {
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

            // Tier 1 quick check: truncate any oversized tool results added this round
            crate::context_compact::truncate_tool_results(&mut input, self.context_window, &self.compact_config);
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
        emit_usage(on_delta, &total_usage, model);

        // Log chat completion summary
        if let Some(logger) = crate::get_logger() {
            let history_len = self.conversation_history.lock().unwrap().len();
            logger.log("info", "agent", "agent::chat_openai_responses::done",
                &format!("OpenAI Responses chat complete: {}chars, {} rounds, usage in={}/out={}",
                    collected_text.len(), round_count, total_usage.input_tokens, total_usage.output_tokens),
                Some(json!({
                    "text_length": collected_text.len(),
                    "total_rounds": round_count,
                    "history_length": history_len,
                    "cancelled": cancelled,
                    "model": model,
                    "usage": {
                        "input_tokens": total_usage.input_tokens,
                        "output_tokens": total_usage.output_tokens,
                        "cache_creation": total_usage.cache_creation_input_tokens,
                        "cache_read": total_usage.cache_read_input_tokens,
                    }
                }).to_string()),
                None, None);
        }

        Ok(collected_text)
    }

    /// Parse OpenAI SSE stream. Returns (collected_text, tool_calls, usage)
    /// Shared by both OpenAI Responses API and Codex providers.
    pub(crate) async fn parse_openai_sse(
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
                                    // Anthropic-style: cache_read_input_tokens / cache_creation_input_tokens
                                    if let Some(cr) = u.cache_read_input_tokens {
                                        usage.cache_read_input_tokens = cr;
                                    }
                                    if let Some(cc) = u.cache_creation_input_tokens {
                                        usage.cache_creation_input_tokens = cc;
                                    }
                                    // OpenAI-style: input_tokens_details.cached_tokens
                                    if usage.cache_read_input_tokens == 0 {
                                        if let Some(details) = &u.input_tokens_details {
                                            if let Some(cached) = details.cached_tokens {
                                                usage.cache_read_input_tokens = cached;
                                            }
                                        }
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

        // Log SSE stream completion
        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            logger.log("debug", "agent", "agent::parse_openai_sse::done",
                &format!("OpenAI Responses SSE done: {}chars text, {} tool_calls",
                    collected_text.len(), tool_calls.len()),
                Some(json!({
                    "text_length": collected_text.len(),
                    "tool_calls": tool_names,
                    "tool_call_count": tool_calls.len(),
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_creation": usage.cache_creation_input_tokens,
                        "cache_read": usage.cache_read_input_tokens,
                    }
                }).to_string()),
                None, None);
        }

        Ok((collected_text, tool_calls, usage))
    }
}
