use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use futures_util::future::join_all;

use super::super::api_types::FunctionCallItem;
use super::super::config::{
    apply_thinking_to_chat_body, build_api_url, get_max_tool_rounds, live_reasoning_effort,
};
use super::super::content::build_user_content_openai_chat;
use super::super::events::{
    build_openai_chat_tool_result_content, emit_max_rounds_notice, emit_text_delta,
    emit_thinking_delta, emit_tool_call, emit_tool_result, emit_usage, extract_media_items,
};
use super::super::types::{AssistantAgent, Attachment, ChatUsage, ThinkTagFilter};
use super::tool_exec_helpers::{execute_tool_with_cancel, log_tool_input, log_tool_output};
use crate::tools::{self, ToolProvider};

impl AssistantAgent {
    // ── OpenAI Chat Completions API with Tool Loop ───────────────

    pub(crate) async fn chat_openai_chat(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        message: &str,
        attachments: &[Attachment],
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Option<String>)> {
        self.reset_chat_flags();
        self.refresh_awareness_suffix(message).await;
        self.refresh_active_memory_suffix(message).await;

        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;
        let tool_schemas = self.build_tool_schemas(ToolProvider::OpenAI);

        // Normalize history in case previous turns were from a different provider (failover / model switch)
        let mut messages = Self::normalize_history_for_chat(
            &self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let user_content = build_user_content_openai_chat(message, attachments);
        Self::push_user_message(&mut messages, user_content);

        let api_url = build_api_url(base_url, "/v1/chat/completions");
        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
        let mut last_round_thinking = String::new();
        let mut total_usage = ChatUsage::default();
        let mut first_ttft_ms: Option<u64> = None;
        let system_prompt = self.build_full_system_prompt(model, "OpenAIChat");
        let system_prompt_for_budget =
            self.build_merged_system_prompt(model, "OpenAIChat");

        // Run context compaction (Tier 1-3) before API call
        self.run_compaction(&mut messages, &system_prompt_for_budget, 16384, on_delta)
            .await;

        // LLM memory selection: filter to most relevant memories
        let mut system_prompt = system_prompt;
        self.select_memories_if_needed(&mut system_prompt, message)
            .await;

        // Context engine hook: optional system prompt addition (e.g. Active Memory)
        self.apply_engine_prompt_addition(&mut system_prompt);

        // Save cache-safe params for side_query reuse (prompt cache sharing)
        self.save_cache_safe_params(
            system_prompt.clone(),
            tool_schemas.clone(),
            messages.clone(),
        );

        // Apply thinking parameters based on ThinkingStyle

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 {
            u32::MAX
        } else {
            max_rounds
        };
        let mut round_count: u32 = 0;
        let mut natural_exit = false;
        for round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            round_count = round + 1;

            if let Some(ref sid) = self.session_id {
                crate::awareness::touch_active_session(sid);
            }

            // Refresh effort per round when following the global picker so UI
            // toggles apply on the very next request instead of waiting for
            // the next user turn. Stored as an owned String to satisfy the
            // Option<&str> signature of downstream helpers.
            let live_effort_owned: Option<String> = if self.follow_global_reasoning_effort {
                live_reasoning_effort(reasoning_effort).await
            } else {
                reasoning_effort.map(|s| s.to_string())
            };
            let effective_effort = live_effort_owned.as_deref();

            // Drain steer mailbox: inject any pending steer messages as user messages
            if let Some(ref rid) = self.steer_run_id {
                for msg in crate::subagent::SUBAGENT_MAILBOX.drain(rid) {
                    Self::push_user_message(
                        &mut messages,
                        serde_json::json!(format!("[Steer from parent agent]: {}", msg)),
                    );
                }
            }

            // Build messages array: system + conversation (strip _oc_round metadata).
            // OpenAI Chat supports multiple system messages; send the static prefix
            // as one and the dynamic awareness suffix as a second so OpenAI's
            // automatic prefix caching can still hit the first one when the suffix
            // changes between turns.
            let mut api_messages = vec![json!({ "role": "system", "content": &system_prompt })];
            if let Some(suffix) = self.current_awareness_suffix() {
                if !suffix.is_empty() {
                    api_messages.push(json!({ "role": "system", "content": suffix.as_str() }));
                }
            }
            // Active Memory (Phase B1) — third system message so automatic
            // prefix caching still hits the earlier two when the recall
            // sentence changes between turns.
            if let Some(active_suffix) = self.current_active_memory_suffix() {
                if !active_suffix.is_empty() {
                    api_messages
                        .push(json!({ "role": "system", "content": active_suffix.as_str() }));
                }
            }
            api_messages.extend(crate::context_compact::prepare_messages_for_api(&messages));

            // Build tools array in Chat Completions format
            let tools_array: Vec<serde_json::Value> = tool_schemas
                .iter()
                .map(|t| json!({ "type": "function", "function": t }))
                .collect();

            // On the final allowed round omit `tools` so the model is forced to
            // produce a text response. Without this, a tool call here would
            // execute and append tool_results to history that the model never
            // gets to see, leaving the user with only a "max rounds" notice.
            let is_final_round = round + 1 == max_rounds;

            let mut body = json!({
                "model": model,
                "messages": api_messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });
            if !is_final_round {
                body["tools"] = json!(tools_array);
            }

            // Apply thinking parameters based on provider's ThinkingStyle
            apply_thinking_to_chat_body(&mut body, &self.thinking_style, effective_effort, 16384);

            // Add temperature if configured
            if let Some(temp) = self.temperature {
                body["temperature"] = json!(temp);
            }

            // Log API request details (including raw body for debugging)
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
                        round,
                        api_messages.len(),
                        tools_array.len(),
                        body_size
                    ),
                    Some(
                        json!({
                            "round": round,
                            "api_url": &api_url,
                            "model": model,
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
                            "round": round,
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
                            json!({"status": status, "error": error_text, "round": round})
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

            // Parse SSE stream for Chat Completions format
            let (text, tool_calls, round_usage, thinking, round_ttft) = self
                .parse_chat_completions_sse(resp, request_start, effective_effort, cancel, on_delta)
                .await?;
            if first_ttft_ms.is_none() {
                first_ttft_ms = round_ttft;
            }
            collected_text.push_str(&text);
            collected_thinking.push_str(&thinking);
            last_round_thinking = thinking.clone();
            total_usage.accumulate_round(&round_usage);

            if tool_calls.is_empty() {
                natural_exit = true;
                break;
            }

            // Log tool loop progress
            if let Some(logger) = crate::get_logger() {
                let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_openai_chat::tool_loop",
                    &format!(
                        "Tool loop round {}: executing {} tools: {:?}",
                        round,
                        tool_calls.len(),
                        tool_names
                    ),
                    Some(
                        json!({
                            "round": round,
                            "tool_count": tool_calls.len(),
                            "tools": tool_names,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }

            // Build assistant message with tool_calls
            let tc_json: Vec<serde_json::Value> = tool_calls
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
            if !text.is_empty() {
                assistant_msg["content"] = json!(text);
            }
            if !thinking.is_empty() {
                assistant_msg["reasoning_content"] = json!(thinking);
            }
            assistant_msg["tool_calls"] = json!(tc_json);
            crate::context_compact::push_and_stamp(&mut messages, assistant_msg, round);

            // Estimate current token usage for adaptive tool output sizing
            let estimated_used =
                crate::context_compact::estimate_request_tokens(&system_prompt, &messages, 16384);

            // Execute tools with concurrent-safe tools in parallel, sequential tools in order.
            // Partition tool calls into concurrent-safe and sequential groups.
            let (concurrent_tcs, sequential_tcs): (Vec<_>, Vec<_>) = tool_calls
                .iter()
                .partition(|tc| tools::is_concurrent_safe(&tc.name));

            let tool_ctx = self.tool_context_with_usage(Some(estimated_used));

            // Phase 1: Execute concurrent-safe tools in parallel
            if !concurrent_tcs.is_empty() && !cancel.load(Ordering::SeqCst) {
                let concurrent_futures: Vec<_> = concurrent_tcs
                    .iter()
                    .map(|tc| {
                        let cancel_clone = cancel.clone();
                        let tool_ctx = tool_ctx.clone();
                        let call_id = tc.call_id.clone();
                        let name = tc.name.clone();
                        let arguments = tc.arguments.clone();
                        async move {
                            let args: serde_json::Value =
                                serde_json::from_str(&arguments).unwrap_or(json!({}));
                            let (result, elapsed_ms) =
                                execute_tool_with_cancel(&name, &args, &tool_ctx, &cancel_clone)
                                    .await;
                            (call_id, name, arguments, result, elapsed_ms)
                        }
                    })
                    .collect();

                // Emit all tool_call events before parallel execution
                for tc in &concurrent_tcs {
                    emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);
                    log_tool_input(tc, round);
                }

                let results = join_all(concurrent_futures).await;

                for (call_id, name, _arguments, result, elapsed_ms) in results {
                    log_tool_output(&call_id, &name, &result, elapsed_ms, round);
                    let is_tool_error = result.starts_with("Tool error:");
                    let (clean_result, media_items) = extract_media_items(&result);
                    emit_tool_result(
                        on_delta,
                        &call_id,
                        &name,
                        &clean_result,
                        elapsed_ms,
                        is_tool_error,
                        &media_items,
                    );
                    crate::context_compact::push_and_stamp(
                        &mut messages,
                        json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": build_openai_chat_tool_result_content(&clean_result),
                        }),
                        round,
                    );
                }
            }

            // Phase 2: Execute sequential tools one by one
            for tc in &sequential_tcs {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }

                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);
                log_tool_input(tc, round);

                let (result, tool_elapsed_ms) =
                    execute_tool_with_cancel(&tc.name, &args, &tool_ctx, cancel).await;

                log_tool_output(&tc.call_id, &tc.name, &result, tool_elapsed_ms, round);
                let is_tool_error = result.starts_with("Tool error:");
                let (clean_result, media_items) = extract_media_items(&result);
                emit_tool_result(
                    on_delta,
                    &tc.call_id,
                    &tc.name,
                    &clean_result,
                    tool_elapsed_ms,
                    is_tool_error,
                    &media_items,
                );

                crate::context_compact::push_and_stamp(
                    &mut messages,
                    json!({
                        "role": "tool",
                        "tool_call_id": tc.call_id,
                        "content": build_openai_chat_tool_result_content(&clean_result),
                    }),
                    round,
                );
            }

            // Track manual memory writes for extraction mutual exclusion
            self.check_manual_memory_save(&tool_calls);

            // Tier 1 quick check: truncate any oversized tool results added this round
            crate::context_compact::truncate_tool_results(
                &mut messages,
                self.context_window,
                &self.compact_config,
            );

            // Reactive microcompact: when usage crosses the threshold mid-loop,
            // clear ephemeral tool_results (Tier 0) to head off emergency compaction.
            self.reactive_microcompact_in_loop(&mut messages, &system_prompt_for_budget, 16384);
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        let rounds_exhausted = !natural_exit && !cancelled && round_count == max_rounds;
        if rounds_exhausted {
            let notice = emit_max_rounds_notice(on_delta, max_rounds);
            collected_text.push_str(&notice);
        }
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from OpenAI Chat API"));
        }

        if !collected_text.is_empty() {
            let mut final_msg = json!({ "role": "assistant", "content": collected_text });
            if !last_round_thinking.is_empty() {
                final_msg["reasoning_content"] = json!(last_round_thinking);
            }
            messages.push(final_msg);
        }
        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages;

        // Emit accumulated usage (with TTFT)
        emit_usage(on_delta, &total_usage, model, first_ttft_ms);

        // Log chat completion summary
        if let Some(logger) = crate::get_logger() {
            let history_len = self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            logger.log(
                "info",
                "agent",
                "agent::chat_openai_chat::done",
                &format!(
                    "OpenAI Chat complete: {}chars, {} rounds, usage in={}/out={}",
                    collected_text.len(),
                    round_count,
                    total_usage.input_tokens,
                    total_usage.output_tokens
                ),
                Some(
                    json!({
                        "text_length": collected_text.len(),
                        "total_rounds": round_count,
                        "history_length": history_len,
                        "cancelled": cancelled,
                        "rounds_exhausted": rounds_exhausted,
                        "model": model,
                        "usage": {
                            "input_tokens": total_usage.input_tokens,
                            "output_tokens": total_usage.output_tokens,
                            "cache_creation": total_usage.cache_creation_input_tokens,
                            "cache_read": total_usage.cache_read_input_tokens,
                        }
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let thinking_result = if collected_thinking.is_empty() {
            None
        } else {
            Some(collected_thinking)
        };
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
        // Track tool calls by index
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

                    if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                        // Parse usage from stream (when stream_options.include_usage is set)
                        if let Some(u) = chunk.get("usage") {
                            if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                                usage.input_tokens = pt;
                            }
                            if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                                usage.output_tokens = ct;
                            }
                            // Anthropic-style at top level (OpenRouter / LiteLLM gateways
                            // can forward Anthropic-shape fields through Chat Completions).
                            if let Some(cr) = u
                                .get("cache_read_input_tokens")
                                .and_then(|v| v.as_u64())
                            {
                                usage.cache_read_input_tokens = cr;
                            }
                            if let Some(cc) = u
                                .get("cache_creation_input_tokens")
                                .and_then(|v| v.as_u64())
                            {
                                usage.cache_creation_input_tokens = cc;
                            }
                            // Fallback: OpenAI prompt_tokens_details.cached_tokens /
                            // Moonshot top-level cached_tokens.
                            if usage.cache_read_input_tokens == 0 {
                                usage.cache_read_input_tokens = u
                                    .get("prompt_tokens_details")
                                    .and_then(|d| d.get("cached_tokens"))
                                    .and_then(|v| v.as_u64())
                                    .or_else(|| {
                                        u.get("cached_tokens").and_then(|v| v.as_u64())
                                    })
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
                                        emit_thinking_delta(on_delta, reasoning);
                                        collected_thinking.push_str(reasoning);
                                    }
                                }

                                // Text content — filter <think>...</think> tags from content stream
                                // Qwen models may embed thinking in content via <think> tags
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str())
                                {
                                    let (text_part, think_part) = think_filter.process(content);
                                    // When thinking is enabled, redirect <think> content as thinking_delta;
                                    // when disabled ("none"), discard it entirely
                                    if !think_part.is_empty() && reasoning_effort != Some("none") {
                                        emit_thinking_delta(on_delta, &think_part);
                                        collected_thinking.push_str(&think_part);
                                    }
                                    if !text_part.is_empty() {
                                        if first_token_time.is_none() {
                                            first_token_time =
                                                Some(request_start.elapsed().as_millis() as u64);
                                        }
                                        emit_text_delta(on_delta, &text_part);
                                        collected_text.push_str(&text_part);
                                    }
                                }

                                // Tool calls
                                if let Some(tcs) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
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
}
