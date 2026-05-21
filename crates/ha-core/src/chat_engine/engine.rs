use std::sync::{Arc, Mutex};

use crate::agent::AssistantAgent;
use crate::failover::{
    self,
    executor::{execute_with_failover, ExecutorError, FailoverPolicy},
};
use crate::provider::{ApiType, AuthProfile};
use crate::session;

use super::context::*;
use super::finalize::{self, PartialMeta, TerminationReason};
use super::im_mirror::{attach_im_live_mirror, finalize_im_live_mirror};
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

/// Stores the last failed attempt that produced user-visible partial output.
/// Empty retry failures intentionally do not replace the slot, so a later
/// all-failed result can still preserve the most recent partial the user saw.
///
/// `api_type` is the provider shape that *wrote* this partial — captured
/// at store time so finalize can rebuild the assistant-side blocks in
/// the matching native format. Without it, model_chain rotation followed
/// by a no-partial failure on the second attempt would make finalize
/// rebuild the *first* attempt's partial using the *second* attempt's
/// provider shape, and the next request would 4xx or silently drop tool
/// calls depending on which provider it crossed into.
struct FailedAttemptPartial {
    persister: Arc<StreamPersister>,
    duration_ms: u64,
    api_type: crate::provider::ApiType,
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
    turn_id: Option<String>,
    terminal_status: Option<session::ChatTurnStatus>,
    interrupt_reason: Option<session::ChatTurnInterruptReason>,
    terminal_error: Option<String>,
    finished: bool,
}

impl StreamLifecycle {
    fn begin(
        session_id: &str,
        source: stream_seq::ChatSource,
        turn_id: Option<String>,
    ) -> Result<Self, String> {
        let stream_id = source
            .tracks_seq()
            .then(|| stream_seq::begin(session_id, source))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            session_id: session_id.to_string(),
            stream_id,
            source,
            turn_id,
            terminal_status: None,
            interrupt_reason: None,
            terminal_error: None,
            finished: false,
        })
    }

    fn set_terminal(
        &mut self,
        status: session::ChatTurnStatus,
        interrupt_reason: Option<session::ChatTurnInterruptReason>,
        error: Option<String>,
    ) {
        debug_assert!(status.is_terminal());
        if self.terminal_status.is_none() {
            self.terminal_status = Some(status);
            self.interrupt_reason = interrupt_reason;
            self.terminal_error = error;
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        if let Some(ref stream_id) = self.stream_id {
            if self.source.broadcasts_to_user_ui() {
                stream_broadcast::broadcast_stream_end(
                    &self.session_id,
                    Some(stream_id),
                    self.turn_id.as_deref(),
                    self.terminal_status,
                    self.interrupt_reason,
                    self.terminal_error.as_deref(),
                );
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

fn take_failed_attempt_partial(
    slot: &Arc<Mutex<Option<FailedAttemptPartial>>>,
) -> Option<FailedAttemptPartial> {
    match slot.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn store_failed_attempt_partial(
    slot: &Arc<Mutex<Option<FailedAttemptPartial>>>,
    partial: FailedAttemptPartial,
) {
    if let Some(previous) = take_failed_attempt_partial(slot) {
        previous.persister.discard_attempt_rows();
    }
    match slot.lock() {
        Ok(mut guard) => {
            *guard = Some(partial);
        }
        Err(poisoned) => {
            *poisoned.into_inner() = Some(partial);
        }
    }
}

fn discard_failed_attempt_partial(slot: &Arc<Mutex<Option<FailedAttemptPartial>>>) {
    if let Some(partial) = take_failed_attempt_partial(slot) {
        partial.persister.discard_attempt_rows();
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
    turn_id: Option<&str>,
    event: &str,
) {
    let payload: String = if !source.broadcasts_to_user_ui() {
        event_sink.send(event);
        event.to_string()
    } else {
        let (enveloped, seq, stream_id) = stream_broadcast::inject_seq(session_id, event, turn_id);
        event_sink.send(&enveloped);
        stream_broadcast::broadcast_delta(session_id, &enveloped, seq, stream_id.as_deref());
        enveloped
    };
    // Fan-out to any extra sinks attached to this session (live GUI ↔ IM
    // mirror is the primary consumer). The primary `event_sink`
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
        turn_id,
        message,
        display_text,
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

    let mut stream_lifecycle = StreamLifecycle::begin(&session_id, source, turn_id.clone())?;
    if let (Some(ref turn_id), Some(ref stream_id)) =
        (turn_id.as_ref(), stream_lifecycle.stream_id.as_ref())
    {
        let _ = super::active_turn::set_stream_id(&session_id, turn_id, stream_id);
        if let Err(e) = db.update_chat_turn_stream_id(turn_id, stream_id) {
            app_warn!(
                "chat",
                "turn",
                "Failed to persist stream id for turn {}: {}",
                turn_id,
                e
            );
        }
        if source.broadcasts_to_user_ui() {
            stream_broadcast::broadcast_turn_started(&session_id, turn_id, Some(stream_id));
        }
    }

    // IM-mirror prefers the friendly `display_text` (e.g. `Using skill **X**...`
    // rendered for `/skill` invocations) so attached IM chats see what the
    // desktop user saw, not the raw `[SYSTEM:...]` prompt fed to the model.
    let mut im_mirror = attach_im_live_mirror(
        &session_id,
        source,
        Some(crate::chat_engine::im_mirror::LastUserSnapshot {
            source: source.as_str().to_string(),
            text: crate::util::non_empty_trim_or(display_text.as_deref(), &message).to_owned(),
            attachment_count: attachments.len(),
        }),
    )
    .await;

    let total_models = model_chain.len();
    let mut last_error: Option<String> = None;
    // Preserve the executor's typed verdict from `ExecutorError::Exhausted`
    // so the IM mirror abort path can render a per-class friendly notice
    // (`🔐 Authentication failed`, `⏱️ Rate limited`, …). Re-classifying
    // `last_error` at the abort site is lossy — provider-specific
    // wrapping can drop the original 4xx/5xx markers that
    // `failover::classify_error` keys off.
    let mut last_reason: Option<failover::FailoverReason> = None;
    // Pinned to `true` only when the failing model's provider is Codex
    // *and* its failure reason is Auth — drives the "re-authorize via
    // desktop app" headline. Tracked per-failure rather than derived from
    // primary-only because the failover chain may have rotated through
    // multiple providers, and the user-facing hint depends on which one
    // actually erred.
    let mut last_is_codex_auth = false;
    // Provider shape of the most recently attempted model; drives the
    // finalize path's partial-block reconstruction. Updated each
    // model_chain iteration so the value reflects whichever model
    // produced the partial currently stored in `failed_attempt_partial`.
    let mut last_provider_api_kind: Option<crate::provider::ApiType> = None;
    // Set when emergency compaction was attempted but still failed to
    // bring history below the model's context window — promoted into
    // `TerminationReason::CompactionFailed` by `derive_termination_reason`
    // so the marker classifies the failure correctly instead of folding
    // it into a generic provider error.
    let mut compaction_failed: Option<String> = None;
    // True when the most recent model attempt bailed with
    // `ExecutorError::NoProfileAvailable`. We still fill `last_reason`
    // / `last_error` in that branch so logs include the model id, but
    // the unified finalize taxonomy needs to surface this as the
    // explicit `NoProfileAvailable` reason (not generic `ProviderFailed`)
    // so the user-facing copy can say "configure provider" instead of
    // "all models failed".
    let mut last_was_no_profile = false;
    let failed_attempt_partial: Arc<Mutex<Option<FailedAttemptPartial>>> =
        Arc::new(Mutex::new(None));

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
                let msg = format!(
                    "Provider {} not found for model {}",
                    model_ref.provider_id, model_ref.model_id
                );
                last_reason = Some(failover::classify_error(&msg));
                last_error = Some(msg);
                continue;
            }
        };
        last_provider_api_kind = Some(prov.api_type.clone());

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
                emit_stream_event(
                    &event_sink,
                    &session_id,
                    source,
                    turn_id.as_deref(),
                    &json_str,
                );
                let _ = db.append_message(
                    &session_id,
                    &session::NewMessage::event(&json_str).with_source(source),
                );
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
                        emit_stream_event(
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &json_str,
                        );
                        // Persist as `role=event` so the GUI's
                        // ProfileRotationBanner survives session reload.
                        let _ = db.append_message(
                            &session_id,
                            &session::NewMessage::event(&json_str).with_source(source),
                        );
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
            let failed_attempt_partial_ref = failed_attempt_partial.clone();

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
                    let turn_id_for_cb = turn_id.clone();

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
                    let failed_attempt_partial_owned = failed_attempt_partial_ref.clone();
                    // Stamp partial with the provider shape that wrote
                    // it; see `FailedAttemptPartial::api_type` rationale.
                    let api_type_for_partial = prov.api_type.clone();

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
                        let persister = StreamPersister::new(
                            db_owned.clone(),
                            session_id_owned.clone(),
                            source_for_cb,
                        );
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
                                        turn_id_for_cb.as_deref(),
                                        delta,
                                    );
                                },
                            )
                            .await;

                        if abort_on_cancel
                            && cancel_for_check.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            // Discard any partial placeholder this attempt left
                            // behind so a cancelled run doesn't show up after
                            // reload. This must clear completed text/tool rows
                            // too, not just the currently active placeholder.
                            persister.discard_attempt_rows();
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
                                if persister.has_visible_partial_output() {
                                    store_failed_attempt_partial(
                                        &failed_attempt_partial_owned,
                                        FailedAttemptPartial {
                                            persister,
                                            duration_ms: chat_start.elapsed().as_millis() as u64,
                                            api_type: api_type_for_partial.clone(),
                                        },
                                    );
                                } else {
                                    persister.discard_attempt_rows();
                                }
                                Err(e)
                            }
                        }
                    }
                },
            )
            .await;

            match exec_result {
                Ok(ok) => {
                    discard_failed_attempt_partial(&failed_attempt_partial);

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
                        emit_stream_event(
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &json_str,
                        );
                    }

                    persister.flush_remaining_thinking();
                    let trailing_text = persister.take_trailing_text();
                    let assistant_msg =
                        persister.build_assistant_message(&trailing_text, thinking, duration_ms);
                    let assistant_id = db.append_message(&session_id, &assistant_msg).ok();

                    // Persist conversation context
                    save_agent_context(&db, &session_id, &agent);

                    // User-stop on a non-abort path. `abort_on_cancel=false`
                    // (Desktop / HTTP / IM / Cron) means `agent.chat` returned
                    // Ok with whatever partial accumulated rather than Err on
                    // cancel. Without this branch the partial would be filed
                    // as a normal `Completed` assistant turn and the model
                    // would never know the user pressed stop on its next
                    // reply. Route through `finalize(UserStop)` so a `[系统
                    // 事件]` marker is appended to `context_json`, a user-
                    // visible event row lands, and the chat_turn closes with
                    // `Interrupted/UserStop` instead of `Completed`.
                    if !abort_on_cancel
                        && cancel.load(std::sync::atomic::Ordering::SeqCst)
                        && persist_final_error_event
                    {
                        // No partial-block rebuild: `save_agent_context` above
                        // already pushed the in-progress history (including
                        // the just-written assistant row) into context_json.
                        // finalize only needs to append the marker, write the
                        // event row, and close the turn.
                        let partial = PartialMeta {
                            user_message: Some(message.clone()),
                            provider_kind: Some(prov.api_type.clone().into()),
                            text: None,
                            thinking: None,
                            tool_calls: Vec::new(),
                            executed_tools: Vec::new(),
                            round_id: None,
                            turn_id: turn_id.clone(),
                            assistant_message_id: assistant_id,
                        };
                        let outcome = finalize::finalize_turn_context(
                            &db,
                            &session_id,
                            TerminationReason::UserStop,
                            partial,
                            source,
                            im_mirror.take(),
                        )
                        .await;
                        let terminal = outcome
                            .turn_status
                            .unwrap_or(session::ChatTurnStatus::Interrupted);
                        stream_lifecycle.set_terminal(terminal, outcome.interrupt_reason, None);
                        stream_lifecycle.finish();
                        return Ok(ChatEngineResult {
                            response,
                            model_used: Some(model_ref.clone()),
                            agent: Some(agent),
                        });
                    }

                    // GUI / HTTP turns mirror into the attached IM chat via
                    // the live stream sink. Kick the final IM flush before
                    // ending the frontend lifecycle and before running
                    // post-turn side effects so title/memory work cannot
                    // delay the remote chat's finalization. It runs in the
                    // background so slow IM network calls never hold the GUI
                    // path open.
                    if let Some(state) = im_mirror.take() {
                        let mirror_response = response.clone();
                        tokio::spawn(async move {
                            finalize_im_live_mirror(state, &mirror_response).await;
                        });
                    }

                    // The user-visible response is complete once the final
                    // assistant row is durable. End the frontend stream here;
                    // memory extraction and other follow-ups below must not
                    // keep the stop button/sidebar spinner alive.
                    let mut terminal_status = session::ChatTurnStatus::Completed;
                    let mut interrupt_reason = None;
                    if let Some(ref turn_id) = turn_id {
                        if let Ok(Some(turn)) = db.finish_chat_turn_after_execution(
                            turn_id,
                            cancel.load(std::sync::atomic::Ordering::SeqCst),
                            None,
                            assistant_id,
                        ) {
                            terminal_status = turn.status;
                            interrupt_reason = turn.interrupt_reason;
                        }
                    }
                    stream_lifecycle.set_terminal(terminal_status, interrupt_reason, None);
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

                        // Skill auto-review trigger (gate 1 of the five-gate
                        // waterfall). Feed tool_use_count from this round's
                        // conversation slice — pure-chat turns yield 0 and
                        // are filtered by `require_tool_use` in the config.
                        // `history_tail_stats` walks the slice under one lock
                        // without cloning the whole history.
                        {
                            let round_tokens = {
                                let u = persister.usage();
                                let input = u.input_tokens.unwrap_or(0);
                                let output = u.output_tokens.unwrap_or(0);
                                (input + output) as usize
                            };
                            let (round_messages, tool_use_count) =
                                agent.history_tail_stats(history_len_before);
                            let cfg = crate::config::cached_config()
                                .skills
                                .auto_review
                                .clone()
                                .sanitize();
                            // Two user messages within 30 seconds is the
                            // "user is correcting themselves" signal — cheap
                            // DB read, only consulted when the master
                            // toggle is on.
                            let user_correction = cfg.correction_signal_enabled
                                && db.user_messages_within(&session_id, 30).unwrap_or(false);
                            let signals = crate::skills::auto_review::TriggerSignals {
                                turn_tokens: round_tokens,
                                new_messages: round_messages,
                                tool_use_count,
                                user_correction,
                            };
                            if let Some(gate) = crate::skills::auto_review::touch_and_maybe_trigger(
                                &session_id,
                                signals,
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

                    return Ok(ChatEngineResult {
                        response,
                        model_used: Some(model_ref.clone()),
                        agent: Some(agent),
                    });
                }

                Err(ExecutorError::NeedsCompaction { last_profile }) => {
                    discard_failed_attempt_partial(&failed_attempt_partial);

                    if compaction_attempts >= MAX_COMPACTION_RETRIES {
                        app_warn!(
                            "context",
                            "compact",
                            "Context overflow on {}::{} persists after compaction, moving to next model",
                            model_ref.provider_id,
                            model_ref.model_id
                        );
                        let msg = format!(
                            "Context overflow on {}::{} after emergency compaction",
                            model_ref.provider_id, model_ref.model_id
                        );
                        last_reason = Some(failover::classify_error(&msg));
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
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
                            let msg = format!(
                                "Cannot build agent for emergency compaction on {}::{}: {}",
                                model_ref.provider_id, model_ref.model_id, e
                            );
                            last_reason = Some(failover::classify_error(&msg));
                            last_error = Some(msg);
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

                    // Manual snake_case shape — `CompactResult` itself is
                    // `rename_all="camelCase"`, but the frontend / IM
                    // formatter / persister all key off snake_case fields
                    // (matching `agent/context.rs`'s pre-LLM compaction
                    // emit). Direct `"data": compact_result` would silently
                    // skip every consumer's tier filter.
                    if let Ok(event_str) = serde_json::to_string(&serde_json::json!({
                        "type": "context_compacted",
                        "data": {
                            "tier_applied": compact_result.tier_applied,
                            "tokens_before": compact_result.tokens_before,
                            "tokens_after": compact_result.tokens_after,
                            "messages_affected": compact_result.messages_affected,
                            "description": compact_result.description,
                        },
                    })) {
                        emit_stream_event(
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &event_str,
                        );
                        // emergency_compact always runs Tier ≥ 3 — persist
                        // unconditionally so the GUI's ContextCompactedBanner
                        // survives session reload. Per-turn pre-LLM compaction
                        // (agent/context.rs) is filtered separately in the
                        // persister's `context_compacted` arm.
                        let _ = db.append_message(
                            &session_id,
                            &session::NewMessage::event(&event_str).with_source(source),
                        );
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
                    last_reason: r,
                    last_error: err_str,
                }) => {
                    app_warn!(
                        "provider",
                        "failover",
                        "Giving up on {}::{} (reason {:?}), moving to next model in chain",
                        model_ref.provider_id,
                        model_ref.model_id,
                        r
                    );

                    // Codex Auth → emit codex_auth_expired so frontend can
                    // prompt the user to re-authorize.
                    let is_codex_auth =
                        matches!(r, failover::FailoverReason::Auth) && prov.api_type.is_codex();
                    if is_codex_auth {
                        if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                            "type": "codex_auth_expired",
                            "error": &err_str,
                        })) {
                            emit_stream_event(
                                &event_sink,
                                &session_id,
                                source,
                                turn_id.as_deref(),
                                &json_str,
                            );
                        }
                    }

                    last_is_codex_auth = is_codex_auth;
                    last_reason = Some(r);
                    last_error = Some(err_str);
                    last_was_no_profile = false;
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
                    let msg = format!(
                        "No auth profile available for {}::{}",
                        model_ref.provider_id, model_ref.model_id
                    );
                    last_reason = Some(failover::classify_error(&msg));
                    last_error = Some(msg);
                    last_was_no_profile = true;
                    break;
                }
            }
        }
    }

    // All non-success paths (cancel, exhausted, no-profile, compaction
    // give-up) converge here.
    let final_error = last_error
        .clone()
        .unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    app_error!(
        "provider",
        "failover",
        "All {} models exhausted for session {}: {}",
        total_models,
        session_id,
        final_error
    );

    let reason = derive_termination_reason(
        abort_on_cancel,
        &cancel,
        last_reason,
        last_error.as_deref(),
        last_is_codex_auth,
        compaction_failed.as_deref(),
        last_was_no_profile,
    );

    // Discard or preserve the visible partial depending on the
    // disambiguated termination reason. Subagent `abort_on_cancel=true`
    // is the only path that throws partials away.
    let (failed_assistant_id, partial_api_type) =
        if matches!(reason, TerminationReason::UserStop) && abort_on_cancel {
            discard_failed_attempt_partial(&failed_attempt_partial);
            (None, None)
        } else {
            match take_failed_attempt_partial(&failed_attempt_partial) {
                Some(partial) => {
                    let assistant_id = partial
                        .persister
                        .persist_failed_partial_assistant(None, partial.duration_ms);
                    (assistant_id, Some(partial.api_type))
                }
                None => (None, None),
            }
        };

    // Prefer the API type that *wrote* the surviving partial; fall
    // back to the last attempted provider when no partial exists
    // (which only matters for `provider_kind` selection — the message
    // table will be empty of that turn's blocks).
    let api_type_for_rebuild = partial_api_type.or_else(|| last_provider_api_kind.clone());
    let partial = collect_partial_meta_from_runtime(
        &db,
        &session_id,
        &message,
        api_type_for_rebuild,
        failed_assistant_id,
        turn_id.as_deref(),
    );

    if persist_final_error_event {
        let outcome = finalize::finalize_turn_context(
            &db,
            &session_id,
            reason.clone(),
            partial,
            source,
            im_mirror.take(),
        )
        .await;
        let terminal_status = outcome
            .turn_status
            .unwrap_or(session::ChatTurnStatus::Failed);
        let terminal_error =
            (terminal_status == session::ChatTurnStatus::Failed).then(|| final_error.clone());
        stream_lifecycle.set_terminal(terminal_status, outcome.interrupt_reason, terminal_error);
    } else {
        // Subagent / Cron / IM-inbound entry points self-manage their
        // user-facing error surfaces (the channel worker has its own IM
        // notice, subagents drop partials, cron writes its delivery
        // event). The unified path still writes `chat_turns` if there's
        // a turn id, but skips context_json / event row / IM dispatch.
        if let Some(ref tid) = turn_id {
            let _ = db.finish_chat_turn_once(
                tid,
                reason.to_chat_turn_status(),
                Some(reason.to_chat_turn_interrupt_reason()),
                reason.to_error_text().as_deref(),
                failed_assistant_id,
            );
        }
        stream_lifecycle.set_terminal(
            reason.to_chat_turn_status(),
            Some(reason.to_chat_turn_interrupt_reason()),
            (reason.to_chat_turn_status() == session::ChatTurnStatus::Failed)
                .then(|| final_error.clone()),
        );
    }

    Err(final_error)
}

// ── Termination reason derivation ────────────────────────────────────

/// Map runtime convergence state to a [`TerminationReason`].
///
/// `cancel + abort_on_cancel` is the only positive signal for
/// `UserStop` — `cancel.load()` alone fires on IM channel turns where
/// `/cancel` flips the flag but the engine still wants to surface the
/// partial as a normal failure. `last_reason == None` after a non-cancel
/// path means we never even reached an executor call → `NoProfileAvailable`.
/// Everything else is `ProviderFailed` carrying the classified reason.
fn derive_termination_reason(
    abort_on_cancel: bool,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_reason: Option<failover::FailoverReason>,
    last_error: Option<&str>,
    last_is_codex_auth: bool,
    compaction_failed: Option<&str>,
    last_was_no_profile: bool,
) -> TerminationReason {
    let user_cancel = abort_on_cancel && cancel.load(std::sync::atomic::Ordering::SeqCst);
    if user_cancel {
        return TerminationReason::UserStop;
    }
    if let Some(detail) = compaction_failed {
        return TerminationReason::CompactionFailed {
            detail: detail.to_string(),
        };
    }
    // Profile-availability failure is configuration-class, not API-class.
    // The `Err(NoProfileAvailable)` branch fills `last_reason`/`last_error`
    // for logging, but the unified taxonomy surfaces this distinctly.
    if last_was_no_profile {
        return TerminationReason::NoProfileAvailable;
    }
    match (last_reason, last_error) {
        (Some(kind), Some(msg)) => TerminationReason::ProviderFailed {
            last_kind: kind,
            last_message: msg.to_string(),
            is_codex_auth: last_is_codex_auth,
        },
        (Some(kind), None) => TerminationReason::ProviderFailed {
            last_kind: kind,
            last_message: String::new(),
            is_codex_auth: last_is_codex_auth,
        },
        (None, Some(msg)) => TerminationReason::Other {
            message: msg.to_string(),
        },
        (None, None) => TerminationReason::NoProfileAvailable,
    }
}

/// Build [`PartialMeta`] from runtime convergence state.
///
/// The text / thinking / tool_use rebuild is reverse-engineered from
/// the `messages` table by [`finalize::rebuild::collect_partial_from_messages`]
/// — `persist_failed_partial_assistant` has already written the
/// assistant row that links text/thinking blocks, and the tool rows
/// persist independently. Runtime only needs to overlay metadata that
/// the table doesn't carry (user_message text for the early-persist
/// gap, provider shape from the last attempt, turn id, persisted
/// assistant id).
fn collect_partial_meta_from_runtime(
    db: &std::sync::Arc<session::SessionDB>,
    session_id: &str,
    user_message: &str,
    api_type: Option<crate::provider::ApiType>,
    assistant_message_id: Option<i64>,
    turn_id: Option<&str>,
) -> PartialMeta {
    let provider_kind = api_type.map(finalize::ProviderApiKind::from);
    let mut meta = finalize::rebuild::collect_partial_from_messages(db, session_id, provider_kind);
    meta.user_message = Some(user_message.to_string());
    meta.turn_id = turn_id.map(str::to_owned);
    if assistant_message_id.is_some() {
        meta.assistant_message_id = assistant_message_id;
    }
    meta
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
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::context_compact::CompactConfig;
    use crate::provider::{ActiveModel, ApiType, ModelConfig, ProviderConfig};
    use crate::session::{MessageRole, NewMessage, SessionDB};
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn finish_marks_stream_inactive_before_scope_drop() {
        let sid = "test-chat-engine-stream-lifecycle-finish";

        {
            let mut lifecycle =
                StreamLifecycle::begin(sid, stream_seq::ChatSource::Desktop, None).unwrap();
            assert!(stream_seq::is_active(sid));

            lifecycle.finish();

            assert!(!stream_seq::is_active(sid));
        }

        assert!(!stream_seq::is_active(sid));
    }

    fn temp_db() -> (TempDir, Arc<SessionDB>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let db = Arc::new(SessionDB::open(&path).unwrap());
        (dir, db)
    }

    fn model_config(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            name: id.to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        }
    }

    fn openai_provider(base_url: String, model_id: &str) -> ProviderConfig {
        let mut provider = ProviderConfig::new(
            format!("test-provider-{model_id}"),
            ApiType::OpenaiResponses,
            base_url,
            "test-key".to_string(),
        );
        provider.models.push(model_config(model_id));
        provider
    }

    fn sse_text_then_done(text: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            text
        )
    }

    fn sse_partial_then_failed(text: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"message\":\"upstream failed\",\"code\":\"bad_response_status_code\",\"type\":\"server_error\"}}}}}}\n\n",
            text
        )
    }

    fn sse_thinking_then_failed(text: &str) -> String {
        format!(
            "data: {{\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"message\":\"upstream failed\",\"code\":\"bad_response_status_code\",\"type\":\"server_error\"}}}}}}\n\n",
            text
        )
    }

    fn sse_partial_then_timeout_failed(text: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"message\":\"request timeout\",\"code\":\"timeout\",\"type\":\"timeout\"}}}}}}\n\n",
            text
        )
    }

    fn sse_failed_without_output() -> String {
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"upstream failed\",\"code\":\"bad_response_status_code\",\"type\":\"server_error\"}}}\n\n".to_string()
    }

    fn sse_tool_call_then_done(text: &str, path: &str) -> String {
        let args = serde_json::to_string(&serde_json::json!({ "path": path, "limit": 1 }))
            .expect("serialize tool args");
        let args_json = serde_json::to_string(&args).expect("serialize args as json string");
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"read\",\"arguments\":{}}}}}\n\n\
             data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"read\",\"arguments\":{}}}}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            text, args_json, args_json
        )
    }

    fn params(
        db: Arc<SessionDB>,
        session_id: String,
        model_chain: Vec<ActiveModel>,
        providers: Vec<ProviderConfig>,
    ) -> ChatEngineParams {
        ChatEngineParams {
            session_id,
            agent_id: crate::agent_loader::DEFAULT_AGENT_ID.to_string(),
            turn_id: None,
            message: "hello".to_string(),
            display_text: None,
            attachments: Vec::new(),
            session_db: db,
            model_chain,
            providers,
            codex_token: None,
            resolved_temperature: None,
            compact_config: CompactConfig::default(),
            extra_system_context: None,
            reasoning_effort: Some("none".to_string()),
            cancel: Arc::new(AtomicBool::new(false)),
            plan_context_override: Some(crate::agent::PlanResolvedContext::off()),
            skill_allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            subagent_depth: 0,
            steer_run_id: None,
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            post_turn_effects: false,
            abort_on_cancel: false,
            persist_final_error_event: true,
            source: stream_seq::ChatSource::Desktop,
            event_sink: Arc::new(NoopEventSink),
        }
    }

    struct CancelOnTextDelta {
        cancel: Arc<AtomicBool>,
    }

    impl EventSink for CancelOnTextDelta {
        fn send(&self, event: &str) {
            if event.contains("\"type\":\"text_delta\"") {
                self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    struct CancelOnToolCall {
        cancel: Arc<AtomicBool>,
    }

    impl EventSink for CancelOnToolCall {
        fn send(&self, event: &str) {
            if event.contains("\"type\":\"tool_call\"") {
                self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    #[tokio::test]
    async fn final_failure_preserves_partial_assistant_before_error_event() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_partial_then_failed("partial answer")),
            )
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };

        let result = run_chat_engine(params(
            db.clone(),
            session.id.clone(),
            vec![model],
            vec![provider],
        ))
        .await;
        assert!(result.is_err());

        let messages = db.load_session_messages(&session.id).unwrap();
        let assistant_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Assistant)
            .expect("partial assistant should be persisted");
        let error_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Event && msg.is_error == Some(true))
            .expect("error event should be persisted");
        assert!(assistant_idx < error_idx);
        assert_eq!(messages[assistant_idx].content, "partial answer");
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::TextBlock));

        let context_json = db
            .load_context(&session.id)
            .unwrap()
            .expect("failed turn should persist model context");
        assert!(
            context_json.contains("partial answer"),
            "failed partial assistant should be visible to the next turn context: {context_json}"
        );
        // The unified finalize path keeps the partial as a structured
        // native block (`output_text` for Responses) and writes the
        // model marker as a separate assistant message, instead of the
        // old behavior of flattening both into one. The marker phrasing
        // is Chinese now since `copy::model_marker` is the source of
        // truth.
        let context: Vec<serde_json::Value> = serde_json::from_str(&context_json).unwrap();
        let assistant_contexts: Vec<_> = context
            .iter()
            .filter(|item| item.get("role").and_then(|role| role.as_str()) == Some("assistant"))
            .collect();
        assert_eq!(
            assistant_contexts.len(),
            2,
            "expected one partial assistant block + one [系统事件] marker assistant: {context_json}"
        );
        // Last assistant message is the model marker (Chinese, says
        // "all configured models failed").
        let marker = assistant_contexts
            .last()
            .unwrap()
            .get("content")
            .and_then(|c| c.as_str())
            .expect("marker is plain text assistant");
        assert!(marker.contains("[系统事件]"), "marker: {marker}");
        assert!(marker.contains("所有已配置模型都失败"), "marker: {marker}");
    }

    #[tokio::test]
    async fn final_failure_context_includes_completed_tool_args_and_result() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();

        let readable = std::env::current_dir()
            .unwrap()
            .join("Cargo.toml")
            .to_string_lossy()
            .to_string();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_tool_call_then_done("partial before tool", &readable)),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_partial_then_failed("failed after tool")),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };

        let result = run_chat_engine(params(
            db.clone(),
            session.id.clone(),
            vec![model],
            vec![provider],
        ))
        .await;
        assert!(result.is_err());

        let messages = db.load_session_messages(&session.id).unwrap();
        assert!(
            messages.iter().any(|msg| {
                msg.role == MessageRole::Tool
                    && msg.tool_name.as_deref() == Some("read")
                    && msg
                        .tool_result
                        .as_deref()
                        .is_some_and(|result| result.contains("[Read 1 lines"))
            }),
            "completed tool row should remain in DB history"
        );

        let context_json = db
            .load_context(&session.id)
            .unwrap()
            .expect("failed turn should persist model context");
        assert!(
            context_json.contains("failed after tool"),
            "partial text should be preserved in context: {context_json}"
        );
        // Unified finalize keeps tool calls as Responses-native
        // function_call / function_call_output items rather than the
        // old flattened `[Tool call: read]\nArguments: ...` markdown.
        // The name, args path, and result text all still appear in the
        // raw JSON.
        assert!(
            context_json.contains("\"name\":\"read\""),
            "tool name should be preserved as native function_call: {context_json}"
        );
        assert!(
            context_json.contains("Cargo.toml"),
            "tool args should be preserved in context: {context_json}"
        );
        assert!(
            context_json.contains("[Read 1 lines"),
            "tool result should be preserved in context: {context_json}"
        );
    }

    #[tokio::test]
    async fn final_failure_preserves_thinking_only_without_text_bubble() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_thinking_then_failed("thinking only")),
            )
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };

        let result = run_chat_engine(params(
            db.clone(),
            session.id.clone(),
            vec![model],
            vec![provider],
        ))
        .await;
        assert!(result.is_err());

        let messages = db.load_session_messages(&session.id).unwrap();
        let thinking_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::ThinkingBlock)
            .expect("thinking block should be persisted");
        let assistant_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Assistant)
            .expect("assistant row should claim thinking-only block");
        let error_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Event && msg.is_error == Some(true))
            .expect("error event should be persisted");
        assert!(thinking_idx < assistant_idx);
        assert!(assistant_idx < error_idx);
        assert_eq!(messages[assistant_idx].content, "");
        assert_eq!(messages[thinking_idx].content, "thinking only");

        let context_json = db
            .load_context(&session.id)
            .unwrap()
            .expect("failed turn should persist model context");
        // The unified finalize path intentionally preserves thinking
        // content in the model-facing history — the design principle
        // is "let the model perceive as much of what happened as
        // possible". For Responses-shaped partials, thinking and text
        // are merged into a single `output_text` since reasoning
        // items require an `encrypted_content` we don't have for
        // runtime partials.
        assert!(
            context_json.contains("thinking only"),
            "thinking should be preserved in model-facing context for thinking-only failures: {context_json}"
        );
        // Chinese marker mentions "all configured models failed".
        assert!(
            context_json.contains("所有已配置模型都失败"),
            "marker should classify provider failure: {context_json}"
        );
    }

    #[tokio::test]
    async fn abort_on_cancel_discards_failed_partial_candidate() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_partial_then_failed("partial before cancel")),
            )
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut params = params(db.clone(), session.id.clone(), vec![model], vec![provider]);
        params.cancel = cancel.clone();
        params.abort_on_cancel = true;
        params.event_sink = Arc::new(CancelOnTextDelta {
            cancel: cancel.clone(),
        });

        let result = run_chat_engine(params).await;
        assert!(result.is_err());
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));

        let messages = db.load_session_messages(&session.id).unwrap();
        assert!(!messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant || msg.content == "partial before cancel"
        }));
    }

    #[tokio::test]
    async fn abort_on_cancel_after_tool_call_discards_completed_attempt_rows() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();
        let readable = std::env::current_dir()
            .unwrap()
            .join("Cargo.toml")
            .to_string_lossy()
            .to_string();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_tool_call_then_done("partial before tool", &readable)),
            )
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut params = params(db.clone(), session.id.clone(), vec![model], vec![provider]);
        params.cancel = cancel.clone();
        params.abort_on_cancel = true;
        params.event_sink = Arc::new(CancelOnToolCall {
            cancel: cancel.clone(),
        });

        let result = run_chat_engine(params).await;
        assert!(result.is_err());
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));

        let messages = db.load_session_messages(&session.id).unwrap();
        assert!(!messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant
                || msg.role == MessageRole::TextBlock
                || msg.role == MessageRole::Tool
                || msg.content == "partial before tool"
        }));
    }

    #[tokio::test]
    async fn fallback_success_discards_failed_model_partial() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();

        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_partial_then_failed("failed partial")),
            )
            .mount(&first)
            .await;

        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_text_then_done("final answer")),
            )
            .mount(&second)
            .await;

        let provider1 = openai_provider(first.uri(), "m1");
        let provider2 = openai_provider(second.uri(), "m2");
        let model1 = ActiveModel {
            provider_id: provider1.id.clone(),
            model_id: "m1".to_string(),
        };
        let model2 = ActiveModel {
            provider_id: provider2.id.clone(),
            model_id: "m2".to_string(),
        };

        let result = run_chat_engine(params(
            db.clone(),
            session.id.clone(),
            vec![model1, model2],
            vec![provider1, provider2],
        ))
        .await
        .expect("fallback model should succeed");
        assert_eq!(result.response, "final answer");

        let messages = db.load_session_messages(&session.id).unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role == MessageRole::Assistant)
            .collect();
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].content, "final answer");
        assert!(!messages.iter().any(|msg| msg.content == "failed partial"));
    }

    #[tokio::test]
    async fn fallback_success_discards_failed_model_tool_round() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();
        let readable = std::env::current_dir()
            .unwrap()
            .join("Cargo.toml")
            .to_string_lossy()
            .to_string();

        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_tool_call_then_done("failed completed round", &readable)),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&first)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_partial_then_failed("failed trailing")),
            )
            .with_priority(2)
            .mount(&first)
            .await;

        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_text_then_done("final answer")),
            )
            .mount(&second)
            .await;

        let provider1 = openai_provider(first.uri(), "m1");
        let provider2 = openai_provider(second.uri(), "m2");
        let model1 = ActiveModel {
            provider_id: provider1.id.clone(),
            model_id: "m1".to_string(),
        };
        let model2 = ActiveModel {
            provider_id: provider2.id.clone(),
            model_id: "m2".to_string(),
        };

        let result = run_chat_engine(params(
            db.clone(),
            session.id.clone(),
            vec![model1, model2],
            vec![provider1, provider2],
        ))
        .await
        .expect("fallback model should succeed");
        assert_eq!(result.response, "final answer");

        let messages = db.load_session_messages(&session.id).unwrap();
        let assistants: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role == MessageRole::Assistant)
            .collect();
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].content, "final answer");
        assert!(!messages.iter().any(|msg| {
            msg.content == "failed completed round" || msg.content == "failed trailing"
        }));
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::Tool
                && msg.tool_call_id.as_deref() == Some("call-1")));
    }

    #[tokio::test]
    async fn final_failure_preserves_previous_partial_when_last_attempt_is_empty() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_partial_then_timeout_failed("visible before retry")),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_failed_without_output()),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };

        let result = run_chat_engine(params(
            db.clone(),
            session.id.clone(),
            vec![model],
            vec![provider],
        ))
        .await;
        assert!(result.is_err());

        let messages = db.load_session_messages(&session.id).unwrap();
        let assistant_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Assistant)
            .expect("previous visible partial should be persisted");
        let error_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Event && msg.is_error == Some(true))
            .expect("error event should be persisted");
        assert!(assistant_idx < error_idx);
        assert_eq!(messages[assistant_idx].content, "visible before retry");
    }
}
