use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::json;

use crate::tools::{self, ToolProvider};
use super::super::api_types::FunctionCallItem;
use super::super::config::{apply_thinking_to_chat_body, build_api_url, get_max_tool_rounds};
use super::super::content::build_user_content_openai_chat;
use super::super::events::{
    emit_text_delta, emit_thinking_delta, emit_tool_call, emit_tool_result, emit_usage, extract_media_urls,
    build_openai_chat_tool_result_content,
};
use super::super::types::{AssistantAgent, Attachment, ChatUsage, ThinkTagFilter};

impl AssistantAgent {
    // ── OpenAI Chat Completions API with Tool Loop ───────────────

    pub(crate) async fn chat_openai_chat(&self, api_key: &str, base_url: &str, model: &str, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<(String, Option<String>)> {
        let client = crate::provider::apply_proxy(
            reqwest::Client::builder().user_agent(&self.user_agent)
        )
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let mut tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);
        if self.notification_enabled {
            tool_schemas.push(tools::get_notification_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        if let Some(ref img_config) = self.image_gen_config {
            tool_schemas.push(tools::get_image_generate_tool_dynamic(img_config).to_provider_schema(ToolProvider::OpenAI));
        }
        if self.canvas_enabled {
            tool_schemas.push(tools::get_canvas_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        if self.subagent_tool_enabled() {
            tool_schemas.push(tools::get_subagent_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        // Filter out denied tools (depth-based tool policy)
        if !self.denied_tools.is_empty() {
            tool_schemas.retain(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                !self.denied_tools.contains(&name.to_string())
            });
        }

        let mut messages = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_openai_chat(message, attachments);
        Self::push_user_message(&mut messages, user_content);

        let api_url = build_api_url(base_url, "/v1/chat/completions");
        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
        let mut total_usage = ChatUsage::default();
        let mut first_ttft_ms: Option<u64> = None;
        let system_prompt = self.build_full_system_prompt(model, "OpenAIChat");

        // Run context compaction (Tier 1-3) before API call
        self.run_compaction(&mut messages, &system_prompt, 16384, on_delta).await;

        // Apply thinking parameters based on ThinkingStyle

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 { u32::MAX } else { max_rounds };
        let mut round_count: u32 = 0;
        for round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) { break; }
            round_count = round + 1;

            // Drain steer mailbox: inject any pending steer messages as user messages
            if let Some(ref rid) = self.steer_run_id {
                for msg in crate::subagent::SUBAGENT_MAILBOX.drain(rid) {
                    Self::push_user_message(&mut messages, serde_json::json!(format!("[Steer from parent agent]: {}", msg)));
                }
            }

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

            // Log API request details (including raw body for debugging)
            let body_str = serde_json::to_string(&body).unwrap_or_default();
            if let Some(logger) = crate::get_logger() {
                let body_size = body_str.len();
                let raw_body = if body_size > 32768 {
                    format!("{}...(truncated, total {}B)", crate::truncate_utf8(&body_str, 32768), body_size)
                } else {
                    body_str.clone()
                };
                let raw_body = crate::logging::redact_sensitive(&raw_body);
                logger.log("debug", "agent", "agent::chat_openai_chat::request",
                    &format!("OpenAI Chat API request round {}: {} messages, {} tools, body {}B",
                        round, api_messages.len(), tools_array.len(), body_size),
                    Some(json!({
                        "round": round,
                        "api_url": &api_url,
                        "model": model,
                        "message_count": api_messages.len(),
                        "tool_count": tools_array.len(),
                        "body_size_bytes": body_size,
                        "request_body": raw_body,
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
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("OpenAI Chat API request failed: {}", e))?;

            // Log API response status with headers
            if let Some(logger) = crate::get_logger() {
                let status = resp.status().as_u16();
                let headers = resp.headers();
                let request_id = headers.get("x-request-id")
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
                logger.log("debug", "agent", "agent::chat_openai_chat::response",
                    &format!("OpenAI Chat API response: status={}, request_id={}, ttfb={}ms", status, request_id, ttfb_ms),
                    Some(json!({
                        "status": status,
                        "request_id": request_id,
                        "ttfb_ms": ttfb_ms,
                        "round": round,
                        "response_headers": response_headers,
                    }).to_string()),
                    None, None);
            }

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let error_text = resp.text().await.unwrap_or_default();
                if let Some(logger) = crate::get_logger() {
                    let error_preview = if error_text.len() > 500 { format!("{}...", crate::truncate_utf8(&error_text, 500)) } else { error_text.clone() };
                    logger.log("error", "agent", "agent::chat_openai_chat::error",
                        &format!("OpenAI Chat API error ({}): {}", status, error_preview),
                        Some(json!({"status": status, "error": error_text, "round": round}).to_string()),
                        None, None);
                }
                return Err(anyhow::anyhow!("OpenAI Chat API error ({}): {}", status, error_text));
            }

            // Parse SSE stream for Chat Completions format
            let (text, tool_calls, round_usage, thinking, round_ttft) = self.parse_chat_completions_sse(resp, request_start, reasoning_effort, cancel, on_delta).await?;
            if first_ttft_ms.is_none() {
                first_ttft_ms = round_ttft;
            }
            collected_text.push_str(&text);
            collected_thinking.push_str(&thinking);
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
                logger.log("info", "agent", "agent::chat_openai_chat::tool_loop",
                    &format!("Tool loop round {}: executing {} tools: {:?}", round, tool_calls.len(), tool_names),
                    Some(json!({
                        "round": round,
                        "tool_count": tool_calls.len(),
                        "tools": tool_names,
                    }).to_string()),
                    None, None);
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
                // Check cancel before each tool execution
                if cancel.load(Ordering::SeqCst) { break; }

                let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);

                // Log tool execution input
                if let Some(logger) = crate::get_logger() {
                    let args_str = tc.arguments.as_str();
                    let args_preview = if args_str.len() > 2048 {
                        format!("{}...(truncated, total {}B)", crate::truncate_utf8(args_str, 2048), args_str.len())
                    } else {
                        args_str.to_string()
                    };
                    logger.log("debug", "agent", "agent::tool_exec::input",
                        &format!("Tool exec [{}] id={}", tc.name, tc.call_id),
                        Some(json!({
                            "tool_name": tc.name,
                            "call_id": tc.call_id,
                            "arguments": args_preview,
                            "round": round,
                        }).to_string()),
                        None, None);
                }

                let tool_start = std::time::Instant::now();
                // Use tokio::select! to race tool execution against cancel flag
                let cancel_clone = cancel.clone();
                let tool_ctx = self.tool_context();
                let result = tokio::select! {
                    res = tools::execute_tool_with_context(&tc.name, &args, &tool_ctx) => {
                        match res {
                            Ok(r) => r,
                            Err(e) => format!("Tool error: {}", e),
                        }
                    }
                    _ = async {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            if cancel_clone.load(Ordering::SeqCst) { break; }
                        }
                    } => {
                        String::from("Tool execution cancelled by user")
                    }
                };
                let tool_elapsed_ms = tool_start.elapsed().as_millis() as u64;

                // Log tool execution output
                if let Some(logger) = crate::get_logger() {
                    let result_preview = if result.len() > 2048 {
                        format!("{}...(truncated, total {}B)", crate::truncate_utf8(&result, 2048), result.len())
                    } else {
                        result.clone()
                    };
                    let is_error = result.starts_with("Tool error:");
                    logger.log(if is_error { "warn" } else { "debug" }, "agent", "agent::tool_exec::output",
                        &format!("Tool result [{}] {}B, {}ms{}", tc.name, result.len(), tool_elapsed_ms, if is_error { " (ERROR)" } else { "" }),
                        Some(json!({
                            "tool_name": tc.name,
                            "call_id": tc.call_id,
                            "result_size_bytes": result.len(),
                            "elapsed_ms": tool_elapsed_ms,
                            "is_error": is_error,
                            "result_preview": result_preview,
                            "round": round,
                        }).to_string()),
                        None, None);
                }

                let is_tool_error = result.starts_with("Tool error:");
                let (clean_result, media_urls) = extract_media_urls(&result);
                emit_tool_result(on_delta, &tc.call_id, &tc.name, &clean_result, tool_elapsed_ms, is_tool_error, &media_urls);

                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc.call_id,
                    "content": build_openai_chat_tool_result_content(&clean_result),
                }));
            }

            // Tier 1 quick check: truncate any oversized tool results added this round
            crate::context_compact::truncate_tool_results(&mut messages, self.context_window, &self.compact_config);
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from OpenAI Chat API"));
        }

        if !collected_text.is_empty() {
            messages.push(json!({ "role": "assistant", "content": collected_text }));
        }
        *self.conversation_history.lock().unwrap() = messages;

        // Emit accumulated usage (with TTFT)
        emit_usage(on_delta, &total_usage, model, first_ttft_ms);

        // Log chat completion summary
        if let Some(logger) = crate::get_logger() {
            let history_len = self.conversation_history.lock().unwrap().len();
            logger.log("info", "agent", "agent::chat_openai_chat::done",
                &format!("OpenAI Chat complete: {}chars, {} rounds, usage in={}/out={}",
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

        let thinking_result = if collected_thinking.is_empty() { None } else { Some(collected_thinking) };
        Ok((collected_text, thinking_result))
    }

    /// Parse OpenAI Chat Completions SSE stream
    async fn parse_chat_completions_sse(
        &self,
        resp: reqwest::Response,
        request_start: std::time::Instant,
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>, ChatUsage, String, Option<u64>)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
        let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
        // Track tool calls by index
        let mut pending_calls: std::collections::HashMap<usize, FunctionCallItem> = std::collections::HashMap::new();
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
                                        if first_token_time.is_none() {
                                            first_token_time = Some(request_start.elapsed().as_millis() as u64);
                                        }
                                        emit_thinking_delta(on_delta, reasoning);
                                        collected_thinking.push_str(reasoning);
                                    }
                                }

                                // Text content — filter <think>...</think> tags from content stream
                                // Qwen models may embed thinking in content via <think> tags
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                    let (text_part, think_part) = think_filter.process(content);
                                    // When thinking is enabled, redirect <think> content as thinking_delta;
                                    // when disabled ("none"), discard it entirely
                                    if !think_part.is_empty() && reasoning_effort != Some("none") {
                                        emit_thinking_delta(on_delta, &think_part);
                                        collected_thinking.push_str(&think_part);
                                    }
                                    if !text_part.is_empty() {
                                        if first_token_time.is_none() {
                                            first_token_time = Some(request_start.elapsed().as_millis() as u64);
                                        }
                                        emit_text_delta(on_delta, &text_part);
                                        collected_text.push_str(&text_part);
                                    }
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

        // Log SSE stream completion
        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            logger.log("debug", "agent", "agent::parse_chat_completions_sse::done",
                &format!("OpenAI Chat SSE done: {}chars text, {} tool_calls",
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

        Ok((collected_text, tool_calls, usage, collected_thinking, first_token_time))
    }
}
