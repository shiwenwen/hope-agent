use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::failover;
use crate::provider::{ActiveModel, ApiType, ProviderConfig};
use crate::session;
use crate::turn_durability::TurnDurabilitySink;

use super::context::*;
use super::finalize::{self, TerminationReason};
use super::sink_registry;
use super::stream_broadcast;
use super::stream_seq;
use super::types::*;

const CHAT_CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// Deletes durable typed-resource snapshots unless the Initial Context event
/// that references them has crossed the durability barrier. A backend run UUID
/// in every basename gives crash recovery the same deterministic cleanup scope
/// if the process exits before this guard can run.
#[doc(hidden)]
pub struct PendingTypedResourceSnapshots {
    pub session_id: String,
    pub snapshot_names: Vec<String>,
    pub refs_committed: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for PendingTypedResourceSnapshots {
    fn drop(&mut self) {
        if self
            .refs_committed
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        crate::attachments::remove_uncommitted_typed_resource_snapshots(
            &self.session_id,
            &self.snapshot_names,
        );
    }
}

#[doc(hidden)]
pub async fn wait_for_chat_cancel(cancel: Arc<std::sync::atomic::AtomicBool>) {
    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(CHAT_CANCEL_POLL_INTERVAL).await;
    }
}

#[doc(hidden)]
pub fn event_enters_runtime_loop(event: &str) -> bool {
    event.contains("\"type\":\"text_delta\"")
        || event.contains("\"type\":\"thinking_delta\"")
        || event.contains("\"type\":\"tool_call\"")
        || event.contains("\"type\":\"tool_result\"")
}

#[doc(hidden)]
pub fn should_retry_model_chain(
    current_round: u32,
    max_rounds: u32,
    reason: Option<failover::FailoverReason>,
    no_profile_available: bool,
    compaction_failed: bool,
    had_tool_activity: bool,
) -> bool {
    current_round < max_rounds
        && matches!(
            reason,
            Some(failover::FailoverReason::Timeout | failover::FailoverReason::Unknown)
        )
        && !no_profile_available
        && !compaction_failed
        && !had_tool_activity
}

#[doc(hidden)]
pub fn chain_reason_after_missing_provider(
    previous: Option<failover::FailoverReason>,
) -> failover::FailoverReason {
    match previous {
        Some(reason)
            if matches!(
                reason,
                failover::FailoverReason::Timeout
                    | failover::FailoverReason::Unknown
                    | failover::FailoverReason::ContextOverflow
            ) =>
        {
            reason
        }
        _ => failover::FailoverReason::ModelNotFound,
    }
}

#[doc(hidden)]
pub fn fallback_event_reason(
    typed_reason: Option<failover::FailoverReason>,
    display_error: Option<&str>,
) -> failover::FailoverReason {
    typed_reason
        .or_else(|| display_error.map(failover::classify_error))
        .unwrap_or(failover::FailoverReason::Unknown)
}

#[doc(hidden)]
pub fn has_resolvable_fallback(
    model_chain: &[ActiveModel],
    providers: &[ProviderConfig],
    current_index: usize,
) -> bool {
    let Some(remaining) = current_index
        .checked_add(1)
        .and_then(|next_index| model_chain.get(next_index..))
    else {
        return false;
    };

    remaining.iter().any(|candidate| {
        providers
            .iter()
            .any(|provider| provider.id == candidate.provider_id)
    })
}

#[doc(hidden)]
pub fn resolve_slash_skill_binding<'a>(
    entries: &'a [crate::skills::SkillEntry],
    target_id: &str,
    command_name: &str,
) -> Option<&'a crate::skills::SkillEntry> {
    let names = entries
        .iter()
        .map(|entry| entry.all_command_names().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    crate::slash_defs::resolve_dynamic_command_names(
        &names,
        crate::slash_defs::builtin_command_names(),
    )
    .into_iter()
    .find(|resolved| {
        resolved.typed_name == command_name && entries[resolved.entry_index].name == target_id
    })
    .map(|resolved| &entries[resolved.entry_index])
}

#[doc(hidden)]
pub fn ensure_explicit_slash_skill_requirements(
    entry: &crate::skills::SkillEntry,
    env_check: bool,
    skill_env: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) -> Result<(), String> {
    if !env_check {
        return Ok(());
    }
    let detail =
        crate::skills::check_requirements_detail(&entry.requires, skill_env.get(&entry.name));
    if detail.eligible {
        Ok(())
    } else {
        Err(format!(
            "Explicit slash skill '{}' is no longer eligible: {}",
            entry.name,
            crate::skills::format_requirements_diagnostic(entry, &detail)
        ))
    }
}

#[doc(hidden)]
pub struct MaterializedSlashSkill {
    pub content: String,
    pub tool_ceiling: crate::skills::SkillToolCeiling,
}

#[doc(hidden)]
pub fn require_explicit_mention_skill_activation(
    requested_names: &[String],
    activation: Option<crate::skills::MentionSkillActivation>,
) -> Result<crate::skills::MentionSkillActivation, String> {
    if requested_names.is_empty() {
        return Err("explicit @skill activation set is empty".to_string());
    }
    let activation = activation
        .ok_or_else(|| "explicit @skill resolver is unavailable; activation denied".to_string())?;
    let requested = requested_names
        .iter()
        .collect::<std::collections::HashSet<_>>();
    let resolved = activation
        .resolved_names
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if !activation.rejected_names.is_empty()
        || activation.content.trim().is_empty()
        || resolved != requested
    {
        return Err(
            "explicit @skill set could not be resolved and materialized atomically; activation denied"
                .to_string(),
        );
    }
    Ok(activation)
}

#[doc(hidden)]
pub fn require_explicit_slash_skill_materialization(
    entry: &crate::skills::SkillEntry,
    args: Option<String>,
    rendered: anyhow::Result<String>,
) -> Result<MaterializedSlashSkill, String> {
    let _args = args.ok_or_else(|| {
        format!(
            "Explicit slash skill '{}' arguments no longer match the validated command binding",
            entry.name
        )
    })?;
    let content = rendered.map_err(|error| {
        format!(
            "Explicit slash skill '{}' could not be materialized: {}",
            entry.name, error
        )
    })?;
    if content.trim().is_empty() {
        return Err(format!(
            "Explicit slash skill '{}' produced no model prompt",
            entry.name
        ));
    }
    Ok(MaterializedSlashSkill {
        content,
        tool_ceiling: entry.tool_ceiling(),
    })
}

#[doc(hidden)]
pub fn terminal_turn_state(
    db: &session::SessionDB,
    turn_id: Option<&str>,
) -> Option<(
    session::ChatTurnStatus,
    Option<session::ChatTurnInterruptReason>,
    Option<String>,
)> {
    let turn_id = turn_id?;
    db.get_chat_turn(turn_id)
        .ok()
        .flatten()
        .filter(|turn| turn.status.is_terminal())
        .map(|turn| (turn.status, turn.interrupt_reason, turn.error))
}

/// Consume an attached GUI / HTTP mirror and terminate its existing preview
/// through the channel-owned abort path. The engine never waits on remote IM
/// I/O: desktop completion remains independent, while the owned mirror state
/// guarantees the same Message / Card / Native identity is used for the
/// terminal mutation.
///
/// Returning the task makes the helper directly testable. Production callers
/// intentionally detach it because these paths never replay the logical turn.
#[doc(hidden)]
pub fn abort_im_mirror_in_background(
    im_mirror: &mut Option<Box<dyn crate::channel_hooks::ImLiveMirror>>,
    session_id: &str,
    reason: &TerminationReason,
) -> Option<tokio::task::JoinHandle<()>> {
    let state = im_mirror.take()?;
    let body = finalize::copy::im_notice(reason);
    let session_id = session_id.to_string();
    // Construct the channel-owned future before spawning. `abort` synchronously
    // detaches the session fan-out sink, so a subsequent turn cannot race its
    // first delta into this terminal generation while the task waits to poll.
    let abort = state.abort(Some(body));
    Some(tokio::spawn(async move {
        let status = abort.await;
        if !status.is_confirmed() {
            app_warn!(
                "channel",
                "mirror",
                "IM mirror abnormal terminal could not be confirmed for session {}",
                session_id
            );
        }
    }))
}

#[doc(hidden)]
pub fn abort_im_mirror_after_internal_error(
    im_mirror: &mut Option<Box<dyn crate::channel_hooks::ImLiveMirror>>,
    session_id: &str,
    message: &str,
) -> Option<tokio::task::JoinHandle<()>> {
    abort_im_mirror_in_background(
        im_mirror,
        session_id,
        &TerminationReason::Other {
            message: message.to_string(),
        },
    )
}

/// Consume a completed mirror, synchronously detach its stream sink while
/// constructing the channel future, then move only that detached future to the
/// background task.
#[doc(hidden)]
pub fn finalize_im_mirror_in_background(
    im_mirror: &mut Option<Box<dyn crate::channel_hooks::ImLiveMirror>>,
    response: String,
) -> Option<tokio::task::JoinHandle<()>> {
    let state = im_mirror.take()?;
    let finalize = state.finalize(response);
    Some(tokio::spawn(finalize))
}

/// Reconstruct the closest public termination taxonomy when another owner
/// (Stop watchdog / request guard) has already finalized `chat_turns` while
/// the provider future is unwinding. This keeps IM copy aligned with the GUI
/// event instead of collapsing every external terminal into an internal error.
#[doc(hidden)]
pub fn mirror_reason_from_terminal_state(
    status: session::ChatTurnStatus,
    interrupt: Option<session::ChatTurnInterruptReason>,
    error: Option<&str>,
) -> TerminationReason {
    let detail = error
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("turn was finalized by another runtime owner")
        .to_string();
    match interrupt {
        Some(session::ChatTurnInterruptReason::UserStop) => TerminationReason::UserStop,
        Some(
            session::ChatTurnInterruptReason::RuntimeCancel
            | session::ChatTurnInterruptReason::ToolCancel,
        ) => TerminationReason::RuntimeCancel,
        Some(session::ChatTurnInterruptReason::Shutdown) => TerminationReason::Shutdown,
        Some(session::ChatTurnInterruptReason::CrashRecovery) => TerminationReason::Crash,
        Some(session::ChatTurnInterruptReason::NoProfile) => TerminationReason::NoProfileAvailable,
        Some(session::ChatTurnInterruptReason::ProviderFailed) => {
            TerminationReason::ProviderFailed {
                last_kind: failover::classify_error(&detail),
                last_message: detail,
                // The terminal DB projection does not retain the exact
                // provider/API identity. Never guess the Codex-specific hint.
                is_codex_auth: false,
            }
        }
        Some(session::ChatTurnInterruptReason::CurrentToolGroupOverflow) => {
            TerminationReason::ProviderFailed {
                last_kind: failover::FailoverReason::CurrentToolGroupOverflow,
                last_message: detail,
                is_codex_auth: false,
            }
        }
        Some(session::ChatTurnInterruptReason::DispatchUnknown) => {
            TerminationReason::ProviderFailed {
                last_kind: failover::FailoverReason::DispatchUnknown,
                last_message: detail,
                is_codex_auth: false,
            }
        }
        Some(session::ChatTurnInterruptReason::CompactionFailed) => {
            TerminationReason::CompactionFailed { detail }
        }
        Some(session::ChatTurnInterruptReason::Unknown) | None => TerminationReason::Other {
            message: format!("turn ended with status {}: {detail}", status.as_str()),
        },
    }
}

#[doc(hidden)]
pub fn turn_accepts_stream_event(
    db: &session::SessionDB,
    session_id: &str,
    turn_id: Option<&str>,
) -> bool {
    let Some(turn_id) = turn_id else {
        return true;
    };
    // Hot path: `is_accepting` reads the registry without cloning the
    // 3-String + Arc snapshot that `current` allocates per call.
    match super::active_turn::is_accepting(session_id, turn_id) {
        Some(accepting) => accepting,
        // No entry for *this* turn. Preserve the original semantics: if some
        // other turn is live for this session, reject without a DB probe;
        // only a fully-absent entry falls back to the terminal-state probe.
        None if super::active_turn::has_entry(session_id) => false,
        None => terminal_turn_state(db, Some(turn_id)).is_none(),
    }
}

/// Successful chat round payload returned by the executor closure.
/// Bundles everything the post-success path needs to flush thinking, build
/// the assistant message, save context, and run extraction follow-ups.
#[doc(hidden)]
pub struct ChatRoundOk {
    pub response: String,
    pub thinking: Option<String>,
    pub agent: AssistantAgent,
    pub history_len_before: usize,
    pub chat_start: std::time::Instant,
}

/// The mutable Agent cache is a runtime optimization, never part of the turn
/// contract. Keeping it behind the engine boundary prevents shells from
/// receiving a concrete `AssistantAgent` that could call the model directly.
#[doc(hidden)]
pub async fn retain_desktop_agent(source: stream_seq::ChatSource, agent: AssistantAgent) {
    if source != stream_seq::ChatSource::Desktop {
        return;
    }
    if let Some(cache) = crate::get_cached_agent() {
        *cache.lock().await = Some(agent);
    }
}

/// Drop-guarded scope for a session's visible stream lifecycle. Ensures
/// `stream_seq::end` fires on every admitted runtime return path (including
/// panics), while allowing the successful path to end the UI stream before
/// post-turn follow-ups run. Desktop / HTTP / parent-injection turns broadcast
/// on the main `chat:*` bus; IM channel turns have a separate `channel:*`
/// lifecycle.
#[doc(hidden)]
pub struct StreamLifecycle {
    session_id: String,
    pub stream_id: Option<String>,
    source: stream_seq::ChatSource,
    turn_id: Option<String>,
    terminal_status: Option<session::ChatTurnStatus>,
    interrupt_reason: Option<session::ChatTurnInterruptReason>,
    terminal_error: Option<String>,
    abandoned_recovery: Option<(std::sync::Arc<session::SessionDB>, String)>,
    finished: bool,
}

impl StreamLifecycle {
    pub fn begin(
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
            abandoned_recovery: None,
            finished: false,
        })
    }

    pub fn from_admission(
        session_id: &str,
        source: stream_seq::ChatSource,
        turn_id: Option<String>,
        stream_id: String,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            stream_id: Some(stream_id),
            source,
            turn_id,
            terminal_status: None,
            interrupt_reason: None,
            terminal_error: None,
            abandoned_recovery: None,
            finished: false,
        }
    }

    pub fn arm_abandoned_recovery(
        &mut self,
        db: std::sync::Arc<session::SessionDB>,
        persistence_run_id: String,
    ) {
        self.abandoned_recovery = Some((db, persistence_run_id));
    }

    pub fn set_terminal(
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

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        if let Some(ref stream_id) = self.stream_id {
            let released = stream_seq::end_if_stream(&self.session_id, stream_id);
            if !released {
                if let Some(ref turn_id) = self.turn_id {
                    super::turn_injection::clear_turn(&self.session_id, turn_id);
                }
                self.finished = true;
                return;
            }
            // A dropped/panicked engine future has no committed terminal fact
            // yet. Its Drop path schedules journal convergence below; emitting
            // an unqualified end here would let the UI outrun persistence.
            if self.source.broadcasts_to_user_ui() && self.terminal_status.is_some() {
                stream_broadcast::broadcast_stream_end(
                    &self.session_id,
                    Some(stream_id),
                    self.turn_id.as_deref(),
                    self.terminal_status,
                    self.interrupt_reason,
                    self.terminal_error.as_deref(),
                );
            }
        }
        if let Some(ref turn_id) = self.turn_id {
            super::turn_injection::clear_turn(&self.session_id, turn_id);
        }
        self.finished = true;
    }
}

impl Drop for StreamLifecycle {
    fn drop(&mut self) {
        if !self.finished && self.terminal_status.is_none() {
            if let Some((db, persistence_run_id)) = self.abandoned_recovery.take() {
                super::spawn_abandoned_stream_recovery(
                    db,
                    self.session_id.clone(),
                    self.turn_id.clone(),
                    self.source,
                    persistence_run_id,
                );
            }
        }
        self.finish();
    }
}

/// Emit one stream event. Desktop / HTTP turns send through both the per-call
/// sink and the main `chat:stream_delta` EventBus path with a shared `_oc_seq`
/// for dedup. Parent-injection turns use the same bus so background-completion
/// follow-up replies are visible while they stream. Channel / cron turns stay
/// off the main chat bus; IM uses `ChannelStreamSink` to emit
/// `channel:stream_delta` instead.
#[doc(hidden)]
pub fn emit_stream_event(
    db: &session::SessionDB,
    event_sink: &std::sync::Arc<dyn EventSink>,
    session_id: &str,
    source: stream_seq::ChatSource,
    turn_id: Option<&str>,
    event: &str,
) -> bool {
    if !turn_accepts_stream_event(db, session_id, turn_id) {
        return false;
    }
    emit_stream_event_unchecked(event_sink, session_id, source, turn_id, event);
    true
}

#[doc(hidden)]
pub fn emit_context_compaction_progress(
    db: &session::SessionDB,
    event_sink: &std::sync::Arc<dyn EventSink>,
    session_id: &str,
    source: stream_seq::ChatSource,
    turn_id: Option<&str>,
    phase: &str,
    kind: &str,
    extra: Option<serde_json::Map<String, serde_json::Value>>,
) -> bool {
    let mut data = serde_json::Map::new();
    data.insert("phase".to_string(), serde_json::json!(phase));
    data.insert("kind".to_string(), serde_json::json!(kind));
    if let Some(extra) = extra {
        for (key, value) in extra {
            data.insert(key, value);
        }
    }
    let Ok(event) = serde_json::to_string(&serde_json::json!({
        "type": "context_compaction_progress",
        "data": data,
    })) else {
        return false;
    };
    emit_stream_event(db, event_sink, session_id, source, turn_id, &event)
}

fn persist_manual_context_compaction_event(
    db: &session::SessionDB,
    session_id: &str,
    source: stream_seq::ChatSource,
    event: &str,
) {
    let _ = db.append_message(
        session_id,
        &session::NewMessage::event(event).with_source(source),
    );
}

fn persist_manual_context_compaction_failed(
    db: &session::SessionDB,
    session_id: &str,
    source: stream_seq::ChatSource,
) {
    let Ok(event) = serde_json::to_string(&serde_json::json!({
        "type": "context_compaction_progress",
        "data": {
            "phase": "failed",
            "kind": "summary",
        },
    })) else {
        return;
    };
    persist_manual_context_compaction_event(db, session_id, source, &event);
}

fn persist_manual_context_compacted(
    db: &session::SessionDB,
    session_id: &str,
    source: stream_seq::ChatSource,
    result: &crate::context_compact::CompactResult,
) {
    let kind = if result.tier_applied >= 4 {
        "emergency"
    } else {
        "summary"
    };
    let Ok(event) = serde_json::to_string(&serde_json::json!({
        "type": "context_compacted",
        "data": {
            "tier_applied": result.tier_applied,
            "tokens_before": result.tokens_before,
            "tokens_after": result.tokens_after,
            "messages_affected": result.messages_affected,
            "description": &result.description,
            "kind": kind,
            "manifest": &result.manifest,
        },
    })) else {
        return;
    };
    persist_manual_context_compaction_event(db, session_id, source, &event);
}

/// Emit a stream event when the caller has *already* confirmed the turn
/// accepts events this tick. The per-token streaming hot loop calls this after
/// its own `turn_accepts_stream_event` guard, avoiding a second registry lock
/// + snapshot clone per token.
#[doc(hidden)]
pub fn emit_stream_event_unchecked(
    event_sink: &std::sync::Arc<dyn EventSink>,
    session_id: &str,
    source: stream_seq::ChatSource,
    turn_id: Option<&str>,
    event: &str,
) {
    if let Some(coordinator) = super::durability::active(session_id) {
        if let Err(error) = coordinator.accept_event(event) {
            app_error!(
                "chat",
                "stream_durability",
                "failed to accept stream event for {}: {}",
                session_id,
                error
            );
        }
        return;
    }
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

/// Run a user-requested context compaction for a stored session.
///
/// HTTP/server mode uses this path so manual compaction restores persisted
/// context, bypasses cache throttles, forces Tier 3 summarization when
/// possible, saves the compacted history, and emits the same compaction events
/// as the chat engine.
pub async fn compact_session_now(
    params: CompactSessionParams,
) -> Result<CompactSessionResult, String> {
    let CompactSessionParams {
        session_id,
        agent_id,
        session_db,
        model,
        providers,
        codex_token,
        resolved_temperature,
        compact_config,
        source,
        event_sink,
    } = params;

    let persist_failed = |message: String| {
        persist_manual_context_compaction_failed(&session_db, &session_id, source);
        message
    };

    let _active_turn_guard = super::active_turn::try_acquire(
        &session_id,
        source,
        format!("manual-compact-{}", uuid::Uuid::new_v4()),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .map_err(|e| persist_failed(e.to_string()))?;

    let provider = providers
        .iter()
        .find(|p| p.id == model.provider_id)
        .ok_or_else(|| persist_failed(format!("Provider {} not found", model.provider_id)))?;
    let provider_label = provider.name.clone();

    let mut codex_token = codex_token;
    if provider.api_type == ApiType::Codex {
        let current = codex_token.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
        if let Some(pair) = crate::oauth::ensure_fresh_codex_token(current).await {
            codex_token = Some(pair);
        }
    }

    let mut agent = build_agent_from_snapshot(
        &model,
        &providers,
        codex_token,
        &compact_config,
        None,
        &session_id,
    )
    .await
    .map_err(|e| {
        persist_failed(format!(
            "Cannot build agent for manual compaction on {}::{}: {}",
            model.provider_id, model.model_id, e
        ))
    })?;

    let plan_resolved = crate::chat_engine::resolve_plan_context_for_session(&session_id).await;
    configure_agent(
        &mut agent,
        &agent_id,
        &session_id,
        None,
        session_db.clone(),
        resolved_temperature,
        None,
        &[],
        &[],
        &[],
        &[],
        None,
        0,
        None,
        plan_resolved,
        false,
        false,
        true,
        source,
        kb_access_source(source),
        None,
        None,
    );
    let original_context_json = session_db
        .load_context(&session_id)
        .map_err(|e| persist_failed(format!("Cannot load context for manual compaction: {e}")))?;
    if let Some(json_str) = original_context_json.as_deref() {
        restore_agent_context_from_json(&session_id, json_str, &agent);
    }

    let emit = |delta: &str| {
        let _ = emit_stream_event(&session_db, &event_sink, &session_id, source, None, delta);
    };
    let compact_result = agent.compact_conversation_now(&emit).await;
    let summary_applied =
        compact_result.tier_applied >= 3 && compact_result.description == "summarized";

    let compacted_context_json = serde_json::to_string(&agent.get_conversation_history())
        .map_err(|e| persist_failed(format!("Cannot serialize compacted context: {e}")))?;
    if original_context_json.as_deref() != Some(compacted_context_json.as_str()) {
        let saved = if summary_applied {
            session_db.save_context_if_unchanged_and_clear_tier3_recovery(
                &session_id,
                original_context_json.as_deref(),
                &compacted_context_json,
            )
        } else {
            session_db.save_context_if_unchanged(
                &session_id,
                original_context_json.as_deref(),
                &compacted_context_json,
            )
        }
        .map_err(|e| persist_failed(format!("Cannot save compacted context: {e}")))?;
        if !saved {
            return Err(persist_failed(
                "Session context changed during manual compaction; skipped stale compacted snapshot"
                    .to_string(),
            ));
        }
    } else if summary_applied {
        session_db
            .clear_tier3_recovery_after_summary(&session_id)
            .map_err(|e| persist_failed(format!("Cannot finalize compaction recovery: {e}")))?;
    }
    if summary_applied && crate::session::is_session_incognito(Some(&session_id)) {
        crate::session::clear_incognito_tier3_recovery(&session_id);
    }
    persist_manual_context_compacted(&session_db, &session_id, source, &compact_result);
    app_info!(
        "context",
        "compact::manual",
        "Manual compaction: provider={}, tier={}, {} → {} tokens, {} affected",
        provider_label,
        compact_result.tier_applied,
        compact_result.tokens_before,
        compact_result.tokens_after,
        compact_result.messages_affected
    );

    Ok(CompactSessionResult {
        compact_result,
        agent,
    })
}

// ── Core Chat Engine ────────────────────────────────────────────────

#[doc(hidden)]
pub fn merge_explicit_skill_ceiling(
    current: &mut Vec<String>,
    selected: crate::skills::SkillToolCeiling,
) {
    crate::skills::narrow_skill_execution_filter(current, selected);
}

/// Reserve at most one fifth of the primary model's context for explicit
/// typed notes and split it fairly across the selected set. Small notes are
/// therefore injected completely; larger notes get a deterministic preview
/// plus a version-checked `note_read` continuation. This is a byte upper bound
/// used before provider-exact token accounting, so keep a conservative floor
/// and cap.
#[doc(hidden)]
pub fn typed_note_byte_budget(
    model_chain: &[ActiveModel],
    providers: &[ProviderConfig],
    note_count: usize,
) -> usize {
    const MIN_PER_NOTE: usize = 8 * 1024;
    const MAX_PER_NOTE: usize = 200_000;
    let context_tokens = model_chain
        .first()
        .and_then(|model| {
            crate::provider::model_context_window(providers, &model.provider_id, &model.model_id)
        })
        .unwrap_or(128_000) as usize;
    // ~4 UTF-8 bytes/token conservative planning estimate, with 20% of the
    // window available to all explicit notes.
    let total_note_bytes = context_tokens.saturating_mul(4) / 5;
    (total_note_bytes / note_count.max(1)).clamp(MIN_PER_NOTE, MAX_PER_NOTE)
}

/// Remove only the source spans already represented by typed Note bindings so
/// the read-only `[[note]]` compatibility scanner can coexist with other typed
/// mentions without resolving a typed Note twice. The wire has already passed
/// UTF-8 boundary validation before this helper is called.
#[doc(hidden)]
pub fn message_without_typed_note_spans(
    message: &str,
    wire: &crate::prompt_context::IncomingTurnWire,
) -> String {
    let mut spans = wire
        .mentions
        .iter()
        .filter(|mention| mention.kind == crate::prompt_context::MentionKind::Note)
        .filter_map(|mention| match &mention.source_anchor {
            crate::prompt_context::SourceAnchor::Inline {
                start_utf8,
                end_utf8,
                ..
            } => Some((*start_utf8 as usize, *end_utf8 as usize)),
            crate::prompt_context::SourceAnchor::AdjacentContentPart { .. } => None,
        })
        .collect::<Vec<_>>();
    if spans.is_empty() {
        return message.to_string();
    }
    spans.sort_unstable();
    let mut result = String::with_capacity(message.len());
    let mut cursor = 0usize;
    for (start, end) in spans {
        let (Some(prefix), true) = (message.get(cursor..start), end <= message.len()) else {
            return message.to_string();
        };
        result.push_str(prefix);
        result.push(' ');
        cursor = end;
    }
    let Some(suffix) = message.get(cursor..) else {
        return message.to_string();
    };
    result.push_str(suffix);
    result
}

#[doc(hidden)]
pub fn validate_engine_typed_resource_boundary(
    message: &str,
    incoming_turn: Option<&crate::prompt_context::IncomingTurnWire>,
    attachments: &[crate::agent::Attachment],
) -> Result<(), String> {
    crate::attachments::validate_typed_resource_attachment_bindings(
        message,
        incoming_turn,
        attachments,
    )
    .map_err(|error| format!("Invalid typed resource attachment binding: {error}"))
}

#[doc(hidden)]
pub fn prepare_typed_resource_mentions_for_session(
    session: &crate::session::SessionMeta,
    file_targets: &[String],
    plan_targets: &[String],
    attachments: &[crate::agent::Attachment],
) -> anyhow::Result<crate::attachments::PreparedTypedResourceMentions> {
    let working_dir = crate::session::effective_working_dir_for_meta(session);
    crate::attachments::prepare_typed_resource_mentions(
        working_dir.as_deref(),
        file_targets,
        plan_targets,
        session.incognito,
        attachments,
    )
}

/// Map a transport/runtime source to the knowledge-access source used by the
/// kernel policy. Manual compaction and the feature-owned turn runtime share
/// this mapping so authorization semantics cannot drift.
pub fn kb_access_source(source: stream_seq::ChatSource) -> crate::knowledge::KbAccessSource {
    use crate::knowledge::KbAccessSource;
    use stream_seq::ChatSource;
    match source {
        ChatSource::Desktop => KbAccessSource::Gui,
        ChatSource::Http => KbAccessSource::Http,
        ChatSource::Channel => KbAccessSource::Im,
        ChatSource::Subagent => KbAccessSource::Subagent,
        ChatSource::ParentInjection | ChatSource::SessionTool => KbAccessSource::Other,
        ChatSource::Cron => KbAccessSource::Cron,
        ChatSource::Eval | ChatSource::Acp => KbAccessSource::Other,
    }
}

/// Convert a turn source into immutable tool-execution provenance.
pub fn tool_turn_provenance(
    source: stream_seq::ChatSource,
) -> crate::tool_defs::ToolTurnProvenance {
    if source.carries_foreground_user_intent() {
        crate::tool_defs::ToolTurnProvenance::ForegroundUser
    } else {
        crate::tool_defs::ToolTurnProvenance::Autonomous
    }
}

/// Run the shared chat execution engine with a typed terminal failure.
///
/// Handles: model chain traversal → agent building → config → history restoration
/// → streaming execution → tool persistence → failover → context compaction
/// → response saving → context persistence → memory extraction.
#[cfg(test)]
use crate::test_agent_runtime::streaming_loop;

#[cfg(test)]
#[path = "../../../ha-agent-runtime/src/engine.rs"]
mod test_runtime_engine;

#[cfg(test)]
use test_runtime_engine::*;

#[cfg(test)]
pub(super) async fn execute_admitted_turn_for_test(
    turn: crate::turn_kernel::AdmittedTurn,
) -> Result<crate::turn_kernel::AgentTurnOutput, TurnFailure> {
    let result = execute_admitted_params(turn.into_runtime_params()).await?;
    Ok(crate::turn_kernel::AgentTurnOutput {
        response: result.response,
        model_used: result.model_used,
        usage: result.usage,
        terminal: result.terminal,
    })
}

pub fn configure_agent(
    agent: &mut crate::agent::AssistantAgent,
    agent_id: &str,
    session_id: &str,
    turn_id: Option<&str>,
    session_db: Arc<session::SessionDB>,
    temperature: Option<f64>,
    run_context: Option<&crate::prompt_context::RunInstructionContext>,
    agent_binding_refs: &[crate::prompt_context::AgentBindingRef],
    context_resource_refs: &[crate::prompt_context::ContextResourceRef],
    skill_allowed_tools: &[String],
    denied_tools: &[String],
    tool_scope: Option<crate::tool_defs::ToolScope>,
    subagent_depth: u32,
    steer_run_id: Option<String>,
    plan_resolved: crate::agent::PlanResolvedContext,
    plan_locked: bool,
    auto_approve_tools: bool,
    follow_global_reasoning_effort: bool,
    source: stream_seq::ChatSource,
    kb_origin: crate::knowledge::KbAccessSource,
    turn_stop_admission: Option<(u64, u64, u64)>,
    channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
) {
    agent.set_agent_id(agent_id);
    agent.set_session_db(session_db);
    agent.set_session_id(session_id);
    agent.set_turn_id(turn_id.map(str::to_string));
    agent.set_chat_source(kb_access_source(source));
    agent.set_origin_chat_source(kb_origin);
    agent.set_turn_provenance(tool_turn_provenance(source));
    if let Some((lineage_epoch, global_stop_epoch, global_stop_receipt_count)) = turn_stop_admission
    {
        agent.set_turn_stop_admission(lineage_epoch, global_stop_epoch, global_stop_receipt_count);
    }
    agent.set_channel_kb_context(channel_kb_context);
    agent.set_temperature(temperature);
    if let Some(ctx) = run_context {
        agent.set_run_context(ctx.clone());
    }
    agent.set_agent_binding_refs(agent_binding_refs.to_vec());
    agent.set_context_resource_refs(context_resource_refs.to_vec());
    if !skill_allowed_tools.is_empty() {
        agent.set_skill_allowed_tools(skill_allowed_tools.to_vec());
    }
    if !denied_tools.is_empty() {
        agent.set_denied_tools(denied_tools.to_vec());
    }
    agent.set_tool_scope(tool_scope);
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
async fn run_chat_engine(params: ChatEngineParams) -> Result<ChatEngineResult, String> {
    execute_admitted_params(params)
        .await
        .map_err(|failure| failure.to_string())
}

#[cfg(test)]
mod stream_lifecycle_tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::context_compact::CompactConfig;
    use crate::provider::{ActiveModel, ApiType, ModelConfig, ProviderConfig};
    use crate::session::{MessageRole, NewMessage, SessionDB};
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn engine_defense_rejects_forged_typed_attachment_without_sidecar() {
        let attachment = crate::agent::Attachment {
            name: "forged.txt".into(),
            mime_type: "text/plain".into(),
            source: Some("mention".into()),
            data: None,
            file_path: Some("/tmp/forged.txt".into()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_project_root: None,
            quote_worktree_root: None,
            quote_role: None,
        };
        let error = validate_engine_typed_resource_boundary("plain", None, &[attachment])
            .expect_err("engine must not trust a client-controlled attachment source marker");
        assert!(error.contains("exactly match"));
    }

    #[test]
    fn typed_resource_freeze_uses_project_working_dir_when_session_override_is_null() {
        let data_root = tempfile::tempdir().expect("data root");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", data_root.path())], || {
            let project_id = format!("typed-resource-{}", uuid::Uuid::new_v4());
            let project_workspace = crate::paths::project_workspace_dir(&project_id)
                .expect("resolve default project workspace");
            std::fs::create_dir_all(&project_workspace).expect("create project workspace");
            let dockerfile = project_workspace.join("Dockerfile");
            std::fs::write(&dockerfile, b"FROM scratch\n").expect("write Dockerfile");

            let db = SessionDB::open(&data_root.path().join("engine-project-session.db"))
                .expect("open session db");
            let session = db
                .create_session_with_project("ha-main", Some(&project_id), None)
                .expect("create project session");
            assert_eq!(
                session.working_dir, None,
                "fixture must inherit from its project"
            );

            let attachment = crate::agent::Attachment {
                name: "Dockerfile".into(),
                mime_type: "text/plain".into(),
                source: Some("mention".into()),
                data: None,
                file_path: Some(dockerfile.to_string_lossy().into_owned()),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_project_root: None,
                quote_worktree_root: None,
                quote_role: None,
            };
            prepare_typed_resource_mentions_for_session(
                &session,
                &["Dockerfile".into()],
                &[],
                &[attachment],
            )
            .expect("project-inherited working dir should authorize the selected root file");
        });
    }

    struct RecordingImMirror {
        detached: Arc<AtomicBool>,
        finalized: Arc<AtomicBool>,
        aborted_bodies: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl crate::channel_hooks::ImLiveMirror for RecordingImMirror {
        fn finalize(
            self: Box<Self>,
            _response: String,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
            self.detached.store(true, Ordering::SeqCst);
            let finalized = self.finalized.clone();
            Box::pin(async move {
                finalized.store(true, Ordering::SeqCst);
            })
        }

        fn abort(
            self: Box<Self>,
            body: Option<String>,
        ) -> Pin<
            Box<
                dyn Future<Output = crate::channel_hooks::ImLiveMirrorAbortStatus> + Send + 'static,
            >,
        > {
            self.detached.store(true, Ordering::SeqCst);
            let aborted_bodies = self.aborted_bodies.clone();
            Box::pin(async move {
                aborted_bodies
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(body);
                crate::channel_hooks::ImLiveMirrorAbortStatus::Confirmed
            })
        }
    }

    #[tokio::test]
    async fn abnormal_terminal_consumes_live_mirror_through_abort_once() {
        let detached = Arc::new(AtomicBool::new(false));
        let finalized = Arc::new(AtomicBool::new(false));
        let aborted_bodies = Arc::new(Mutex::new(Vec::new()));
        let mut mirror: Option<Box<dyn crate::channel_hooks::ImLiveMirror>> =
            Some(Box::new(RecordingImMirror {
                detached: detached.clone(),
                finalized: finalized.clone(),
                aborted_bodies: aborted_bodies.clone(),
            }));
        let reason = TerminationReason::NoProfileAvailable;

        let task = abort_im_mirror_in_background(&mut mirror, "mirror-test", &reason)
            .expect("attached mirror should spawn an abort task");
        assert!(
            mirror.is_none(),
            "terminal owner must be consumed immediately"
        );
        assert!(
            detached.load(Ordering::SeqCst),
            "abort must detach before the spawned future is polled"
        );
        task.await.expect("abort task should complete");

        assert!(!finalized.load(Ordering::SeqCst));
        assert_eq!(
            *aborted_bodies
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![Some(finalize::copy::im_notice(&reason))]
        );
        assert!(abort_im_mirror_in_background(&mut mirror, "mirror-test", &reason).is_none());
    }

    #[tokio::test]
    async fn completed_terminal_detaches_before_background_poll() {
        let detached = Arc::new(AtomicBool::new(false));
        let finalized = Arc::new(AtomicBool::new(false));
        let mut mirror: Option<Box<dyn crate::channel_hooks::ImLiveMirror>> =
            Some(Box::new(RecordingImMirror {
                detached: detached.clone(),
                finalized: finalized.clone(),
                aborted_bodies: Arc::new(Mutex::new(Vec::new())),
            }));

        let task = finalize_im_mirror_in_background(&mut mirror, "done".to_string())
            .expect("attached mirror should spawn a finalize task");
        assert!(mirror.is_none());
        assert!(
            detached.load(Ordering::SeqCst),
            "finalize must detach before the spawned future is polled"
        );
        task.await.expect("finalize task should complete");
        assert!(finalized.load(Ordering::SeqCst));
    }

    #[test]
    fn external_terminal_reason_preserves_user_stop_and_provider_failure() {
        assert!(matches!(
            mirror_reason_from_terminal_state(
                session::ChatTurnStatus::Interrupted,
                Some(session::ChatTurnInterruptReason::UserStop),
                None,
            ),
            TerminationReason::UserStop
        ));

        let provider = mirror_reason_from_terminal_state(
            session::ChatTurnStatus::Failed,
            Some(session::ChatTurnInterruptReason::ProviderFailed),
            Some("429 rate limit"),
        );
        assert!(matches!(
            provider,
            TerminationReason::ProviderFailed {
                last_kind: failover::FailoverReason::RateLimit,
                is_codex_auth: false,
                ..
            }
        ));
    }

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

    #[test]
    fn whole_chain_retry_is_bounded_to_uncertain_pre_tool_failures() {
        assert!(should_retry_model_chain(
            1,
            2,
            Some(failover::FailoverReason::Timeout),
            false,
            false,
            false,
        ));
        assert!(should_retry_model_chain(
            1,
            2,
            Some(failover::FailoverReason::Unknown),
            false,
            false,
            false,
        ));
        assert!(!should_retry_model_chain(
            1,
            2,
            Some(failover::FailoverReason::Auth),
            false,
            false,
            false,
        ));
        assert!(!should_retry_model_chain(
            1,
            2,
            Some(failover::FailoverReason::Unknown),
            false,
            false,
            true,
        ));
        assert!(!should_retry_model_chain(
            2,
            2,
            Some(failover::FailoverReason::Timeout),
            false,
            false,
            false,
        ));
    }

    #[test]
    fn missing_final_provider_preserves_an_uncertain_chain_failure() {
        let reason = chain_reason_after_missing_provider(Some(failover::FailoverReason::Timeout));
        assert_eq!(reason, failover::FailoverReason::Timeout);
        assert!(should_retry_model_chain(
            1,
            2,
            Some(reason),
            false,
            false,
            false,
        ));

        assert_eq!(
            chain_reason_after_missing_provider(None),
            failover::FailoverReason::ModelNotFound
        );
    }

    #[test]
    fn switch_model_requires_a_remaining_resolvable_provider() {
        let current_provider = openai_provider("http://current.invalid".to_string(), "m1");
        let fallback_provider = openai_provider("http://fallback.invalid".to_string(), "m3");
        let chain = vec![
            ActiveModel {
                provider_id: current_provider.id.clone(),
                model_id: "m1".to_string(),
            },
            ActiveModel {
                provider_id: "deleted-provider".to_string(),
                model_id: "m2".to_string(),
            },
            ActiveModel {
                provider_id: fallback_provider.id.clone(),
                model_id: "m3".to_string(),
            },
        ];

        assert!(!has_resolvable_fallback(
            &chain[..2],
            std::slice::from_ref(&current_provider),
            0,
        ));
        assert!(has_resolvable_fallback(
            &chain,
            &[current_provider, fallback_provider],
            0,
        ));
        assert!(!has_resolvable_fallback(&chain, &[], 2));
    }

    #[test]
    fn slash_skill_binding_uses_collision_resolved_command_ownership() {
        let fixture = |name: &str, aliases: &[&str]| {
            serde_json::from_value::<crate::skills::SkillEntry>(serde_json::json!({
                "name": name,
                "aliases": aliases,
                "description": "test",
                "source": "test",
                "file_path": "/tmp/SKILL.md",
                "base_dir": "/tmp"
            }))
            .expect("skill fixture")
        };
        let entries = vec![
            fixture("new", &["shared-alias"]),
            fixture("other-skill", &["shared-alias"]),
        ];

        assert!(resolve_slash_skill_binding(&entries, "new", "new_skill").is_some());
        assert!(
            resolve_slash_skill_binding(&entries, "new", "new").is_none(),
            "the built-in command owns the unsuffixed name"
        );
        assert!(
            resolve_slash_skill_binding(&entries, "new", "shared_alias").is_some(),
            "the first skill owns a shared alias"
        );
        assert!(
            resolve_slash_skill_binding(&entries, "other-skill", "shared_alias").is_none(),
            "a later skill cannot claim an alias dropped by collision resolution"
        );
        assert!(resolve_slash_skill_binding(&entries, "other-skill", "other_skill").is_some());

        let mut blocked = fixture("collision-name", &[]);
        blocked.requires.os = vec!["definitely-unsupported-os".to_string()];
        let available = fixture("available", &["collision-name"]);
        let surfaced = crate::skills::filter_catalog_eligible_skills(
            vec![blocked, available],
            true,
            &std::collections::HashMap::new(),
        );
        assert!(
            resolve_slash_skill_binding(&surfaced, "available", "collision_name").is_some(),
            "hard-blocked skills cannot reserve a command hidden from list/help"
        );
    }

    #[test]
    fn explicit_slash_skill_requirement_drift_fails_before_provider_dispatch() {
        let mut entry = serde_json::from_value::<crate::skills::SkillEntry>(serde_json::json!({
            "name": "restricted-review",
            "description": "test",
            "source": "test",
            "file_path": "/tmp/restricted-review/SKILL.md",
            "base_dir": "/tmp/restricted-review",
            "allowed_tools": ["read"],
            "allowed_tools_declared": true
        }))
        .expect("skill fixture");
        entry.requires.os = vec!["definitely-unsupported-os".to_string()];

        let error = ensure_explicit_slash_skill_requirements(
            &entry,
            true,
            &std::collections::HashMap::new(),
        )
        .expect_err("a requirement race must reject the explicit slash turn");

        assert!(error.contains("no longer eligible"));
    }

    #[test]
    fn explicit_slash_skill_read_failure_has_no_unrestricted_activation() {
        let entry = serde_json::from_value::<crate::skills::SkillEntry>(serde_json::json!({
            "name": "restricted-review",
            "description": "test",
            "source": "test",
            "file_path": "/missing/restricted-review/SKILL.md",
            "base_dir": "/missing/restricted-review",
            "allowed_tools": ["read"],
            "allowed_tools_declared": true
        }))
        .expect("skill fixture");
        let temp = tempfile::tempdir().expect("tempdir");
        let read_error = std::fs::read_to_string(temp.path().join("missing-SKILL.md"))
            .map_err(anyhow::Error::from);

        let error =
            require_explicit_slash_skill_materialization(&entry, Some(String::new()), read_error)
                .err()
                .expect("a missing SKILL.md must stop before provider dispatch");

        assert!(error.contains("could not be materialized"));

        let activation = require_explicit_slash_skill_materialization(
            &entry,
            Some(String::new()),
            Ok("restricted body".to_string()),
        )
        .expect("successful materialization");
        assert_eq!(
            activation.tool_ceiling,
            crate::skills::SkillToolCeiling::Restricted(vec!["read".to_string()]),
            "the body and execution ceiling must leave materialization together"
        );
    }

    #[test]
    fn explicit_slash_skill_argument_mismatch_fails_before_provider_dispatch() {
        let entry = serde_json::from_value::<crate::skills::SkillEntry>(serde_json::json!({
            "name": "review",
            "description": "test",
            "source": "test",
            "file_path": "/tmp/review/SKILL.md",
            "base_dir": "/tmp/review"
        }))
        .expect("skill fixture");

        let error = require_explicit_slash_skill_materialization(
            &entry,
            None,
            Ok("must not be sent".to_string()),
        )
        .err()
        .expect("invalid canonical args must stop the explicit slash turn");

        assert!(error.contains("arguments no longer match"));
    }

    #[test]
    fn explicit_at_skill_rejection_cannot_continue_as_unrestricted_turn() {
        let requested = vec!["restricted-review".to_string()];
        let rejection = crate::skills::MentionSkillActivation {
            content: "# Skill activation rejected".to_string(),
            resolved_names: Vec::new(),
            rejected_names: requested.clone(),
            tool_ceiling: crate::skills::SkillToolCeiling::Unspecified,
        };

        let error = require_explicit_mention_skill_activation(&requested, Some(rejection))
            .expect_err("a rejected typed @skill must stop before provider dispatch");

        assert!(error.contains("activation denied"));
    }

    #[test]
    fn explicit_at_skill_unwired_resolver_fails_closed() {
        let requested = vec!["restricted-review".to_string()];
        let error = require_explicit_mention_skill_activation(&requested, None)
            .expect_err("missing skill machinery must stop the typed turn");

        assert!(error.contains("resolver is unavailable"));
    }

    #[test]
    fn explicit_at_skill_content_and_ceiling_are_accepted_as_one_atomic_set() {
        let requested = vec!["restricted-review".to_string()];
        let activation = crate::skills::MentionSkillActivation {
            content: "restricted body".to_string(),
            resolved_names: requested.clone(),
            rejected_names: Vec::new(),
            tool_ceiling: crate::skills::SkillToolCeiling::Restricted(vec!["read".to_string()]),
        };

        let accepted = require_explicit_mention_skill_activation(&requested, Some(activation))
            .expect("complete typed @skill set");
        assert_eq!(
            accepted.tool_ceiling,
            crate::skills::SkillToolCeiling::Restricted(vec!["read".to_string()])
        );
    }

    fn temp_db() -> (std::sync::MutexGuard<'static, ()>, TempDir, Arc<SessionDB>) {
        let lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let db = Arc::new(SessionDB::open_ephemeral_for_test(&path).unwrap());
        (lock, dir, db)
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
            cost_input: Some(0.0),
            cost_output: Some(0.0),
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

    fn sse_two_text_then_done(first: &str, second: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            first, second
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
            pre_admitted_stream: None,
            active_turn_guard: None,
            ui_surface: None,
            message: "hello".to_string(),
            incoming_turn: None,
            display_text: None,
            attachments: Vec::new(),
            session_db: db,
            model_chain,
            providers,
            config_revision: [0; 32],
            codex_token: None,
            resolved_temperature: None,
            compact_config: CompactConfig::default(),
            run_context: None,
            reasoning_effort: Some("none".to_string()),
            cancel: Arc::new(AtomicBool::new(false)),
            foreground_stop_admission: None,
            plan_context_override: Some(crate::agent::PlanResolvedContext::off()),
            skill_allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            tool_scope: None,
            subagent_depth: 0,
            steer_run_id: None,
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            post_turn_effects: false,
            abort_on_cancel: false,
            persist_final_error_event: true,
            source: stream_seq::ChatSource::Desktop,
            origin_source: None,
            channel_kb_context: None,
            event_sink: Arc::new(NoopEventSink),
        }
    }

    fn create_user_turn(db: &SessionDB, session_id: &str) -> String {
        let user_id = db
            .append_message(session_id, &NewMessage::user("hello"))
            .unwrap();
        let turn_id = uuid::Uuid::new_v4().to_string();
        db.create_chat_turn_with_id(
            &turn_id,
            session_id,
            stream_seq::ChatSource::Desktop.as_str(),
            None,
            Some(user_id),
        )
        .unwrap();
        turn_id
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

    #[derive(Default)]
    struct RecordingSink {
        events: std::sync::Mutex<Vec<String>>,
    }

    impl EventSink for RecordingSink {
        fn send(&self, event: &str) {
            self.events.lock().unwrap().push(event.to_string());
        }
    }

    #[test]
    fn stream_events_stop_after_cancel_or_terminal_turn() {
        let (_lock, _dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let turn_id = create_user_turn(&db, &session.id);
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            stream_seq::ChatSource::Desktop,
            turn_id.clone(),
            cancel.clone(),
        )
        .unwrap();
        let sink = Arc::new(RecordingSink::default());
        let event_sink: Arc<dyn EventSink> = sink.clone();

        assert!(emit_stream_event(
            &db,
            &event_sink,
            &session.id,
            stream_seq::ChatSource::Desktop,
            Some(&turn_id),
            r#"{"type":"text_delta","content":"kept"}"#,
        ));
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!emit_stream_event(
            &db,
            &event_sink,
            &session.id,
            stream_seq::ChatSource::Desktop,
            Some(&turn_id),
            r#"{"type":"text_delta","content":"dropped"}"#,
        ));
        assert_eq!(sink.events.lock().unwrap().len(), 1);

        cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(crate::chat_engine::active_turn::force_release(
            &session.id,
            &turn_id
        ));
        db.finish_chat_turn_once(
            &turn_id,
            session::ChatTurnStatus::Interrupted,
            Some(session::ChatTurnInterruptReason::UserStop),
            None,
            None,
        )
        .unwrap();
        assert!(!emit_stream_event(
            &db,
            &event_sink,
            &session.id,
            stream_seq::ChatSource::Desktop,
            Some(&turn_id),
            r#"{"type":"text_delta","content":"late"}"#,
        ));
        assert_eq!(sink.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn user_stop_before_first_model_event_finalizes_without_empty_assistant() {
        let (_lock, _dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let turn_id = create_user_turn(&db, &session.id);

        let server = MockServer::start().await;
        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let cancel = Arc::new(AtomicBool::new(true));
        let mut p = params(db.clone(), session.id.clone(), vec![model], vec![provider]);
        p.turn_id = Some(turn_id.clone());
        p.cancel = cancel;

        let result = run_chat_engine(p)
            .await
            .expect("user stop should not surface as chat error");
        assert_eq!(result.response, "");

        let turn = db.get_chat_turn(&turn_id).unwrap().unwrap();
        assert_eq!(turn.status, session::ChatTurnStatus::Interrupted);
        assert_eq!(
            turn.interrupt_reason,
            Some(session::ChatTurnInterruptReason::UserStop)
        );

        let messages = db.load_session_messages(&session.id).unwrap();
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::Assistant && msg.content.is_empty()));
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::Event && msg.content.contains("已停止此次回复")
        }));
        let context_json = db.load_context(&session.id).unwrap().unwrap_or_default();
        assert!(context_json.contains("用户主动停止"));
    }

    #[tokio::test]
    async fn user_stop_after_text_delta_preserves_partial_and_marker() {
        let (_lock, _dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let turn_id = create_user_turn(&db, &session.id);

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_text_then_done("partial before stop")),
            )
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut p = params(db.clone(), session.id.clone(), vec![model], vec![provider]);
        p.turn_id = Some(turn_id.clone());
        p.cancel = cancel.clone();
        p.event_sink = Arc::new(CancelOnTextDelta {
            cancel: cancel.clone(),
        });

        let result = run_chat_engine(p)
            .await
            .expect("user stop should preserve partial");
        assert_eq!(result.response, "");

        let turn = db.get_chat_turn(&turn_id).unwrap().unwrap();
        assert_eq!(turn.status, session::ChatTurnStatus::Interrupted);
        assert_eq!(
            turn.interrupt_reason,
            Some(session::ChatTurnInterruptReason::UserStop)
        );

        let messages = db.load_session_messages(&session.id).unwrap();
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant && msg.content == "partial before stop"
        }));
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::Event && msg.content.contains("已停止此次回复")
        }));
        let context_json = db.load_context(&session.id).unwrap().unwrap_or_default();
        assert!(context_json.contains("partial before stop"));
        assert!(context_json.contains("用户主动停止"));
    }

    #[tokio::test]
    async fn user_stop_preserves_the_batch_durable_before_cancel_was_observed() {
        let (_lock, _dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let turn_id = create_user_turn(&db, &session.id);

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_two_text_then_done("before stop", " after stop")),
            )
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri(), "m1");
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            stream_seq::ChatSource::Desktop,
            turn_id.clone(),
            cancel.clone(),
        )
        .unwrap();
        let mut p = params(db.clone(), session.id.clone(), vec![model], vec![provider]);
        p.turn_id = Some(turn_id.clone());
        p.cancel = cancel.clone();
        p.event_sink = Arc::new(CancelOnTextDelta {
            cancel: cancel.clone(),
        });

        run_chat_engine(p)
            .await
            .expect("user stop should not surface as chat error");

        let messages = db.load_session_messages(&session.id).unwrap();
        // Both deltas entered the same 100ms journal batch before the sink saw
        // the first durable broadcast and flipped `cancel`. The durable batch
        // is indivisible for terminal replay: preserving it avoids showing a
        // prefix that a reload cannot reproduce without per-token fsync.
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant && msg.content == "before stop after stop"
        }));
        let context_json = db.load_context(&session.id).unwrap().unwrap_or_default();
        assert!(context_json.contains("before stop"));
        assert!(context_json.contains("after stop"));
        assert!(context_json.contains("用户主动停止"));
    }

    #[tokio::test]
    async fn final_failure_preserves_partial_assistant_before_error_event() {
        let (_lock, _dir, db) = temp_db();
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

        let result = execute_admitted_params(params(
            db.clone(),
            session.id.clone(),
            vec![model],
            vec![provider],
        ))
        .await;
        assert!(matches!(
            result,
            Err(error) if error.kind == TurnFailureKind::ProviderExhausted
        ));

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
        let (_lock, _dir, db) = temp_db();
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
        let (_lock, _dir, db) = temp_db();
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
    async fn abort_on_cancel_preserves_durable_partial_without_changing_error_semantics() {
        let (_lock, _dir, db) = temp_db();
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
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant && msg.content == "partial before cancel"
        }));
    }

    #[tokio::test]
    async fn abort_on_cancel_after_tool_call_preserves_side_effect_barrier_record() {
        let (_lock, _dir, db) = temp_db();
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
        assert!(
            messages.iter().any(|msg| {
                msg.role == MessageRole::TextBlock && msg.content == "partial before tool"
            }),
            "durable messages: {messages:#?}"
        );
        assert!(messages
            .iter()
            .any(|msg| msg.role == MessageRole::Tool && msg.tool_call_id.is_some()));
        let context = db
            .load_context(&session.id)
            .unwrap()
            .expect("interrupted context");
        assert!(context.contains("function_call"));
        assert!(
            context.contains("function_call_output")
                || context.contains(finalize::rebuild::INTERRUPTED_TOOL_RESULT),
            "tool call must have a real or synthetic matching result: {context}"
        );
    }

    #[tokio::test]
    async fn fallback_success_discards_failed_model_partial() {
        let (_lock, _dir, db) = temp_db();
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
    async fn failure_before_first_context_checkpoint_keeps_user_prompt() {
        let (_lock, _dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let prompt = "keep this prompt after early provider failure";
        db.append_message(&session.id, &NewMessage::user(prompt))
            .unwrap();

        let model = ActiveModel {
            provider_id: "missing-provider".to_string(),
            model_id: "missing-model".to_string(),
        };
        let mut chat_params = params(db.clone(), session.id.clone(), vec![model], Vec::new());
        chat_params.message = prompt.to_string();

        let result = run_chat_engine(chat_params).await;
        assert!(result.is_err());

        let context = db
            .load_context(&session.id)
            .unwrap()
            .expect("terminal context");
        let history: Vec<serde_json::Value> = serde_json::from_str(&context).unwrap();
        let prompt_index = history
            .iter()
            .position(|item| {
                item.get("role").and_then(|role| role.as_str()) == Some("user")
                    && item.get("content").and_then(|content| content.as_str()) == Some(prompt)
            })
            .expect("failed turn must retain the user prompt");
        let marker_index = history
            .iter()
            .rposition(|item| item.get("role").and_then(|role| role.as_str()) == Some("assistant"))
            .expect("terminal marker");
        assert!(prompt_index < marker_index);
    }

    #[tokio::test]
    async fn fallback_success_discards_failed_model_tool_round() {
        let (_lock, _dir, db) = temp_db();
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
        let (_lock, _dir, db) = temp_db();
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
