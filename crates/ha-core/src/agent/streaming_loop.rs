//! Provider-agnostic streaming chat orchestration.
//!
//! [`AssistantAgent::run_streaming_chat`] runs the full tool loop using a
//! [`StreamingChatAdapter`] for provider-specific concerns (body / SSE /
//! history). All compaction, tool dispatch, microcompact, steer mailbox
//! drain, and event emission live here — provider files become thin
//! adapters owning only body construction + SSE decoding + history shape.
//!
//! See [`super::streaming_adapter`] for the trait that adapters implement.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use futures_util::future::join_all;
use serde_json::json;

use super::api_types::FunctionCallItem;
use super::events::{
    emit_max_rounds_notice, emit_tool_call, emit_tool_result, emit_usage, extract_media_items,
};
use super::streaming_adapter::{ExecutedTool, RoundRequest, StreamingChatAdapter};
use super::types::{AssistantAgent, ChatUsage};
use crate::tools::{self, ToolExecContext};

/// All four providers share the same max_tokens budget: it caps Anthropic's
/// `max_tokens` request field and feeds the compaction / microcompact token
/// budget estimator. OpenAI Chat / Responses / Codex don't put this in the
/// request body — only in the budget calculator.
const MAX_OUTPUT_TOKENS: u32 = 16384;

fn final_round_handoff_guidance(max_rounds: u32) -> String {
    format!(
        "# Tool-Call Limit Reached\n\n\
         This is the final allowed response for this user turn: the tool-call \
         limit of {} rounds has been reached and tools are now unavailable. \
         Tell the user this limit was reached, summarize what is done, list \
         what remains, and ask them to send a new message such as \"继续\" \
         if they want you to continue. Do not claim the whole task is complete \
         unless every required item has actually been verified.",
        max_rounds
    )
}

// ── Tool execution helpers (private to streaming_loop, no other caller).

/// Log tool execution input.
fn log_tool_input(tc: &FunctionCallItem, round: u32) {
    if let Some(logger) = crate::get_logger() {
        let args_str = tc.arguments.as_str();
        let args_preview = if args_str.len() > 2048 {
            format!(
                "{}...(truncated, total {}B)",
                crate::truncate_utf8(args_str, 2048),
                args_str.len()
            )
        } else {
            args_str.to_string()
        };
        logger.log(
            "debug",
            "agent",
            "agent::tool_exec::input",
            &format!("Tool exec [{}] id={}", tc.name, tc.call_id),
            Some(
                json!({
                    "tool_name": tc.name,
                    "call_id": tc.call_id,
                    "arguments": args_preview,
                    "round": round,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

/// Log tool execution output.
fn log_tool_output(call_id: &str, name: &str, result: &str, elapsed_ms: u64, round: u32) {
    if let Some(logger) = crate::get_logger() {
        let result_preview = if result.len() > 2048 {
            format!(
                "{}...(truncated, total {}B)",
                crate::truncate_utf8(result, 2048),
                result.len()
            )
        } else {
            result.to_string()
        };
        let is_error = result.starts_with("Tool error:");
        logger.log(
            if is_error { "warn" } else { "debug" },
            "agent",
            "agent::tool_exec::output",
            &format!(
                "Tool result [{}] {}B, {}ms{}",
                name,
                result.len(),
                elapsed_ms,
                if is_error { " (ERROR)" } else { "" }
            ),
            Some(
                json!({
                    "tool_name": name,
                    "call_id": call_id,
                    "result_size_bytes": result.len(),
                    "elapsed_ms": elapsed_ms,
                    "is_error": is_error,
                    "result_preview": result_preview,
                    "round": round,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

/// Execute a tool with cancel-flag racing. Returns `(result_string, elapsed_ms)`.
async fn execute_tool_with_cancel(
    name: &str,
    args: &serde_json::Value,
    ctx: &ToolExecContext,
    cancel: &Arc<AtomicBool>,
) -> (String, u64) {
    let tool_start = std::time::Instant::now();
    let cancel_clone = cancel.clone();
    let result = tokio::select! {
        res = tools::execute_tool_with_context(name, args, ctx) => {
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
    let elapsed_ms = tool_start.elapsed().as_millis() as u64;
    (result, elapsed_ms)
}

impl AssistantAgent {
    /// Provider-agnostic streaming chat with tool loop.
    ///
    /// All four `chat_<provider>` entry points delegate here, passing a
    /// provider-specific [`StreamingChatAdapter`] and a pre-built user-content
    /// `Value` (because content shape differs per provider).
    ///
    /// The orchestrator owns:
    ///   - reset_chat_flags / refresh_awareness / refresh_active_memory
    ///   - tool schema build + history normalize + push_user_message
    ///   - system prompt build + compaction + memory selection + cache snapshot
    ///   - per-round: cancel check, touch_active_session, drain steer mailbox,
    ///     prepare_messages_for_api, dispatch tools (concurrent + sequential),
    ///     manual_memory_save check, truncate_tool_results, reactive_microcompact
    ///   - max-rounds notice, final assistant persist, emit_usage
    ///
    /// The adapter owns: normalize_history, chat_round (body+SSE),
    /// append_round_to_history, append_final_assistant, loop_should_exit.
    pub(crate) async fn run_streaming_chat<F>(
        &self,
        adapter: &dyn StreamingChatAdapter,
        model: &str,
        message: &str,
        user_content_for_history: serde_json::Value,
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &F,
    ) -> Result<(String, Option<String>)>
    where
        F: Fn(&str) + Send + Sync,
    {
        let provider_label = adapter.provider_format().label();

        self.reset_chat_flags();
        self.refresh_awareness_suffix(message).await;
        self.refresh_active_memory_suffix(message).await;

        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        let tool_schemas = self.build_tool_schemas(adapter.tool_provider());

        // Normalize prior history (it may have been persisted from a different
        // provider during failover / model switch). Then append the new user
        // message via push_user_message (handles consecutive-user merging for
        // Anthropic role-alternation requirement).
        let mut messages = {
            let history_guard = self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut h = history_guard.clone();
            drop(history_guard);
            adapter.normalize_history(&mut h);
            h
        };
        Self::push_user_message(&mut messages, user_content_for_history);

        // Static system prompt prefix (cache-friendly). The dynamic awareness
        // and active-memory suffixes go in their own cache breakpoints inside
        // chat_round (each adapter handles the placement).
        let system_prompt = self.build_full_system_prompt(model, provider_label);
        let system_prompt_for_budget = self.build_merged_system_prompt(model, provider_label);

        self.run_compaction(
            &mut messages,
            &system_prompt_for_budget,
            MAX_OUTPUT_TOKENS,
            on_delta,
        )
        .await;

        let mut system_prompt = system_prompt;
        self.select_memories_if_needed(&mut system_prompt, message)
            .await;
        self.apply_engine_prompt_addition(&mut system_prompt);

        // Snapshot cache-safe params for side_query reuse (prompt cache sharing).
        // Must run AFTER compaction + memory selection so the snapshot matches
        // what the next API request actually sends.
        self.save_cache_safe_params(
            system_prompt.clone(),
            tool_schemas.clone(),
            messages.clone(),
        );

        let max_rounds_cfg = super::config::get_max_tool_rounds();
        let max_rounds = if max_rounds_cfg == 0 {
            u32::MAX
        } else {
            max_rounds_cfg
        };
        let round_limit_enabled = max_rounds_cfg != 0;
        let mut round_count: u32 = 0;
        let mut natural_exit = false;
        let mut collected_text = String::new();
        let mut collected_thinking = String::new();
        let mut last_round_thinking = String::new();
        let mut total_usage = ChatUsage::default();
        let mut first_ttft_ms: Option<u64> = None;

        // Coerce the generic `&F` to a `&dyn` once for trait method calls.
        // Generic emit_* helpers continue to use `on_delta` directly (zero
        // dispatch overhead in the hot SSE path).
        let on_delta_dyn: &(dyn Fn(&str) + Send + Sync) = on_delta;

        for round in 0..max_rounds {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            round_count = round + 1;

            // Keep this session marked as "active" during long tool loops
            // so peer sessions see it in the registry.
            if let Some(ref sid) = self.session_id {
                crate::awareness::touch_active_session(sid);
            }

            // Drain steer mailbox: inject any pending steer messages as user msgs.
            if let Some(ref rid) = self.steer_run_id {
                for msg in crate::subagent::SUBAGENT_MAILBOX.drain(rid) {
                    Self::push_user_message(
                        &mut messages,
                        json!(format!("[Steer from parent agent]: {}", msg)),
                    );
                }
            }

            let is_final_round = round + 1 == max_rounds;
            let final_round_system_prompt;
            let round_system_prompt = if round_limit_enabled && is_final_round {
                final_round_system_prompt = format!(
                    "{}\n\n{}",
                    system_prompt,
                    final_round_handoff_guidance(max_rounds)
                );
                final_round_system_prompt.as_str()
            } else {
                system_prompt.as_str()
            };
            let api_messages = crate::context_compact::prepare_messages_for_api(&messages);
            let effort_live = self.effective_reasoning_effort(reasoning_effort).await;
            let awareness_suffix = self.current_awareness_suffix();
            let active_suffix = self.current_active_memory_suffix();

            let req = RoundRequest {
                system_prompt: round_system_prompt,
                awareness_suffix: awareness_suffix.as_deref().map(|s| s.as_str()),
                active_memory_suffix: active_suffix.as_deref().map(|s| s.as_str()),
                tool_schemas: &tool_schemas,
                history_for_api: &api_messages,
                reasoning_effort: effort_live.as_deref(),
                temperature: self.temperature,
                max_tokens: MAX_OUTPUT_TOKENS,
                is_final_round,
                round,
            };

            let outcome = adapter
                .chat_round(&client, req, cancel, on_delta_dyn)
                .await?;

            if first_ttft_ms.is_none() {
                first_ttft_ms = outcome.ttft_ms;
            }
            collected_text.push_str(&outcome.text);
            collected_thinking.push_str(&outcome.thinking);
            last_round_thinking = outcome.thinking.clone();
            total_usage.accumulate_round(&outcome.usage);

            // Per-round reasoning items (Responses/Codex only) must be persisted
            // before either tool dispatch or the natural-exit final message so
            // the next turn can reference the encrypted_content. No-op for
            // Anthropic / OpenAI Chat (their thinking lives inline in messages).
            adapter.append_reasoning_items(&mut messages, &outcome);

            if adapter.loop_should_exit(&outcome) {
                natural_exit = true;
                break;
            }

            // Estimate current token usage for adaptive tool output sizing.
            let estimated_used = crate::context_compact::estimate_request_tokens(
                &system_prompt,
                &messages,
                MAX_OUTPUT_TOKENS,
            );

            // Partition tool calls by concurrent-safety:
            //   Phase 1: parallel concurrent-safe tools (read-only)
            //   Phase 2: sequential write/exec tools
            let (concurrent_tcs, sequential_tcs): (Vec<_>, Vec<_>) = outcome
                .tool_calls
                .iter()
                .partition(|tc| tools::is_concurrent_safe(&tc.name));

            let mut executed: Vec<ExecutedTool> = Vec::new();
            let tool_ctx = self.tool_context_with_usage(Some(estimated_used));

            // Phase 1: concurrent-safe in parallel.
            if !concurrent_tcs.is_empty() && !cancel.load(Ordering::SeqCst) {
                let futures: Vec<_> = concurrent_tcs
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

                // Emit all tool_call events before parallel execution starts so
                // the UI shows the in-flight set immediately.
                for tc in &concurrent_tcs {
                    emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);
                    log_tool_input(tc, round);
                }

                let results = join_all(futures).await;

                for (call_id, name, arguments, result, elapsed_ms) in results {
                    log_tool_output(&call_id, &name, &result, elapsed_ms, round);
                    let is_error = result.starts_with("Tool error:");
                    let (clean_result, media_items) = extract_media_items(&result);
                    emit_tool_result(
                        on_delta,
                        &call_id,
                        &name,
                        &clean_result,
                        elapsed_ms,
                        is_error,
                        &media_items,
                    );
                    executed.push(ExecutedTool {
                        call_id,
                        name,
                        arguments,
                        clean_result,
                    });
                }
            }

            // Phase 2: sequential write/exec tools.
            for tc in &sequential_tcs {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }

                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);
                log_tool_input(tc, round);

                let (result, elapsed_ms) =
                    execute_tool_with_cancel(&tc.name, &args, &tool_ctx, cancel).await;

                log_tool_output(&tc.call_id, &tc.name, &result, elapsed_ms, round);
                let is_error = result.starts_with("Tool error:");
                let (clean_result, media_items) = extract_media_items(&result);
                emit_tool_result(
                    on_delta,
                    &tc.call_id,
                    &tc.name,
                    &clean_result,
                    elapsed_ms,
                    is_error,
                    &media_items,
                );
                executed.push(ExecutedTool {
                    call_id: tc.call_id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    clean_result,
                });
            }

            // Adapter writes assistant + tool_results into history in its
            // native shape (Anthropic content blocks / OpenAI tool_calls /
            // Responses function_call+function_call_output items).
            adapter.append_round_to_history(&mut messages, round, &outcome, &executed);

            self.check_manual_memory_save(&outcome.tool_calls);

            // Tier 1 quick check: truncate any oversized tool results added
            // this round before the next request.
            crate::context_compact::truncate_tool_results(
                &mut messages,
                self.context_window,
                &self.compact_config,
            );

            // Reactive microcompact: when usage crosses the threshold mid-loop,
            // clear ephemeral tool_results (Tier 0) to head off emergency compaction.
            self.reactive_microcompact_in_loop(
                &mut messages,
                &system_prompt_for_budget,
                MAX_OUTPUT_TOKENS,
            );
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        let hit_round_limit = round_limit_enabled && !cancelled && round_count == max_rounds;
        let rounds_exhausted = hit_round_limit && !natural_exit;
        if hit_round_limit {
            let notice = emit_max_rounds_notice(on_delta, max_rounds);
            collected_text.push_str(&notice);
        }
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!(
                "No content received from {} API",
                provider_label
            ));
        }

        // Persist final assistant message in this provider's native shape.
        adapter.append_final_assistant(&mut messages, &collected_text, &last_round_thinking);

        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages;

        emit_usage(on_delta, &total_usage, model, first_ttft_ms);

        // Log chat completion summary.
        if let Some(logger) = crate::get_logger() {
            let history_len = self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            logger.log(
                "info",
                "agent",
                "agent::chat::done",
                &format!(
                    "{} chat complete: {}chars, {} rounds, usage in={}/out={}",
                    provider_label,
                    collected_text.len(),
                    round_count,
                    total_usage.input_tokens,
                    total_usage.output_tokens
                ),
                Some(
                    json!({
                        "provider": provider_label,
                        "text_length": collected_text.len(),
                        "total_rounds": round_count,
                        "hit_round_limit": hit_round_limit,
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
}
