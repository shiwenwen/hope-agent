use crate::failover;
use crate::provider::{ApiType, AuthProfile};
use crate::session;

use super::context::*;
use super::persister::StreamPersister;
use super::stream_broadcast;
use super::stream_seq;
use super::types::*;

/// Drop-guarded scope for a session's stream lifecycle. Ensures `stream_seq::end`
/// + `chat:stream_end` broadcast fire on every `run_chat_engine` return path
/// (including panics).
struct StreamLifecycle {
    session_id: String,
}

impl StreamLifecycle {
    fn begin(session_id: &str) -> Self {
        stream_seq::begin(session_id);
        Self {
            session_id: session_id.to_string(),
        }
    }
}

impl Drop for StreamLifecycle {
    fn drop(&mut self) {
        stream_seq::end(&self.session_id);
        stream_broadcast::broadcast_stream_end(&self.session_id);
    }
}

/// Emit one stream event through the per-call sink and the EventBus broadcast,
/// injecting a monotonic `_oc_seq` shared by both paths.
fn emit_stream_event(
    event_sink: &std::sync::Arc<dyn EventSink>,
    session_id: &str,
    event: &str,
) {
    let (enveloped, seq) = stream_broadcast::inject_seq(session_id, event);
    event_sink.send(&enveloped);
    stream_broadcast::broadcast_delta(session_id, &enveloped, seq);
}

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

    let _stream_lifecycle = StreamLifecycle::begin(&session_id);

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
        // ── Auth profile selection ──────────────────────────────
        let current_provider = providers
            .iter()
            .find(|p| p.id == model_ref.provider_id);
        let mut current_profile: Option<AuthProfile> = current_provider
            .and_then(|prov| failover::select_profile(prov, &session_id));
        let mut profile_rotated = false;
        let mut tried_profiles: Vec<String> = Vec::new();
        if let Some(ref p) = current_profile {
            tried_profiles.push(p.id.clone());
        }

        let mut agent =
            match build_agent_from_snapshot(
                model_ref,
                &providers,
                &codex_token,
                &compact_config,
                current_profile.as_ref(),
            ) {
                Some(a) => a,
                None => {
                    last_error = Some(format!(
                        "Cannot build agent for {}::{}",
                        model_ref.provider_id, model_ref.model_id
                    ));
                    continue;
                }
            };
        configure_agent(
            &mut agent,
            &agent_id,
            &session_id,
            web_search_enabled,
            notification_enabled,
            image_gen_config.clone(),
            canvas_enabled,
            resolved_temperature,
            extra_system_context.as_deref(),
            &skill_allowed_tools,
            plan_agent_mode.as_ref(),
            plan_mode_allow_paths.as_ref(),
            auto_approve_tools,
        );

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
            if idx > 0 && retry_count == 0 && !profile_rotated {
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
                    emit_stream_event(&event_sink, &session_id, &json_str);
                    let _ = db.append_message(&session_id, &session::NewMessage::event(&json_str));
                }
            }

            let effort_ref = effort_str.as_deref();
            let cancel_clone = cancel.clone();

            let persister = StreamPersister::new();
            let persist_cb = persister.build_callback(&db, session_id.clone());
            let event_sink_clone = event_sink.clone();
            let session_id_for_cb = session_id.clone();

            let history_len_before = agent.get_conversation_history().len();
            let chat_start = std::time::Instant::now();
            match agent
                .chat(
                    &message,
                    &attachments,
                    effort_ref,
                    cancel_clone,
                    move |delta| {
                        persist_cb(delta);
                        emit_stream_event(&event_sink_clone, &session_id_for_cb, delta);
                    },
                )
                .await
            {
                Ok((result, thinking)) => {
                    let duration_ms = chat_start.elapsed().as_millis() as u64;

                    // ── Profile rotation: mark success ──────────
                    if let Some(ref profile) = current_profile {
                        failover::PROFILE_STICKY.set(
                            &model_ref.provider_id,
                            &session_id,
                            &profile.id,
                        );
                        failover::PROFILE_COOLDOWNS.clear(&profile.id);
                    }

                    // Emit usage event with duration
                    let usage_event = serde_json::json!({
                        "type": "usage",
                        "duration_ms": duration_ms,
                    });
                    if let Ok(json_str) = serde_json::to_string(&usage_event) {
                        emit_stream_event(&event_sink, &session_id, &json_str);
                    }

                    persister.flush_remaining_thinking(&db, &session_id);
                    let trailing_text = persister.take_trailing_text();
                    let assistant_msg = persister.build_assistant_message(
                        &trailing_text,
                        thinking,
                        duration_ms,
                    );
                    let _ = db.append_message(&session_id, &assistant_msg);

                    // Persist conversation context
                    save_agent_context(&db, &session_id, &agent);

                    {
                        let usage_snapshot = persister.usage();
                        let round_tokens = {
                            let input = usage_snapshot.input_tokens.unwrap_or(0);
                            let output = usage_snapshot.output_tokens.unwrap_or(0);
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

                    // Phase B'1: skill auto-review — in a single registry
                    // lock, record this turn's stats AND decide whether to
                    // fire. Only spawn the background task when the gate is
                    // actually acquired, so short turns that don't cross the
                    // threshold don't pay the cost of starting a task at all.
                    {
                        let round_tokens = {
                            let u = persister.usage();
                            let input = u.input_tokens.unwrap_or(0);
                            let output = u.output_tokens.unwrap_or(0);
                            (input + output) as usize
                        };
                        let round_messages = agent
                            .get_conversation_history()
                            .len()
                            .saturating_sub(history_len_before);
                        let cfg = crate::config::cached_config()
                            .skills
                            .auto_review
                            .clone()
                            .sanitize();
                        if let Some(gate) = crate::skills::auto_review::touch_and_maybe_trigger(
                            &session_id,
                            round_tokens,
                            round_messages,
                            &cfg,
                        ) {
                            let session_id_for_review = session_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = crate::skills::auto_review::run_review_cycle(
                                    &session_id_for_review,
                                    crate::skills::auto_review::ReviewTrigger::PostTurn,
                                    gate,
                                    None,
                                )
                                .await
                                {
                                    app_warn!(
                                        "skills",
                                        "auto_review",
                                        "post-turn review cycle failed: {}",
                                        e
                                    );
                                }
                                // Opportunistic sweep (cheap, runs ~once per fired review).
                                crate::skills::auto_review::sweep_stale(7 * 24 * 3600);
                            });
                        }
                    }

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
                            emit_stream_event(&event_sink, &session_id, &event_str);
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

                    // ── Auth profile rotation ───────────────────
                    // On profile-rotatable errors, try the next auth profile
                    // within the same provider before falling through to
                    // model-level retry/failover.
                    if reason.is_profile_rotatable() {
                        if let Some(ref profile) = current_profile {
                            failover::PROFILE_COOLDOWNS.mark_cooldown(&profile.id, &reason);
                        }
                        if let Some(prov) = current_provider {
                            if let Some(next) = failover::next_profile(prov, &tried_profiles) {
                                app_info!(
                                    "provider",
                                    "failover",
                                    "Rotating auth profile for {}::{}: {} -> {} (reason: {:?})",
                                    model_ref.provider_id,
                                    model_ref.model_id,
                                    current_profile.as_ref().map(|p| p.label.as_str()).unwrap_or("?"),
                                    next.label,
                                    reason
                                );

                                // Emit profile_rotation event to frontend
                                if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                                    "type": "profile_rotation",
                                    "provider_id": model_ref.provider_id,
                                    "model_id": model_ref.model_id,
                                    "from_profile": current_profile.as_ref().map(|p| p.label.as_str()),
                                    "to_profile": next.label,
                                    "reason": reason,
                                })) {
                                    emit_stream_event(&event_sink, &session_id, &json_str);
                                }

                                tried_profiles.push(next.id.clone());
                                current_profile = Some(next.clone());
                                profile_rotated = true;

                                // Rebuild agent with the new profile, preserving conversation history
                                let history = agent.get_conversation_history();
                                if let Some(new_agent) = build_agent_from_snapshot(
                                    model_ref,
                                    &providers,
                                    &codex_token,
                                    &compact_config,
                                    Some(&next),
                                ) {
                                    agent = new_agent;
                                    configure_agent(
                                        &mut agent,
                                        &agent_id,
                                        &session_id,
                                        web_search_enabled,
                                        notification_enabled,
                                        image_gen_config.clone(),
                                        canvas_enabled,
                                        resolved_temperature,
                                        extra_system_context.as_deref(),
                                        &skill_allowed_tools,
                                        plan_agent_mode.as_ref(),
                                        plan_mode_allow_paths.as_ref(),
                                        auto_approve_tools,
                                    );
                                    agent.set_conversation_history(history);
                                    retry_count = 0; // reset retries for the new profile
                                    continue;
                                }
                            }
                        }
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
                                emit_stream_event(&event_sink, &session_id, &json_str);
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

/// Apply common agent configuration. Extracted to avoid duplication between
/// initial agent setup and profile-rotation rebuild.
#[allow(clippy::too_many_arguments)]
fn configure_agent(
    agent: &mut crate::agent::AssistantAgent,
    agent_id: &str,
    session_id: &str,
    web_search_enabled: bool,
    notification_enabled: bool,
    image_gen_config: Option<crate::tools::image_generate::ImageGenConfig>,
    canvas_enabled: bool,
    temperature: Option<f64>,
    extra_system_context: Option<&str>,
    skill_allowed_tools: &[String],
    plan_agent_mode: Option<&crate::agent::PlanAgentMode>,
    plan_mode_allow_paths: Option<&Vec<String>>,
    auto_approve_tools: bool,
) {
    agent.set_agent_id(agent_id);
    agent.set_session_id(session_id);
    agent.set_web_search_enabled(web_search_enabled);
    agent.set_notification_enabled(notification_enabled);
    agent.set_image_generate_config(image_gen_config);
    agent.set_canvas_enabled(canvas_enabled);
    agent.set_temperature(temperature);
    if let Some(ctx) = extra_system_context {
        agent.set_extra_system_context(ctx.to_string());
    }
    if !skill_allowed_tools.is_empty() {
        agent.set_skill_allowed_tools(skill_allowed_tools.to_vec());
    }
    if let Some(mode) = plan_agent_mode {
        agent.set_plan_agent_mode(mode.clone());
    }
    if let Some(paths) = plan_mode_allow_paths {
        agent.set_plan_mode_allow_paths(paths.clone());
    }
    if auto_approve_tools {
        agent.set_auto_approve_tools(true);
    }
}
