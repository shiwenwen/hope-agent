use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::failover::{
    self,
    executor::{execute_with_failover, ExecutorError, FailoverPolicy},
};
use crate::provider::{ApiType, AuthProfile};
use crate::session;

use super::context::*;
use super::im_mirror::{attach_im_mirrors, finalize_im_mirrors, ImMirrorState};
use super::persister::StreamPersister;
use super::sink_registry;
use super::stream_broadcast;
use super::stream_seq;
use super::types::*;

/// Successful chat round payload returned by the executor closure.
/// Bundles everything the post-success path needs to flush thinking, build
/// the assistant message, save context, and run extraction follow-ups.
struct ChatRoundOk {
    response: String,
    thinking: Option<String>,
    agent: AssistantAgent,
    persister: Arc<StreamPersister>,
    history_len_before: usize,
    chat_start: std::time::Instant,
}

/// Drop-guarded scope for a session's visible stream lifecycle. Ensures
/// `stream_seq::end` fires on every `run_chat_engine` return path (including
/// panics), while allowing the successful path to end the UI stream before
/// post-turn follow-ups run. Only desktop / HTTP turns broadcast on the main
/// `chat:*` bus; IM channel turns have a separate `channel:*` lifecycle.
struct StreamLifecycle {
    session_id: String,
    stream_id: Option<String>,
    source: stream_seq::ChatSource,
    finished: bool,
}

impl StreamLifecycle {
    fn begin(session_id: &str, source: stream_seq::ChatSource) -> Result<Self, String> {
        let stream_id = source
            .tracks_seq()
            .then(|| stream_seq::begin(session_id, source))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            session_id: session_id.to_string(),
            stream_id,
            source,
            finished: false,
        })
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        if let Some(ref stream_id) = self.stream_id {
            if self.source.broadcasts_to_user_ui() {
                stream_broadcast::broadcast_stream_end(&self.session_id, stream_id);
            }
            stream_seq::end(&self.session_id);
        }
        self.finished = true;
    }
}

impl Drop for StreamLifecycle {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Emit one stream event. Desktop / HTTP turns send through both the per-call
/// sink and the main `chat:stream_delta` EventBus path with a shared `_oc_seq`
/// for dedup. Channel / cron turns stay off the main chat bus; IM uses
/// `ChannelStreamSink` to emit `channel:stream_delta` instead.
fn emit_stream_event(
    event_sink: &std::sync::Arc<dyn EventSink>,
    session_id: &str,
    source: stream_seq::ChatSource,
    event: &str,
) {
    let payload: String = if !source.broadcasts_to_user_ui() {
        event_sink.send(event);
        event.to_string()
    } else {
        let (enveloped, seq, stream_id) = stream_broadcast::inject_seq(session_id, event);
        event_sink.send(&enveloped);
        stream_broadcast::broadcast_delta(session_id, &enveloped, seq, stream_id.as_deref());
        enveloped
    };
    // Fan-out to any extra sinks attached to this session (handover targets,
    // /session <id> observers, GUI ↔ IM mirrors). The primary `event_sink`
    // above is intentionally not registered, so each consumer fires once.
    sink_registry::sink_registry().emit(session_id, &payload);
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
        compact_config,
        extra_system_context,
        reasoning_effort,
        cancel,
        plan_context_override,
        skill_allowed_tools,
        denied_tools,
        subagent_depth,
        steer_run_id,
        auto_approve_tools,
        follow_global_reasoning_effort,
        post_turn_effects,
        abort_on_cancel,
        persist_final_error_event,
        source,
        event_sink,
    } = params;

    // Wrap attachments in Arc<[T]> so the failover-executor closure's per-
    // retry capture is a pointer bump instead of a deep clone of base64
    // image data (Attachment.data may carry MB-sized strings).
    let attachments: std::sync::Arc<[crate::agent::Attachment]> = std::sync::Arc::from(attachments);

    if model_chain.is_empty() {
        return Err("No model configured for chat execution".to_string());
    }

    // Resolve the Plan-mode bundle once at turn start. Spawn-supplied
    // overrides win (their child sessions have backend `plan_mode = Off`
    // even though they're meant to run as PlanAgent); otherwise read this
    // session's backend state. The `plan_context_locked` flag rides along
    // so configure_agent picks the right setter and the streaming loop's
    // mid-turn probe knows whether to leave the bundle alone.
    //
    // The plan-derived extra context is NOT merged into the caller's
    // `extra_system_context` here — it goes into a separate agent slot
    // (`plan_extra_context`) so the streaming loop's mid-turn probe can
    // swap it on a state flip without losing the caller's framing
    // (cron task / subagent role / etc.). `build_full_system_prompt`
    // appends both.
    let plan_context_locked = plan_context_override.is_some();
    let plan_resolved = match plan_context_override {
        Some(o) => o,
        None => crate::chat_engine::resolve_plan_context_for_session(&session_id).await,
    };

    // Codex OAuth token lives on disk; it's the single source of truth for
    // desktop / HTTP / IM channel entry points. Callers may pass None — when
    // the chain actually needs Codex we hydrate from disk here so all three
    // runtimes behave identically without threading AppState through.
    let chain_needs_codex = model_chain.iter().any(|m| {
        providers
            .iter()
            .any(|p| p.id == m.provider_id && p.api_type == ApiType::Codex)
    });
    let mut codex_token = codex_token;
    if chain_needs_codex {
        let current = codex_token.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
        // Refresh on-disk token if stale; if a refresh produced a new pair,
        // also update the in-memory hint we thread down to the agent builder
        // — the disk write inside refresh may have failed, but the new token
        // is still valid in this process.
        if let Some(pair) = crate::oauth::ensure_fresh_codex_token(current).await {
            codex_token = Some(pair);
        }
    }

    let mut stream_lifecycle = StreamLifecycle::begin(&session_id, source)?;

    // Attach IM-mirror sinks for any primary attach this session has
    // (Phase B7 follow-up). No-op for non-Desktop / non-Http sources.
    // The state is held to the end of `run_chat_engine` so each
    // mirror's stream task stays alive while the engine streams; the
    // success path moves the state into `finalize_im_mirrors` to flush
    // the final reply, and any error / early return drops the state so
    // every sink detaches and every stream task observes channel-close.
    let mut im_mirrors: ImMirrorState = attach_im_mirrors(&session_id, source);

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
        // Look up provider once per model. Skip the model if missing — same
        // semantics as the pre-Phase-3 build_agent_from_snapshot None path.
        let current_provider = providers.iter().find(|p| p.id == model_ref.provider_id);
        let prov = match current_provider {
            Some(p) => p,
            None => {
                last_error = Some(format!(
                    "Provider {} not found for model {}",
                    model_ref.provider_id, model_ref.model_id
                ));
                continue;
            }
        };

        // Update session with current model info
        {
            let provider_name = Some(prov.name.as_str());
            let _ = db.update_session_model(
                &session_id,
                Some(&model_ref.provider_id),
                provider_name,
                Some(&model_ref.model_id),
            );
        }

        // Emit fallback event if this is not the first model in the chain.
        // Only fires once per model (not per executor retry / rotation).
        if idx > 0 {
            let display = format!("{} / {}", prov.name, model_ref.model_id);
            let reason_str = last_error
                .as_deref()
                .map(failover::classify_error)
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
                emit_stream_event(&event_sink, &session_id, source, &json_str);
                let _ = db.append_message(&session_id, &session::NewMessage::event(&json_str));
            }
        }

        // ── Outer compaction-retry loop ─────────────────────────
        // The executor (execute_with_failover) handles profile rotation +
        // retry-with-backoff in one call. Context overflow is the only
        // signal that needs to escape and re-enter — emergency_compact
        // borrows the agent mutably so it can't run inside the closure
        // while the operation is still holding the agent. After compact,
        // we write the failed profile back to PROFILE_STICKY so the next
        // executor call's select_profile picks it (preserves prompt cache
        // prefix that compaction did NOT invalidate).
        let mut compaction_attempts: u32 = 0;
        const MAX_COMPACTION_RETRIES: u32 = 1;
        let model_provider_id = model_ref.provider_id.clone();
        let model_id = model_ref.model_id.clone();

        loop {
            // Build the on-rotation callback that emits profile_rotation
            // events. Borrows event_sink + session_id + provider/model ids;
            // executor calls it inline so no Send/Sync gymnastics needed.
            let on_rotate =
                |from: &AuthProfile, to: &AuthProfile, reason: &failover::FailoverReason| {
                    app_info!(
                        "provider",
                        "failover",
                        "Rotating auth profile for {}::{}: {} -> {} (reason: {:?})",
                        model_provider_id,
                        model_id,
                        from.label,
                        to.label,
                        reason
                    );
                    if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                        "type": "profile_rotation",
                        "provider_id": model_provider_id,
                        "model_id": model_id,
                        "from_profile": from.label,
                        "to_profile": to.label,
                        "reason": reason,
                    })) {
                        emit_stream_event(&event_sink, &session_id, source, &json_str);
                    }
                };

            // Capture refs / clones the closure needs. `move` consumes per-
            // call clones; the original chat_engine values stay borrowable
            // for the next compaction-retry iteration.
            let providers_ref = &providers;
            let compact_config_ref = &compact_config;
            let agent_id_ref = &agent_id;
            let session_id_ref = &session_id;
            let extra_system_context_ref = &extra_system_context;
            let skill_allowed_tools_ref = &skill_allowed_tools;
            let plan_resolved_ref = &plan_resolved;
            let message_ref = &message;
            let attachments_ref = &attachments;
            let effort_str_ref = &effort_str;
            let cancel_ref = &cancel;
            let event_sink_ref = &event_sink;
            let db_ref = &db;
            let model_ref_for_op = model_ref;
            let codex_token_ref = &codex_token;

            let exec_result = execute_with_failover(
                prov,
                &session_id,
                FailoverPolicy::chat_engine_default(),
                Some(&on_rotate),
                |profile| {
                    let profile_owned = profile.cloned();
                    // Sync setup: build + configure + restore. If build
                    // fails (e.g. Codex without token), surface as Unknown
                    // so the executor exhausts and we move to next model.
                    // Per-call clones for the streaming callback's `move ||`.
                    let event_sink_for_cb = event_sink_ref.clone();
                    let session_for_cb = session_id_ref.clone();
                    let source_for_cb = source;
                    let cancel_for_op = cancel_ref.clone();
                    let cancel_for_check = cancel_for_op.clone();

                    let agent_id_owned = agent_id_ref.clone();
                    let session_id_owned = session_id_ref.clone();
                    let extra_ctx_owned = extra_system_context_ref.clone();
                    let skill_tools_owned = skill_allowed_tools_ref.clone();
                    let denied_tools_owned = denied_tools.clone();
                    let steer_run_id_owned = steer_run_id.clone();
                    let plan_resolved_owned = plan_resolved_ref.clone();
                    let message_owned = message_ref.clone();
                    // Arc<[Attachment]> clone is a pointer bump regardless
                    // of attachment size. See param destructure for the wrap.
                    let attachments_owned = attachments_ref.clone();
                    let effort_owned = effort_str_ref.clone();
                    let db_owned = db_ref.clone();
                    let provider_id_for_err = model_ref_for_op.provider_id.clone();
                    let model_id_for_err = model_ref_for_op.model_id.clone();
                    let codex_token_owned = codex_token_ref.clone();

                    async move {
                        let mut agent = build_agent_from_snapshot(
                            model_ref_for_op,
                            providers_ref,
                            codex_token_owned,
                            compact_config_ref,
                            profile_owned.as_ref(),
                            session_id_ref,
                        )
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Cannot build agent for {}::{}: {}",
                                provider_id_for_err,
                                model_id_for_err,
                                e
                            )
                        })?;
                        configure_agent(
                            &mut agent,
                            &agent_id_owned,
                            &session_id_owned,
                            resolved_temperature,
                            extra_ctx_owned.as_deref(),
                            &skill_tools_owned,
                            &denied_tools_owned,
                            subagent_depth,
                            steer_run_id_owned,
                            plan_resolved_owned,
                            plan_context_locked,
                            auto_approve_tools,
                            follow_global_reasoning_effort,
                        );
                        restore_agent_context(&db_owned, &session_id_owned, &agent);

                        let history_len_before = agent.get_conversation_history().len();
                        let chat_start = std::time::Instant::now();
                        let persister =
                            StreamPersister::new(db_owned.clone(), session_id_owned.clone());
                        let persist_cb = persister.build_callback();

                        let chat_result = agent
                            .chat(
                                &message_owned,
                                &attachments_owned,
                                effort_owned.as_deref(),
                                cancel_for_op,
                                move |delta| {
                                    persist_cb(delta);
                                    emit_stream_event(
                                        &event_sink_for_cb,
                                        &session_for_cb,
                                        source_for_cb,
                                        delta,
                                    );
                                },
                            )
                            .await;

                        if abort_on_cancel
                            && cancel_for_check.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            // Discard any partial placeholder this attempt left
                            // behind so a cancelled run doesn't show up as an
                            // orphan in the next turn's restore summary.
                            persister.discard_active_placeholder();
                            return Err(anyhow::anyhow!("chat cancelled by caller"));
                        }

                        match chat_result {
                            Ok((response, thinking)) => Ok(ChatRoundOk {
                                response,
                                thinking,
                                agent,
                                persister,
                                history_len_before,
                                chat_start,
                            }),
                            Err(e) => {
                                // Failover may retry on a different model; the
                                // failed attempt's partial text must NOT bleed
                                // into the eventual successful bubble (frontend
                                // would group both text_block rows under the
                                // same assistant) or into the next turn's
                                // orphan-summary injection.
                                persister.discard_active_placeholder();
                                Err(e)
                            }
                        }
                    }
                },
            )
            .await;

            match exec_result {
                Ok(ok) => {
                    let ChatRoundOk {
                        response,
                        thinking,
                        agent,
                        persister,
                        history_len_before,
                        chat_start,
                    } = ok;
                    let duration_ms = chat_start.elapsed().as_millis() as u64;

                    // Emit usage event with duration
                    let usage_event = serde_json::json!({
                        "type": "usage",
                        "duration_ms": duration_ms,
                    });
                    if let Ok(json_str) = serde_json::to_string(&usage_event) {
                        emit_stream_event(&event_sink, &session_id, source, &json_str);
                    }

                    persister.flush_remaining_thinking();
                    let trailing_text = persister.take_trailing_text();
                    let assistant_msg =
                        persister.build_assistant_message(&trailing_text, thinking, duration_ms);
                    let _ = db.append_message(&session_id, &assistant_msg);

                    // Persist conversation context
                    save_agent_context(&db, &session_id, &agent);

                    // The user-visible response is complete once the final
                    // assistant row is durable. End the frontend stream here;
                    // memory extraction and other follow-ups below must not
                    // keep the stop button/sidebar spinner alive.
                    stream_lifecycle.finish();

                    if post_turn_effects {
                        crate::session_title::maybe_schedule_after_success(
                            db.clone(),
                            session_id.clone(),
                            agent_id.clone(),
                            model_ref.clone(),
                            providers.clone(),
                        );

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

                        let idle_timeout = schedule_memory_extraction_after_turn(
                            &agent_id,
                            &session_id,
                            model_ref,
                            &agent,
                        );

                        // Phase B'1: skill auto-review — same as pre-Phase-3.
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
                                    crate::skills::auto_review::sweep_stale(7 * 24 * 3600);
                                });
                            }
                        }

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
                    }

                    // GUI / HTTP turns flush IM mirrors here: drop every
                    // mirror's event_tx so each preview-stream task
                    // observes channel-close and returns its final
                    // preview message id, then either edit_message
                    // (preview existed) or send_message (no preview)
                    // chunks of the final response per mirror. Subagent /
                    // ParentInjection / Channel sources never reach this
                    // branch with non-empty `im_mirrors` (see
                    // `attach_im_mirrors` source filter), so the no-op
                    // case is cheap.
                    if !im_mirrors.is_empty() {
                        finalize_im_mirrors(std::mem::take(&mut im_mirrors), &response).await;
                    }

                    return Ok(ChatEngineResult {
                        response,
                        model_used: Some(model_ref.clone()),
                        agent: Some(agent),
                    });
                }

                Err(ExecutorError::NeedsCompaction { last_profile }) => {
                    if compaction_attempts >= MAX_COMPACTION_RETRIES {
                        app_warn!(
                            "context",
                            "compact",
                            "Context overflow on {}::{} persists after compaction, moving to next model",
                            model_ref.provider_id,
                            model_ref.model_id
                        );
                        last_error = Some(format!(
                            "Context overflow on {}::{} after emergency compaction",
                            model_ref.provider_id, model_ref.model_id
                        ));
                        break;
                    }
                    compaction_attempts += 1;

                    app_info!(
                        "context",
                        "compact",
                        "Context overflow on {}::{}, attempting emergency compaction",
                        model_ref.provider_id,
                        model_ref.model_id
                    );

                    // Build a temporary agent to run the compaction. Same
                    // profile that just hit overflow so the cache prefix is
                    // identical.
                    let mut compact_agent = match build_agent_from_snapshot(
                        model_ref,
                        &providers,
                        codex_token.clone(),
                        &compact_config,
                        last_profile.as_ref(),
                        &session_id,
                    )
                    .await
                    {
                        Ok(a) => a,
                        Err(e) => {
                            last_error = Some(format!(
                                "Cannot build agent for emergency compaction on {}::{}: {}",
                                model_ref.provider_id, model_ref.model_id, e
                            ));
                            break;
                        }
                    };
                    configure_agent(
                        &mut compact_agent,
                        &agent_id,
                        &session_id,
                        resolved_temperature,
                        extra_system_context.as_deref(),
                        &skill_allowed_tools,
                        &denied_tools,
                        subagent_depth,
                        steer_run_id.clone(),
                        plan_resolved.clone(),
                        plan_context_locked,
                        auto_approve_tools,
                        follow_global_reasoning_effort,
                    );
                    restore_agent_context(&db, &session_id, &compact_agent);

                    let mut history = compact_agent.get_conversation_history();
                    let compact_result = compact_agent
                        .context_engine()
                        .emergency_compact(&mut history, &compact_config);
                    compact_agent.set_conversation_history(history);
                    save_agent_context(&db, &session_id, &compact_agent);

                    if let Ok(event_str) = serde_json::to_string(&serde_json::json!({
                        "type": "context_compacted",
                        "data": compact_result,
                    })) {
                        emit_stream_event(&event_sink, &session_id, source, &event_str);
                    }

                    // Write the just-failed profile back to PROFILE_STICKY
                    // so the next executor call's select_profile picks it
                    // first (compaction reduces tokens but doesn't change
                    // the cached prefix → same key avoids a cache miss).
                    if let Some(ref p) = last_profile {
                        failover::PROFILE_STICKY.set(&model_ref.provider_id, &session_id, &p.id);
                    }
                    continue;
                }

                Err(ExecutorError::Exhausted {
                    last_reason,
                    last_error: err_str,
                }) => {
                    app_warn!(
                        "provider",
                        "failover",
                        "Giving up on {}::{} (reason {:?}), moving to next model in chain",
                        model_ref.provider_id,
                        model_ref.model_id,
                        last_reason
                    );

                    // Codex Auth → emit codex_auth_expired so frontend can
                    // prompt the user to re-authorize.
                    if matches!(last_reason, failover::FailoverReason::Auth)
                        && prov.api_type == ApiType::Codex
                    {
                        if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                            "type": "codex_auth_expired",
                            "error": &err_str,
                        })) {
                            emit_stream_event(&event_sink, &session_id, source, &json_str);
                        }
                    }

                    last_error = Some(err_str);
                    break;
                }

                Err(ExecutorError::NoProfileAvailable) => {
                    app_warn!(
                        "provider",
                        "failover",
                        "No auth profile available for {}::{}",
                        model_ref.provider_id,
                        model_ref.model_id
                    );
                    last_error = Some(format!(
                        "No auth profile available for {}::{}",
                        model_ref.provider_id, model_ref.model_id
                    ));
                    break;
                }
            }
        }
    }

    let final_error =
        last_error.unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    app_error!(
        "provider",
        "failover",
        "All {} models exhausted for session {}: {}",
        total_models,
        session_id,
        final_error
    );
    if persist_final_error_event {
        persist_failed_turn_context(&db, &session_id, &message, &final_error);
        let _ = db.append_message(&session_id, &session::NewMessage::error_event(&final_error));
    }
    Err(final_error)
}

/// Apply common agent configuration. Extracted to avoid duplication between
/// initial agent setup and profile-rotation rebuild.
///
/// `plan_resolved` is the full Plan-mode bundle (state + mode + allow_paths
/// + extra_system_context). The `plan_locked` flag picks the right setter
/// so the streaming loop's mid-turn probe knows whether it's free to re-sync.
#[allow(clippy::too_many_arguments)]
fn configure_agent(
    agent: &mut crate::agent::AssistantAgent,
    agent_id: &str,
    session_id: &str,
    temperature: Option<f64>,
    extra_system_context: Option<&str>,
    skill_allowed_tools: &[String],
    denied_tools: &[String],
    subagent_depth: u32,
    steer_run_id: Option<String>,
    plan_resolved: crate::agent::PlanResolvedContext,
    plan_locked: bool,
    auto_approve_tools: bool,
    follow_global_reasoning_effort: bool,
) {
    agent.set_agent_id(agent_id);
    agent.set_session_id(session_id);
    agent.set_temperature(temperature);
    if let Some(ctx) = extra_system_context {
        agent.set_extra_system_context(ctx.to_string());
    }
    if !skill_allowed_tools.is_empty() {
        agent.set_skill_allowed_tools(skill_allowed_tools.to_vec());
    }
    if !denied_tools.is_empty() {
        agent.set_denied_tools(denied_tools.to_vec());
    }
    agent.set_subagent_depth(subagent_depth);
    if let Some(run_id) = steer_run_id {
        agent.set_steer_run_id(run_id);
    }
    // Atomic 4-slot plan apply (state + mode + allow_paths + extra_context).
    // `_external` locks against the streaming loop's mid-turn probe
    // (spawn-supplied override), `_from_backend` leaves the probe free to
    // re-sync (snapshot read of this session's backend state).
    if plan_locked {
        agent.apply_plan_resolved_external(plan_resolved);
    } else {
        agent.apply_plan_resolved_from_backend(plan_resolved);
    }
    if auto_approve_tools {
        agent.set_auto_approve_tools(true);
    }
    if follow_global_reasoning_effort {
        // Main-chat path: let provider tool loops re-read the live global effort
        // so UI toggles apply to the next API request, not only the next turn.
        agent.set_follow_global_reasoning_effort(true);
    }
}

#[cfg(test)]
mod stream_lifecycle_tests {
    use super::*;

    #[test]
    fn finish_marks_stream_inactive_before_scope_drop() {
        let sid = "test-chat-engine-stream-lifecycle-finish";

        {
            let mut lifecycle =
                StreamLifecycle::begin(sid, stream_seq::ChatSource::Desktop).unwrap();
            assert!(stream_seq::is_active(sid));

            lifecycle.finish();

            assert!(!stream_seq::is_active(sid));
        }

        assert!(!stream_seq::is_active(sid));
    }
}
