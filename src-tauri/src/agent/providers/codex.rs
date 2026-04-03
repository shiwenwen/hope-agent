use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use futures_util::future::join_all;

use super::super::api_types::{ReasoningConfig, ResponsesRequest};
use super::super::config::{
    clamp_reasoning_effort, get_max_tool_rounds, BASE_DELAY_MS, CODEX_API_URL, MAX_RETRIES,
};
use super::super::content::build_user_content_responses;
use super::super::errors::{is_retryable_error, os_version, parse_error_response};
use super::super::events::{
    build_responses_tool_result, emit_tool_call, emit_tool_result, emit_usage, extract_media_urls,
};
use super::super::types::{AssistantAgent, Attachment, ChatUsage};
use super::tool_exec_helpers::{execute_tool_with_cancel, log_tool_input, log_tool_output};
use crate::tools::{self, ToolProvider};

impl AssistantAgent {
    // ── OpenAI Codex Responses API with Tool Loop ─────────────────

    pub(crate) async fn chat_openai(
        &self,
        access_token: &str,
        account_id: &str,
        model: &str,
        message: &str,
        attachments: &[Attachment],
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<(String, Option<String>)> {
        let client = reqwest::Client::new();
        let mut tool_schemas = tools::get_tools_for_provider(ToolProvider::OpenAI);
        if self.web_search_enabled {
            tool_schemas
                .push(tools::get_web_search_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        if self.notification_enabled {
            tool_schemas
                .push(tools::get_notification_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        if let Some(ref img_config) = self.image_gen_config {
            tool_schemas.push(
                tools::get_image_generate_tool_dynamic(img_config)
                    .to_provider_schema(ToolProvider::OpenAI),
            );
        }
        if self.canvas_enabled {
            tool_schemas.push(tools::get_canvas_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        if self.subagent_tool_enabled() {
            tool_schemas.push(tools::get_subagent_tool().to_provider_schema(ToolProvider::OpenAI));
        }
        // Plan Agent / Build Agent tool injection
        self.apply_plan_tools(&mut tool_schemas, ToolProvider::OpenAI);
        // Filter out denied tools (depth-based tool policy)
        if !self.denied_tools.is_empty() {
            tool_schemas.retain(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                !self.denied_tools.contains(&name.to_string())
            });
        }

        // Build reasoning config with clamping
        let reasoning = reasoning_effort
            .and_then(|e| clamp_reasoning_effort(model, e))
            .map(|effort| ReasoningConfig {
                effort,
                summary: Some("auto".to_string()),
            });

        // Build input from conversation history + new user message (with optional image attachments)
        // Normalize history in case previous turns were from a different provider (failover / model switch)
        let mut input =
            Self::normalize_history_for_responses(&self.conversation_history.lock().unwrap_or_else(|e| e.into_inner()));
        let user_content = build_user_content_responses(message, attachments);
        Self::push_user_message(&mut input, user_content);

        let user_agent = format!(
            "OpenComputer ({} {}; {})",
            std::env::consts::OS,
            os_version(),
            std::env::consts::ARCH,
        );

        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
        let mut total_usage = ChatUsage::default();
        let mut first_ttft_ms: Option<u64> = None;
        let system_prompt = self.build_full_system_prompt(model, "Codex");

        // Run context compaction (Tier 1-3) before API call
        self.run_compaction(&mut input, &system_prompt, 16384, on_delta)
            .await;

        let max_rounds = get_max_tool_rounds();
        let max_rounds = if max_rounds == 0 {
            u32::MAX
        } else {
            max_rounds
        };
        let mut round_count: u32 = 0;
        for round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            round_count = round + 1;

            // Drain steer mailbox: inject any pending steer messages as user messages
            if let Some(ref rid) = self.steer_run_id {
                for msg in crate::subagent::SUBAGENT_MAILBOX.drain(rid) {
                    Self::push_user_message(
                        &mut input,
                        serde_json::json!(format!("[Steer from parent agent]: {}", msg)),
                    );
                }
            }

            let request = ResponsesRequest {
                model: model.to_string(),
                store: false,
                stream: true,
                instructions: system_prompt.clone(),
                input: input.clone(),
                reasoning: reasoning.as_ref().map(|r| ReasoningConfig {
                    effort: r.effort.clone(),
                    summary: Some("auto".to_string()),
                }),
                include: if reasoning.is_some() {
                    Some(vec!["reasoning.encrypted_content".to_string()])
                } else {
                    None
                },
                tools: Some(tool_schemas.clone()),
                temperature: self.temperature,
            };

            let body_json = serde_json::to_string(&request)?;

            // Log API request details (including raw body for debugging)
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
                        round,
                        input.len(),
                        tool_schemas.len(),
                        body_size
                    ),
                    Some(
                        json!({
                            "round": round,
                            "api_url": CODEX_API_URL,
                            "model": model,
                            "input_count": input.len(),
                            "tool_count": tool_schemas.len(),
                            "body_size_bytes": body_size,
                            "reasoning": reasoning.as_ref().map(|r| r.effort.as_str()),
                            "request_body": raw_body,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }

            // Retry loop with exponential backoff
            let mut last_error: Option<String> = None;
            let mut resp_opt: Option<reqwest::Response> = None;

            let request_start = std::time::Instant::now();
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
                            // Log successful response with headers
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
                                        "round": round,
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
                                    json!({"status": status, "error": error_text, "round": round})
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

            // Parse SSE stream
            let (text, tool_calls, round_usage, thinking, round_ttft, round_reasoning_items) = self
                .parse_openai_sse(resp, request_start, cancel, on_delta)
                .await?;
            if first_ttft_ms.is_none() {
                first_ttft_ms = round_ttft;
            }
            collected_text.push_str(&text);
            collected_thinking.push_str(&thinking);
            total_usage.input_tokens += round_usage.input_tokens;
            total_usage.output_tokens += round_usage.output_tokens;
            total_usage.cache_creation_input_tokens += round_usage.cache_creation_input_tokens;
            total_usage.cache_read_input_tokens += round_usage.cache_read_input_tokens;

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                // Last round: save reasoning items for final history
                for ri in &round_reasoning_items {
                    input.push(ri.clone());
                }
                break;
            }

            // Push reasoning items from this round (before function_call items)
            for ri in &round_reasoning_items {
                input.push(ri.clone());
            }

            // Log tool loop progress
            if let Some(logger) = crate::get_logger() {
                let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_codex::tool_loop",
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

            // Estimate current token usage for adaptive tool output sizing
            let estimated_used = crate::context_compact::estimate_request_tokens(
                &system_prompt,
                &input,
                16384,
            );

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

                for (call_id, name, arguments, result, elapsed_ms) in results {
                    log_tool_output(&call_id, &name, &result, elapsed_ms, round);
                    let is_tool_error = result.starts_with("Tool error:");
                    let (clean_result, media_urls) = extract_media_urls(&result);
                    emit_tool_result(
                        on_delta, &call_id, &name, &clean_result, elapsed_ms,
                        is_tool_error, &media_urls,
                    );

                    let (text_output, image_items) = build_responses_tool_result(&clean_result);

                    input.push(json!({
                        "type": "function_call",
                        "id": call_id,
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                    }));
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": text_output,
                    }));
                    for img_item in image_items {
                        input.push(img_item);
                    }
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
                let (clean_result, media_urls) = extract_media_urls(&result);
                emit_tool_result(
                    on_delta, &tc.call_id, &tc.name, &clean_result, tool_elapsed_ms,
                    is_tool_error, &media_urls,
                );

                let (text_output, image_items) = build_responses_tool_result(&clean_result);

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
                    "output": text_output,
                }));
                for img_item in image_items {
                    input.push(img_item);
                }
            }

            // Tier 1 quick check: truncate any oversized tool results added this round
            crate::context_compact::truncate_tool_results(
                &mut input,
                self.context_window,
                &self.compact_config,
            );
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!("No content received from Codex API"));
        }

        // Persist conversation history with proper Responses API format
        if !collected_text.is_empty() {
            input.push(json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": collected_text }],
                "status": "completed"
            }));
        }
        *self.conversation_history.lock().unwrap_or_else(|e| e.into_inner()) = input;

        // Emit accumulated usage (with TTFT)
        emit_usage(on_delta, &total_usage, model, first_ttft_ms);

        // Log chat completion summary
        if let Some(logger) = crate::get_logger() {
            let history_len = self.conversation_history.lock().unwrap_or_else(|e| e.into_inner()).len();
            logger.log(
                "info",
                "agent",
                "agent::chat_codex::done",
                &format!(
                    "Codex chat complete: {}chars, {} rounds, usage in={}/out={}",
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
}
