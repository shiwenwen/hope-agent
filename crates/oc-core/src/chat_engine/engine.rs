use std::sync::Arc;

use crate::provider::ApiType;
use crate::session;
use crate::{context_compact, failover};

use super::context::*;
use super::types::*;

// ── Core Chat Engine ────────────────────────────────────────────────

/// Run the shared chat execution engine.
///
/// Handles: model chain traversal → agent building → config → history restoration
/// → streaming execution → tool persistence → failover → context compaction
/// → response saving → context persistence → memory extraction.
pub async fn run_chat_engine(params: ChatEngineParams) -> Result<ChatEngineResult, String> {
    let ChatEngineParams {
        session_id,
        agent_id,
        message,
        attachments,
        session_db: db,
        model_chain,
        providers,
        codex_token,
        resolved_temperature,
        web_search_enabled,
        notification_enabled,
        image_gen_config,
        canvas_enabled,
        compact_config,
        extra_system_context,
        reasoning_effort,
        cancel,
        plan_agent_mode,
        plan_mode_allow_paths,
        skill_allowed_tools,
        auto_approve_tools,
        event_sink,
    } = params;

    if model_chain.is_empty() {
        return Err("No model configured for chat execution".to_string());
    }

    let total_models = model_chain.len();
    let mut last_error: Option<String> = None;

    // Build primary model display name for fallback events
    let primary_display = {
        let first = &model_chain[0];
        let prov_name = providers
            .iter()
            .find(|p| p.id == first.provider_id)
            .map(|p| p.name.as_str())
            .unwrap_or(&first.provider_id);
        format!("{} / {}", prov_name, first.model_id)
    };

    let effort_str = reasoning_effort.clone();

    for (idx, model_ref) in model_chain.iter().enumerate() {
        let mut agent =
            match build_agent_from_snapshot(model_ref, &providers, &codex_token, &compact_config) {
                Some(a) => a,
                None => {
                    last_error = Some(format!(
                        "Cannot build agent for {}::{}",
                        model_ref.provider_id, model_ref.model_id
                    ));
                    continue;
                }
            };
        agent.set_agent_id(&agent_id);
        agent.set_session_id(&session_id);
        agent.set_web_search_enabled(web_search_enabled);
        agent.set_notification_enabled(notification_enabled);
        agent.set_image_generate_config(image_gen_config.clone());
        agent.set_canvas_enabled(canvas_enabled);
        agent.set_temperature(resolved_temperature);

        if let Some(ref ctx) = extra_system_context {
            agent.set_extra_system_context(ctx.clone());
        }
        if !skill_allowed_tools.is_empty() {
            agent.set_skill_allowed_tools(skill_allowed_tools.clone());
        }
        if let Some(ref mode) = plan_agent_mode {
            agent.set_plan_agent_mode(mode.clone());
        }
        if let Some(ref paths) = plan_mode_allow_paths {
            agent.set_plan_mode_allow_paths(paths.clone());
        }
        if auto_approve_tools {
            agent.set_auto_approve_tools(true);
        }

        // Restore conversation history from DB
        restore_agent_context(&db, &session_id, &agent);

        // Update session with current model info
        {
            let provider_name = providers
                .iter()
                .find(|p| p.id == model_ref.provider_id)
                .map(|p| p.name.as_str());
            let _ = db.update_session_model(
                &session_id,
                Some(&model_ref.provider_id),
                provider_name,
                Some(&model_ref.model_id),
            );
        }

        const MAX_RETRIES: u32 = 2;
        const RETRY_BASE_MS: u64 = 1000;
        const RETRY_MAX_MS: u64 = 10000;
        let mut retry_count: u32 = 0;

        loop {
            // Emit fallback event if this is not the first model
            if idx > 0 && retry_count == 0 {
                let display = {
                    let prov_name = providers
                        .iter()
                        .find(|p| p.id == model_ref.provider_id)
                        .map(|p| p.name.as_str())
                        .unwrap_or(&model_ref.provider_id);
                    format!("{} / {}", prov_name, model_ref.model_id)
                };
                let reason_str = last_error
                    .as_deref()
                    .map(|e| failover::classify_error(e))
                    .unwrap_or(failover::FailoverReason::Unknown);
                let event = serde_json::json!({
                    "type": "model_fallback",
                    "model": display,
                    "from_model": primary_display,
                    "provider_id": model_ref.provider_id,
                    "model_id": model_ref.model_id,
                    "reason": reason_str,
                    "attempt": idx + 1,
                    "total": total_models,
                    "error": last_error.as_deref().unwrap_or(""),
                });
                if let Ok(json_str) = serde_json::to_string(&event) {
                    event_sink.send(&json_str);
                    let _ = db.append_message(&session_id, &session::NewMessage::event(&json_str));
                }
            }

            let effort_ref = effort_str.as_deref();
            let cancel_clone = cancel.clone();

            // Shared state for capturing usage/TTFT from on_delta
            let captured_usage: Arc<std::sync::Mutex<CapturedUsage>> =
                Arc::new(std::sync::Mutex::new(CapturedUsage::default()));
            let captured_usage_clone = captured_usage.clone();

            // Accumulate text_delta / thinking_delta for ordering preservation
            let pending_text: Arc<std::sync::Mutex<String>> =
                Arc::new(std::sync::Mutex::new(String::new()));
            let pending_text_clone = pending_text.clone();
            let pending_thinking: Arc<std::sync::Mutex<String>> =
                Arc::new(std::sync::Mutex::new(String::new()));
            let pending_thinking_clone = pending_thinking.clone();
            let thinking_start_time: Arc<std::sync::Mutex<Option<std::time::Instant>>> =
                Arc::new(std::sync::Mutex::new(None));
            let thinking_start_clone = thinking_start_time.clone();
            let had_thinking_blocks: Arc<std::sync::atomic::AtomicBool> =
                Arc::new(std::sync::atomic::AtomicBool::new(false));
            let had_thinking_blocks_clone = had_thinking_blocks.clone();

            let db_for_cb = db.clone();
            let sid_for_cb = session_id.clone();
            let event_sink_clone = event_sink.clone();

            let history_len_before = agent.get_conversation_history().len();
            let chat_start = std::time::Instant::now();
            match agent
                .chat(
                    &message,
                    &attachments,
                    effort_ref,
                    cancel_clone,
                    move |delta| {
                        // Intercept usage events
                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                            match event.get("type").and_then(|t| t.as_str()) {
                                Some("usage") => {
                                    if let Ok(mut u) = captured_usage_clone.lock() {
                                        if let Some(v) =
                                            event.get("input_tokens").and_then(|v| v.as_i64())
                                        {
                                            u.input_tokens = Some(v);
                                        }
                                        if let Some(v) =
                                            event.get("output_tokens").and_then(|v| v.as_i64())
                                        {
                                            u.output_tokens = Some(v);
                                        }
                                        if let Some(v) = event.get("model").and_then(|v| v.as_str())
                                        {
                                            u.model = Some(v.to_string());
                                        }
                                        if let Some(v) =
                                            event.get("ttft_ms").and_then(|v| v.as_i64())
                                        {
                                            u.ttft_ms = Some(v);
                                        }
                                    }
                                }
                                Some("thinking_delta") => {
                                    if let Some(text) =
                                        event.get("content").and_then(|t| t.as_str())
                                    {
                                        // Record start time on first thinking_delta
                                        if let Ok(mut ts) = thinking_start_clone.lock() {
                                            if ts.is_none() {
                                                *ts = Some(std::time::Instant::now());
                                            }
                                        }
                                        if let Ok(mut pk) = pending_thinking_clone.lock() {
                                            pk.push_str(text);
                                        }
                                    }
                                }
                                Some("text_delta") => {
                                    if let Some(text) = event.get("text").and_then(|t| t.as_str()) {
                                        if let Ok(mut pt) = pending_text_clone.lock() {
                                            pt.push_str(text);
                                        }
                                    }
                                }
                                Some("tool_call") => {
                                    // Flush accumulated thinking before tool_call
                                    if let Ok(mut pk) = pending_thinking_clone.lock() {
                                        if !pk.is_empty() {
                                            let duration = thinking_start_clone
                                                .lock()
                                                .ok()
                                                .and_then(|mut ts| ts.take())
                                                .map(|t| t.elapsed().as_millis() as i64);
                                            let thinking_msg =
                                                session::NewMessage::thinking_block_with_duration(
                                                    &pk, duration,
                                                );
                                            let _ = db_for_cb
                                                .append_message(&sid_for_cb, &thinking_msg);
                                            pk.clear();
                                            had_thinking_blocks_clone
                                                .store(true, std::sync::atomic::Ordering::SeqCst);
                                        }
                                    }
                                    // Flush accumulated text before tool_call
                                    if let Ok(mut pt) = pending_text_clone.lock() {
                                        if !pt.is_empty() {
                                            let text_msg = session::NewMessage::text_block(&pt);
                                            let _ =
                                                db_for_cb.append_message(&sid_for_cb, &text_msg);
                                            pt.clear();
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                        event_sink_clone.send(delta);
                    },
                )
                .await
            {
                Ok((result, thinking)) => {
                    let duration_ms = chat_start.elapsed().as_millis() as u64;

                    // Emit usage event with duration
                    let usage_event = serde_json::json!({
                        "type": "usage",
                        "duration_ms": duration_ms,
                    });
                    if let Ok(json_str) = serde_json::to_string(&usage_event) {
                        event_sink.send(&json_str);
                    }

                    // Flush remaining pending thinking
                    if let Ok(mut pk) = pending_thinking.lock() {
                        if !pk.is_empty() {
                            let duration = thinking_start_time
                                .lock()
                                .ok()
                                .and_then(|mut ts| ts.take())
                                .map(|t| t.elapsed().as_millis() as i64);
                            let thinking_msg =
                                session::NewMessage::thinking_block_with_duration(&pk, duration);
                            let _ = db.append_message(&session_id, &thinking_msg);
                            pk.clear();
                            had_thinking_blocks.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let has_thinking_blocks =
                        had_thinking_blocks.load(std::sync::atomic::Ordering::SeqCst);

                    // Save assistant reply with metadata
                    let mut assistant_msg = session::NewMessage::assistant(&result);
                    assistant_msg.tool_duration_ms = Some(duration_ms as i64);
                    if !has_thinking_blocks {
                        assistant_msg.thinking = thinking;
                    }
                    if let Ok(u) = captured_usage.lock() {
                        assistant_msg.tokens_in = u.input_tokens;
                        assistant_msg.tokens_out = u.output_tokens;
                        assistant_msg.model = u.model.clone();
                        assistant_msg.ttft_ms = u.ttft_ms;
                    }
                    let _ = db.append_message(&session_id, &assistant_msg);

                    // Persist conversation context
                    save_agent_context(&db, &session_id, &agent);

                    {
                        let round_tokens = {
                            let u = captured_usage.lock().unwrap();
                            let input = u.input_tokens.unwrap_or(0);
                            let output = u.output_tokens.unwrap_or(0);
                            (input + output) as u32
                        };
                        let round_messages = agent
                            .get_conversation_history()
                            .len()
                            .saturating_sub(history_len_before)
                            as u32;
                        agent.accumulate_extraction_stats(round_tokens, round_messages);
                    }

                    let idle_timeout =
                        run_memory_extraction_inline(&agent_id, &session_id, model_ref, &agent)
                            .await;

                    // Schedule idle extraction if inline didn't trigger (tracking not reset)
                    if idle_timeout > 0 {
                        let tokens_remain = agent
                            .tokens_since_extraction
                            .load(std::sync::atomic::Ordering::SeqCst);
                        let msgs_remain = agent
                            .messages_since_extraction
                            .load(std::sync::atomic::Ordering::SeqCst);
                        if tokens_remain > 0 || msgs_remain > 0 {
                            let updated_at = db
                                .get_session(&session_id)
                                .ok()
                                .flatten()
                                .map(|s| s.updated_at)
                                .unwrap_or_default();
                            crate::memory_extract::schedule_idle_extraction(
                                agent_id.clone(),
                                session_id.clone(),
                                updated_at,
                                idle_timeout,
                            );
                        }
                    }

                    return Ok(ChatEngineResult {
                        response: result,
                        model_used: Some(model_ref.clone()),
                        agent: Some(agent),
                    });
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let reason = failover::classify_error(&error_msg);

                    app_warn!(
                        "provider",
                        "failover",
                        "Model {}::{} failed (attempt {}/{}, retry {}, reason {:?}): {}",
                        model_ref.provider_id,
                        model_ref.model_id,
                        idx + 1,
                        total_models,
                        retry_count,
                        reason,
                        error_msg
                    );

                    // Context overflow — try emergency compaction, then retry once
                    if reason.needs_compaction() && retry_count == 0 {
                        app_info!(
                            "context",
                            "compact",
                            "Context overflow on {}::{}, attempting emergency compaction",
                            model_ref.provider_id,
                            model_ref.model_id
                        );
                        let mut history = agent.get_conversation_history();
                        let compact_result =
                            agent.context_engine().emergency_compact(&mut history, &compact_config);
                        agent.set_conversation_history(history);
                        save_agent_context(&db, &session_id, &agent);

                        if let Ok(event_str) = serde_json::to_string(&serde_json::json!({
                            "type": "context_compacted",
                            "data": compact_result,
                        })) {
                            event_sink.send(&event_str);
                        }

                        retry_count += 1;
                        continue;
                    }

                    // Terminal errors — surface immediately
                    if reason.is_terminal() || reason.needs_compaction() {
                        save_agent_context(&db, &session_id, &agent);
                        let _ =
                            db.append_message(&session_id, &session::NewMessage::event(&error_msg));
                        return Err(error_msg);
                    }

                    // Retryable errors — retry same model with backoff
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay =
                            failover::retry_delay_ms(retry_count - 1, RETRY_BASE_MS, RETRY_MAX_MS);
                        app_info!(
                            "provider",
                            "failover",
                            "Retrying {}::{} in {}ms (retry {}/{})",
                            model_ref.provider_id,
                            model_ref.model_id,
                            delay,
                            retry_count,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    // Emit codex_auth_expired when a Codex provider gets an Auth error
                    if matches!(reason, failover::FailoverReason::Auth) {
                        let is_codex = providers
                            .iter()
                            .find(|p| p.id == model_ref.provider_id)
                            .map(|p| p.api_type == ApiType::Codex)
                            .unwrap_or(false);
                        if is_codex {
                            if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                                "type": "codex_auth_expired",
                                "error": &error_msg,
                            })) {
                                event_sink.send(&json_str);
                            }
                        }
                    }

                    // Non-retryable or retries exhausted — move to next model
                    last_error = Some(error_msg);
                    break;
                }
            }
        }
    }

    let final_error =
        last_error.unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    let _ = db.append_message(&session_id, &session::NewMessage::event(&final_error));
    Err(final_error)
}
