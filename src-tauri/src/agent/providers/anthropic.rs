use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::json;

use crate::tools::{self, ToolProvider};
use super::super::api_types::{AnthropicSseEvent, FunctionCallItem};
use super::super::config::{build_api_url, get_max_tool_rounds, map_think_anthropic_style, ANTHROPIC_API_VERSION};
use super::super::content::build_user_content_anthropic;
use super::super::events::{
    build_anthropic_tool_result_content, emit_text_delta, emit_thinking_delta,
    emit_tool_call, emit_tool_result, emit_usage, extract_media_urls,
};
use super::super::types::{AssistantAgent, ChatUsage};

impl AssistantAgent {
    // ── Anthropic Messages API with Tool Loop ─────────────────────

    pub(crate) async fn chat_anthropic(&self, api_key: &str, base_url: &str, model: &str, message: &str, attachments: &[super::super::types::Attachment], reasoning_effort: Option<&str>, cancel: &Arc<AtomicBool>, on_delta: &(impl Fn(&str) + Send)) -> Result<(String, Option<String>)> {
        let client = crate::provider::apply_proxy(
            reqwest::Client::builder().user_agent(&self.user_agent)
        )
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let mut tool_schemas = tools::get_tools_for_provider(ToolProvider::Anthropic);
        if self.notification_enabled {
            tool_schemas.push(tools::get_notification_tool().to_provider_schema(ToolProvider::Anthropic));
        }
        if let Some(ref img_config) = self.image_gen_config {
            tool_schemas.push(tools::get_image_generate_tool_dynamic(img_config).to_provider_schema(ToolProvider::Anthropic));
        }
        if self.canvas_enabled {
            tool_schemas.push(tools::get_canvas_tool().to_provider_schema(ToolProvider::Anthropic));
        }
        if self.subagent_tool_enabled() {
            tool_schemas.push(tools::get_subagent_tool().to_provider_schema(ToolProvider::Anthropic));
        }
        // Filter out denied tools (depth-based tool policy)
        if !self.denied_tools.is_empty() {
            tool_schemas.retain(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                !self.denied_tools.contains(&name.to_string())
            });
        }

        // Build messages from conversation history + new user message (with optional image attachments)
        let mut messages = self.conversation_history.lock().unwrap().clone();
        let user_content = build_user_content_anthropic(message, attachments);
        Self::push_user_message(&mut messages, user_content);

        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
        let mut total_usage = ChatUsage::default();

        let api_url = build_api_url(base_url, "/v1/messages");
        let system_prompt = self.build_full_system_prompt(model, "Anthropic");

        // Run context compaction (Tier 1-3) before API call
        let max_tokens: u32 = 16384;
        self.run_compaction(&mut messages, &system_prompt, max_tokens, on_delta).await;

        // Map thinking effort for Anthropic
        let thinking = map_think_anthropic_style(reasoning_effort, max_tokens);

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

            // Build system prompt with cache_control for Anthropic prompt caching
            let system_with_cache = json!([{
                "type": "text",
                "text": system_prompt,
                "cache_control": { "type": "ephemeral" }
            }]);

            // Add cache_control to the last tool definition (tools are static, worth caching)
            let mut tools_with_cache = tool_schemas.clone();
            if let Some(last_tool) = tools_with_cache.last_mut() {
                last_tool["cache_control"] = json!({ "type": "ephemeral" });
            }

            let mut body = json!({
                "model": model,
                "max_tokens": max_tokens,
                "system": system_with_cache,
                "tools": tools_with_cache,
                "messages": messages,
                "stream": true,
            });

            // Add thinking parameter if enabled
            if let Some(ref think_config) = thinking {
                body["thinking"] = think_config.clone();
            }

            // Log API request details (including raw body for debugging)
            let body_str = serde_json::to_string(&body).unwrap_or_default();
            if let Some(logger) = crate::get_logger() {
                let body_size = body_str.len();
                // Truncate body to 32KB and redact sensitive values
                let raw_body = if body_size > 32768 {
                    format!("{}...(truncated, total {}B)", crate::truncate_utf8(&body_str, 32768), body_size)
                } else {
                    body_str.clone()
                };
                let raw_body = crate::logging::redact_sensitive(&raw_body);
                logger.log("debug", "agent", "agent::chat_anthropic::request",
                    &format!("Anthropic API request round {}: {} messages, {} tools, body {}B",
                        round, messages.len(), tool_schemas.len(), body_size),
                    Some(json!({
                        "round": round,
                        "api_url": &api_url,
                        "model": model,
                        "message_count": messages.len(),
                        "tool_count": tool_schemas.len(),
                        "body_size_bytes": body_size,
                        "thinking_enabled": thinking.is_some(),
                        "request_body": raw_body,
                    }).to_string()),
                    None, None);
            }

            let request_start = std::time::Instant::now();
            let resp = client
                .post(&api_url)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_API_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Anthropic API request failed: {}", e))?;

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
                // Collect useful response headers for debugging
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
                logger.log("debug", "agent", "agent::chat_anthropic::response",
                    &format!("Anthropic API response: status={}, request_id={}, ttfb={}ms", status, request_id, ttfb_ms),
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
                // Log API error
                if let Some(logger) = crate::get_logger() {
                    let error_preview = if error_text.len() > 500 { format!("{}...", crate::truncate_utf8(&error_text, 500)) } else { error_text.clone() };
                    logger.log("error", "agent", "agent::chat_anthropic::error",
                        &format!("Anthropic API error ({}): {}", status, error_preview),
                        Some(json!({"status": status, "error": error_text, "round": round}).to_string()),
                        None, None);
                }
                return Err(anyhow::anyhow!("Anthropic API error ({}): {}", status, error_text));
            }

            // Parse SSE stream
            let (text, tool_calls, stop_reason, round_usage, thinking) = self.parse_anthropic_sse(resp, cancel, on_delta).await?;
            collected_text.push_str(&text);
            collected_thinking.push_str(&thinking);
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

            // Log tool loop progress
            if let Some(logger) = crate::get_logger() {
                let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                logger.log("info", "agent", "agent::chat_anthropic::tool_loop",
                    &format!("Tool loop round {}: executing {} tools: {:?}", round, tool_calls.len(), tool_names),
                    Some(json!({
                        "round": round,
                        "tool_count": tool_calls.len(),
                        "tools": tool_names,
                    }).to_string()),
                    None, None);
            }

            // Execute tools and build tool_result messages
            let mut tool_results: Vec<serde_json::Value> = Vec::new();
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

                tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tc.call_id,
                    "content": build_anthropic_tool_result_content(&clean_result),
                }));
            }
            messages.push(json!({ "role": "user", "content": tool_results }));

            // Tier 1 quick check: truncate any oversized tool results added this round
            crate::context_compact::truncate_tool_results(&mut messages, self.context_window, &self.compact_config);
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
        emit_usage(on_delta, &total_usage, model);

        // Log chat completion summary
        if let Some(logger) = crate::get_logger() {
            let history_len = self.conversation_history.lock().unwrap().len();
            logger.log("info", "agent", "agent::chat_anthropic::done",
                &format!("Anthropic chat complete: {}chars, {} rounds, usage in={}/out={}",
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

    /// Parse Anthropic SSE stream. Returns (collected_text, tool_calls, stop_reason, usage, thinking)
    async fn parse_anthropic_sse(
        &self,
        resp: reqwest::Response,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Vec<FunctionCallItem>, Option<String>, ChatUsage, String)> {
        use futures_util::StreamExt;

        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
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
                                            collected_thinking.push_str(text);
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

        // Log SSE stream completion
        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            logger.log("debug", "agent", "agent::parse_anthropic_sse::done",
                &format!("Anthropic SSE done: {}chars text, {} tool_calls, stop={:?}",
                    collected_text.len(), tool_calls.len(), stop_reason),
                Some(json!({
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
                }).to_string()),
                None, None);
        }

        Ok((collected_text, tool_calls, stop_reason, usage, collected_thinking))
    }
}
