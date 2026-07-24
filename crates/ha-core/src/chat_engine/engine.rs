use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::failover::{
    self,
    executor::{execute_with_failover, ExecutorError, FailoverPolicy},
};
use crate::provider::{ApiType, AuthProfile};
use crate::session;
use crate::turn_durability::{FlushReason, TurnDurabilitySink};

use super::context::*;
use super::finalize::{self, PartialMeta, TerminationReason};
use super::im_mirror::{attach_im_live_mirror, finalize_im_live_mirror};
use super::sink_registry;
use super::stream_broadcast;
use super::stream_seq;
use super::types::*;

const CHAT_CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const CHAT_CANCELLED_BY_CALLER: &str = "chat cancelled by caller";

async fn wait_for_chat_cancel(cancel: Arc<std::sync::atomic::AtomicBool>) {
    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(CHAT_CANCEL_POLL_INTERVAL).await;
    }
}

fn event_enters_runtime_loop(event: &str) -> bool {
    event.contains("\"type\":\"text_delta\"")
        || event.contains("\"type\":\"thinking_delta\"")
        || event.contains("\"type\":\"tool_call\"")
        || event.contains("\"type\":\"tool_result\"")
}

fn terminal_turn_state(
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

fn turn_accepts_stream_event(
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
struct ChatRoundOk {
    response: String,
    thinking: Option<String>,
    agent: AssistantAgent,
    history_len_before: usize,
    chat_start: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatEngineFailureKind {
    ProviderExhausted,
    Cancelled,
    Infrastructure,
}

#[derive(Debug)]
pub(crate) struct ChatEngineFailure {
    pub kind: ChatEngineFailureKind,
    message: String,
}

impl ChatEngineFailure {
    fn new(kind: ChatEngineFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<String> for ChatEngineFailure {
    fn from(message: String) -> Self {
        Self::new(ChatEngineFailureKind::Infrastructure, message)
    }
}

impl From<anyhow::Error> for ChatEngineFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::new(ChatEngineFailureKind::Infrastructure, error.to_string())
    }
}

impl std::fmt::Display for ChatEngineFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Drop-guarded scope for a session's visible stream lifecycle. Ensures
/// `stream_seq::end` fires on every `run_chat_engine` return path (including
/// panics), while allowing the successful path to end the UI stream before
/// post-turn follow-ups run. Desktop / HTTP / parent-injection turns broadcast
/// on the main `chat:*` bus; IM channel turns have a separate `channel:*`
/// lifecycle.
struct StreamLifecycle {
    session_id: String,
    stream_id: Option<String>,
    source: stream_seq::ChatSource,
    turn_id: Option<String>,
    terminal_status: Option<session::ChatTurnStatus>,
    interrupt_reason: Option<session::ChatTurnInterruptReason>,
    terminal_error: Option<String>,
    abandoned_recovery: Option<(std::sync::Arc<session::SessionDB>, String)>,
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
            abandoned_recovery: None,
            finished: false,
        })
    }

    fn arm_abandoned_recovery(
        &mut self,
        db: std::sync::Arc<session::SessionDB>,
        persistence_run_id: String,
    ) {
        self.abandoned_recovery = Some((db, persistence_run_id));
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
fn emit_stream_event(
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

fn emit_context_compaction_progress(
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
fn emit_stream_event_unchecked(
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
        session_db.clone(),
        resolved_temperature,
        None,
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

    let compacted_context_json = serde_json::to_string(&agent.get_conversation_history())
        .map_err(|e| persist_failed(format!("Cannot serialize compacted context: {e}")))?;
    if original_context_json.as_deref() != Some(compacted_context_json.as_str()) {
        let saved = session_db
            .save_context_if_unchanged(
                &session_id,
                original_context_json.as_deref(),
                &compacted_context_json,
            )
            .map_err(|e| persist_failed(format!("Cannot save compacted context: {e}")))?;
        if !saved {
            return Err(persist_failed(
                "Session context changed during manual compaction; skipped stale compacted snapshot"
                    .to_string(),
            ));
        }
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

/// Run the shared chat execution engine.
///
/// Handles: model chain traversal → agent building → config → history restoration
/// → streaming execution → tool persistence → failover → context compaction
/// → response saving → context persistence → memory extraction.
pub async fn run_chat_engine(params: ChatEngineParams) -> Result<ChatEngineResult, String> {
    run_chat_engine_classified(params)
        .await
        .map_err(|failure| failure.to_string())
}

pub(crate) async fn run_chat_engine_classified(
    params: ChatEngineParams,
) -> Result<ChatEngineResult, ChatEngineFailure> {
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
        mut extra_system_context,
        reasoning_effort,
        cancel,
        plan_context_override,
        skill_allowed_tools,
        denied_tools,
        tool_scope,
        subagent_depth,
        steer_run_id,
        auto_approve_tools,
        follow_global_reasoning_effort,
        post_turn_effects,
        abort_on_cancel,
        persist_final_error_event,
        source,
        origin_source,
        channel_kb_context,
        event_sink,
    } = params;

    // Atomically register execution against the lifecycle gate. Every desktop,
    // HTTP, channel, ACP, subagent, and parent-injection path must fail closed
    // once an Agent is disabled, and deletion must see admitted work even
    // before its durable activity rows have been written.
    let _agent_run_guard =
        crate::agent_lifecycle::begin_agent_run(&agent_id).map_err(|error| error.to_string())?;

    // Effective KB-access origin for this turn (design D10): top-level turns
    // have origin == source; a subagent carries its parent turn's origin so an
    // IM-origin chain can't reacquire KB access via the neutral Subagent source.
    let kb_origin = origin_source.unwrap_or_else(|| kb_access_source(source));

    // Wrap attachments in Arc<[T]> so the failover-executor closure's per-
    // retry capture is a pointer bump instead of a deep clone of base64
    // image data (Attachment.data may carry MB-sized strings).
    let attachments: std::sync::Arc<[crate::agent::Attachment]> = std::sync::Arc::from(attachments);

    if model_chain.is_empty() {
        return Err("No model configured for chat execution".to_string().into());
    }

    {
        // `maybe_schedule_autonomous_start` runs synchronous SessionDB reads
        // (title classification + goal-fallback repair) before spawning the
        // title task; route it through the blocking pool so it never pins the
        // async worker on the per-turn hot path (see `crate::blocking`).
        let title_db = db.clone();
        let title_session_id = session_id.clone();
        let title_agent_id = agent_id.clone();
        let title_model = model_chain[0].clone();
        crate::blocking::run_blocking(move || {
            crate::session_title::maybe_schedule_autonomous_start(
                title_db,
                title_session_id,
                title_agent_id,
                title_model,
            )
        })
        .await;
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

    // Every conversation-producing entry receives a persistence run, even
    // when it has no user-visible chat_turn id. Incognito registrations stay
    // memory-only inside the coordinator.
    let durability = match super::durability::StreamCoordinator::create(
        db.clone(),
        session_id.clone(),
        source,
        stream_lifecycle.stream_id.clone(),
        turn_id.clone(),
        event_sink.clone(),
        cancel.clone(),
    )
    .await
    {
        Ok(coordinator) => coordinator,
        Err(error) => {
            let message = format!("Cannot initialize durable chat stream: {error}");
            if let Some(turn_id) = turn_id.as_deref() {
                if let Err(finish_error) = db.finish_chat_turn_once(
                    turn_id,
                    session::ChatTurnStatus::Failed,
                    Some(session::ChatTurnInterruptReason::Unknown),
                    Some(&message),
                    None,
                ) {
                    app_error!(
                        "chat",
                        "stream_durability",
                        "failed to converge turn {} after coordinator initialization error: {}",
                        turn_id,
                        finish_error
                    );
                }
            }
            stream_lifecycle.set_terminal(
                session::ChatTurnStatus::Failed,
                Some(session::ChatTurnInterruptReason::Unknown),
                Some(message.clone()),
            );
            stream_lifecycle.finish();
            return Err(message.into());
        }
    };
    stream_lifecycle
        .arm_abandoned_recovery(db.clone(), durability.persistence_run_id().to_string());

    // Idle/busy tracking (R2 — §5.4 fix). Mark this session active for the whole
    // turn so background-job / sub-agent completion injection yields to the live
    // turn instead of splicing into it. Created here at the shared engine entry
    // so all four foreground entry points are covered uniformly — desktop, HTTP,
    // IM channel, and cron (cron turns carry `Channel`). Previously only the
    // Tauri shell created the guard (`commands/chat.rs`), so on server / IM the
    // gate `ACTIVE_CHAT_SESSIONS` stayed at 0 and injection fired immediately
    // against a running turn. The Tauri shell keeps its own earlier guard (to
    // cancel an in-flight injection the moment the user hits send, before this
    // turn's preflight); the refcount in `ChatSessionGuard` makes the overlap
    // safe — the engine guard drops first, the shell guard last, so idle/flush
    // fires exactly once after the whole command. `ParentInjection` / `Subagent`
    // are excluded by `holds_foreground_idle_guard` (the former is the injection
    // itself; the latter is a distinct child session). ACP guards itself.
    let _idle_guard = source
        .holds_foreground_idle_guard()
        .then(|| crate::subagent::ChatSessionGuard::new(&session_id));

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

    // SessionStart hook (startup / resume). Observation event — any
    // additionalContext is merged into `extra_system_context` so it rides this
    // turn's system prompt and survives failover retries (which rebuild the
    // agent from this same local). The helper is shared with the ACP turn loop
    // (which runs `AssistantAgent::chat` directly, not this engine) so both
    // entry points fire SessionStart and resolve cwd identically.
    //
    // Gate on `source.fires_user_lifecycle_hooks()`: subagent / parent-injection
    // runs are internal workers, not user-visible sessions, so they MUST NOT
    // fire SessionStart. Without this gate an `agent` handler on `SessionStart`
    // spawns a sub-agent on every run, whose own chat-engine pass fires another
    // `SessionStart` (new session id ⇒ per-session `claim_session_start` doesn't
    // dedupe), and so on — a single global SessionStart agent hook would burn
    // tokens until concurrency or external limits intervene. Subagent
    // observability lives on `SubagentStart` / `SubagentStop` instead, also
    // gated against hook-spawned children in `subagent::spawn`.
    if source.fires_user_lifecycle_hooks() {
        if let Some(extra) = crate::hooks::fire_session_start_observation(
            &session_id,
            &agent_id,
            model_chain
                .first()
                .map(|m| m.model_id.as_str())
                .unwrap_or_default(),
        )
        .await
        {
            extra_system_context = Some(match extra_system_context.take() {
                Some(e) => format!("{e}\n\n{extra}"),
                None => extra,
            });
        }
    }

    // UserPromptSubmit hook context: the preflight chokepoint stashed any
    // `additionalContext` from the UserPromptSubmit hook keyed by session;
    // drain it here so it rides this turn's system prompt next to SessionStart
    // (and survives failover for the same reason — it lives in this run-local).
    // Drained exactly once per turn.
    if let Some(extra) = crate::hooks::take_user_prompt_context(&session_id) {
        extra_system_context = Some(match extra_system_context.take() {
            Some(e) => format!("{e}\n\n{extra}"),
            None => extra,
        });
    }

    // Knowledge read bridge channel ① (D7): deterministically inject notes the
    // user referenced inline with `[[ ]]`, scoped by `effective_kb_access` (D10)
    // and wrapped as untrusted external data (#7). Skipped for incognito inside
    // the resolver (zero KB access).
    if let Some(extra) = crate::knowledge::inject::resolve_inline_injections(
        &message,
        &session_id,
        kb_access_source(source),
        kb_origin,
        channel_kb_context.clone(),
    ) {
        extra_system_context = Some(match extra_system_context.take() {
            Some(e) => format!("{e}\n\n{extra}"),
            None => extra,
        });
    }

    // Built-in skill activation via the composer's `@skill` mention. Mirrors the
    // note bridge above: deterministic, user-controlled, injected into this
    // turn's system context. The fixed allowlist (office trio + data analytics
    // + browser + mac control) and the OS gate are enforced inside the resolver, so arbitrary
    // skill names in the message can't ride here — they stay as plain text.
    //
    // Gate on `fires_user_lifecycle_hooks()` (Desktop / HTTP / IM): only a real
    // user turn carries a composer `@skill` gesture. Internal Subagent /
    // ParentInjection runs are excluded so a sub-agent's untrusted output
    // containing a `[@…](#skill:…)` token can't self-activate a built-in skill
    // into the parent's system context.
    if source.fires_user_lifecycle_hooks() {
        if let Some(extra) = crate::skills::resolve_inline_skill_mentions(&message) {
            extra_system_context = Some(match extra_system_context.take() {
                Some(e) => format!("{e}\n\n{extra}"),
                None => extra,
            });
        }
        if let Some(extra) = crate::subagent::resolve_inline_agent_mentions(&message) {
            extra_system_context = Some(match extra_system_context.take() {
                Some(e) => format!("{e}\n\n{extra}"),
                None => extra,
            });
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
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
            break;
        }
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

        // Build the fallback event now, but enqueue it only after the next
        // attempt has been opened. Otherwise it would land in the previous
        // attempt and be correctly discarded together with that superseded
        // output during replay/materialization.
        let fallback_event_json = if idx > 0 {
            let display = format!("{} / {}", prov.name, model_ref.model_id);
            let reason_str = last_error
                .as_deref()
                .map(failover::classify_error)
                .unwrap_or(failover::FailoverReason::Unknown);
            crate::eval_context::record_model_retry(&session_id, true, reason_str.as_str(), 0);
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
            serde_json::to_string(&event).ok()
        } else {
            None
        };

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
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &json_str,
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
            let channel_kb_context_ref = &channel_kb_context;
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
            let durability_ref = durability.clone();
            let fallback_event_ref = fallback_event_json.as_deref();

            let exec_result = execute_with_failover(
                prov,
                &session_id,
                FailoverPolicy::chat_engine_default().with_cancel(cancel.clone()),
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
                    let cancel_for_wait = cancel_for_op.clone();
                    let turn_id_for_cb = turn_id.clone();

                    let agent_id_owned = agent_id_ref.clone();
                    let session_id_owned = session_id_ref.clone();
                    let extra_ctx_owned = extra_system_context_ref.clone();
                    let skill_tools_owned = skill_allowed_tools_ref.clone();
                    let denied_tools_owned = denied_tools.clone();
                    let steer_run_id_owned = steer_run_id.clone();
                    let plan_resolved_owned = plan_resolved_ref.clone();
                    let channel_kb_context_owned = channel_kb_context_ref.clone();
                    let message_owned = message_ref.clone();
                    // Arc<[Attachment]> clone is a pointer bump regardless
                    // of attachment size. See param destructure for the wrap.
                    let attachments_owned = attachments_ref.clone();
                    let effort_owned = effort_str_ref.clone();
                    let db_owned = db_ref.clone();
                    let provider_id_for_err = model_ref_for_op.provider_id.clone();
                    let model_id_for_err = model_ref_for_op.model_id.clone();
                    let codex_token_owned = codex_token_ref.clone();
                    let durability_owned = durability_ref.clone();
                    let fallback_event_owned = fallback_event_ref.map(ToOwned::to_owned);
                    async move {
                        let provider_shape = match &prov.api_type {
                            ApiType::Anthropic => "anthropic",
                            ApiType::OpenaiChat => "openai_chat",
                            ApiType::OpenaiResponses => "openai_responses",
                            ApiType::Codex => "codex",
                        };
                        durability_owned
                            .begin_attempt(
                                Some(&model_ref_for_op.provider_id),
                                Some(&model_ref_for_op.model_id),
                                Some(provider_shape),
                            )
                            .await?;
                        if let Some(fallback_event) = fallback_event_owned.as_deref() {
                            emit_stream_event(
                                &db_owned,
                                &event_sink_for_cb,
                                &session_for_cb,
                                source_for_cb,
                                turn_id_for_cb.as_deref(),
                                fallback_event,
                            );
                        }
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
                            db_owned.clone(),
                            resolved_temperature,
                            extra_ctx_owned.as_deref(),
                            &skill_tools_owned,
                            &denied_tools_owned,
                            tool_scope,
                            subagent_depth,
                            steer_run_id_owned,
                            plan_resolved_owned,
                            plan_context_locked,
                            auto_approve_tools,
                            follow_global_reasoning_effort,
                            source,
                            kb_origin,
                            channel_kb_context_owned,
                        );
                        agent.set_turn_durability(durability_owned.clone());
                        restore_agent_context(&db_owned, &session_id_owned, &agent);

                        let history_len_before = agent.get_conversation_history().len();
                        let chat_start = std::time::Instant::now();
                        let allow_hard_cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
                        let allow_hard_cancel_for_cb = allow_hard_cancel.clone();

                        let mut chat_future = Box::pin(agent.chat(
                            &message_owned,
                            &attachments_owned,
                            effort_owned.as_deref(),
                            cancel_for_op,
                            move |delta| {
                                if !turn_accepts_stream_event(
                                    &db_owned,
                                    &session_for_cb,
                                    turn_id_for_cb.as_deref(),
                                ) {
                                    return;
                                }
                                if event_enters_runtime_loop(delta) {
                                    allow_hard_cancel_for_cb
                                        .store(false, std::sync::atomic::Ordering::SeqCst);
                                }
                                // Guard already checked above this tick — skip
                                // the redundant turn_accepts lock + snapshot.
                                emit_stream_event_unchecked(
                                    &event_sink_for_cb,
                                    &session_for_cb,
                                    source_for_cb,
                                    turn_id_for_cb.as_deref(),
                                    delta,
                                );
                            },
                        ));
                        let chat_result = match tokio::select! {
                            biased;
                            _ = wait_for_chat_cancel(cancel_for_wait) => None,
                            result = &mut chat_future => Some(result),
                        } {
                            Some(result) => result,
                            None if allow_hard_cancel.load(std::sync::atomic::Ordering::SeqCst) => {
                                Err(anyhow::anyhow!(CHAT_CANCELLED_BY_CALLER))
                            }
                            None => chat_future.as_mut().await,
                        };
                        drop(chat_future);

                        if abort_on_cancel
                            && cancel_for_check.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            return Err(anyhow::anyhow!("chat cancelled by caller"));
                        }

                        match chat_result {
                            Ok((response, thinking)) => Ok(ChatRoundOk {
                                response,
                                thinking,
                                agent,
                                history_len_before,
                                chat_start,
                            }),
                            Err(e) => Err(e),
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
                        history_len_before,
                        chat_start,
                    } = ok;
                    let duration_ms = chat_start.elapsed().as_millis() as u64;

                    if let Some(ref tid) = turn_id {
                        if let Ok(Some(turn)) = db.get_chat_turn(tid) {
                            if turn.status.is_terminal() {
                                // A watchdog/request guard may have finalized
                                // chat_turns while the provider future was
                                // still unwinding. The journal must still be
                                // materialized atomically; merely marking the
                                // run terminal would strand already displayed
                                // bytes outside canonical messages/context.
                                let terminal = if turn.status == session::ChatTurnStatus::Completed
                                {
                                    session::ChatTurnStatus::Failed
                                } else {
                                    turn.status
                                };
                                let interrupt = turn
                                    .interrupt_reason
                                    .unwrap_or(session::ChatTurnInterruptReason::Unknown);
                                let convergence: Result<(), String> = async {
                                    let final_seq = durability
                                        .flush(FlushReason::Failure)
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    durability
                                        .reconcile_spool_to_sqlite()
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    let mut partial_text = durability.trailing_text();
                                    if partial_text.is_empty() && !durability.had_text_output() {
                                        partial_text = response.clone();
                                    }
                                    let assistant = durability.had_text_output().then(|| {
                                        build_durable_assistant_message(
                                            &durability,
                                            &partial_text,
                                            thinking.clone(),
                                            duration_ms,
                                            source,
                                        )
                                    });
                                    let context_json =
                                        serde_json::to_string(&agent.get_conversation_history())
                                            .map_err(|error| error.to_string())?;
                                    let commit = session::CommitInterruptedTurn {
                                        run_id: durability
                                            .is_persistent()
                                            .then(|| durability.persistence_run_id().to_string()),
                                        attempt_no: durability.current_attempt_no(),
                                        session_id: session_id.clone(),
                                        assistant,
                                        context_json,
                                        expected_context_revision: durability.context_revision(),
                                        turn_id: turn_id.clone(),
                                        final_seq,
                                        status: terminal,
                                        interrupt_reason: Some(interrupt.as_str().to_string()),
                                        error: turn.error.clone(),
                                        recovery_event: None,
                                    };
                                    let db_for_commit = db.clone();
                                    db_for_commit
                                        .run(move |db| db.commit_interrupted_turn(&commit))
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    Ok(())
                                }
                                .await;
                                if let Err(error) = convergence {
                                    let message = format!(
                                        "externally-terminal stream convergence failed: {error}"
                                    );
                                    app_error!(
                                        "chat",
                                        "stream_durability",
                                        "run {}: {}",
                                        durability.persistence_run_id(),
                                        message
                                    );
                                    // Keep the DB run in `running` state so a
                                    // restart can replay its durable journal.
                                    durability.mark_interrupted("persistence_unavailable");
                                    stream_lifecycle.set_terminal(
                                        session::ChatTurnStatus::Failed,
                                        Some(session::ChatTurnInterruptReason::Unknown),
                                        Some(message.clone()),
                                    );
                                    stream_lifecycle.finish();
                                    return Err(message.into());
                                }
                                durability.mark_interrupted(terminal.as_str());
                                stream_lifecycle.set_terminal(
                                    terminal,
                                    Some(interrupt),
                                    turn.error.clone(),
                                );
                                stream_lifecycle.finish();
                                schedule_browser_turn_finalize(source, &session_id);
                                return Ok(ChatEngineResult {
                                    response,
                                    model_used: Some(model_ref.clone()),
                                    usage: durability.usage(),
                                    agent: Some(agent),
                                });
                            }
                        }
                    }

                    // A provider can finish before the 100ms durability writer
                    // publishes its last batch. Publishing that batch may be
                    // the moment the UI observes the first delta and requests
                    // Stop, so check cancellation only after this barrier as
                    // well as inside the provider loop. Otherwise a late Stop
                    // races through the normal completed transaction.
                    if !abort_on_cancel && persist_final_error_event {
                        durability
                            .flush(FlushReason::FinalEnd)
                            .await
                            .map_err(|error| {
                                format!("pre-final durability barrier failed: {error}")
                            })?;
                        durability
                            .reconcile_spool_to_sqlite()
                            .await
                            .map_err(|error| format!("pre-final spool import failed: {error}"))?;
                    }

                    if !abort_on_cancel
                        && cancel.load(std::sync::atomic::Ordering::SeqCst)
                        && persist_final_error_event
                    {
                        // Reuse the common journal-replay convergence below. It
                        // appends the user-stop marker to provider context and
                        // writes the matching UI event in the same transaction;
                        // the former inline branch omitted both.
                        last_reason = None;
                        last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                        last_was_no_profile = false;
                        break;
                    }

                    // Emit usage event with duration
                    let usage_event = serde_json::json!({
                        "type": "usage",
                        "duration_ms": duration_ms,
                    });
                    if let Ok(json_str) = serde_json::to_string(&usage_event) {
                        emit_stream_event(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &json_str,
                        );
                    }

                    // Freeze the complete durable prefix before deriving the
                    // canonical assistant. Reading `trailing_text()` before
                    // this barrier can miss the final <100ms pending batch and
                    // would commit a truncated assistant despite the journal
                    // containing (and the UI receiving) the full response.
                    let final_seq = match durability.flush(FlushReason::FinalEnd).await {
                        Ok(seq) => seq,
                        Err(error) => {
                            let message = format!("final durability barrier failed: {error}");
                            stream_lifecycle.set_terminal(
                                session::ChatTurnStatus::Failed,
                                Some(session::ChatTurnInterruptReason::Unknown),
                                Some(message.clone()),
                            );
                            stream_lifecycle.finish();
                            return Err(message.into());
                        }
                    };
                    if let Err(error) = durability.reconcile_spool_to_sqlite().await {
                        let message = format!("cannot import emergency stream spool: {error}");
                        stream_lifecycle.set_terminal(
                            session::ChatTurnStatus::Failed,
                            Some(session::ChatTurnInterruptReason::Unknown),
                            Some(message.clone()),
                        );
                        stream_lifecycle.finish();
                        return Err(message.into());
                    }

                    let mut trailing_text = durability.trailing_text();
                    let trailing_placeholder_id = None;
                    if trailing_text.is_empty()
                        && !durability.had_text_output()
                        && !response.is_empty()
                    {
                        // Defensive fallback for provider adapters that return
                        // terminal text without emitting text_delta.
                        trailing_text = response.clone();
                    }
                    let mut assistant_msg = build_durable_assistant_message(
                        &durability,
                        &trailing_text,
                        thinking,
                        duration_ms,
                        source,
                    );
                    let active_trace = agent.current_active_memory_trace();
                    let used_refs = agent.current_used_memory_refs();
                    let retrieval_planner_trace = agent.current_retrieval_planner_trace(&used_refs);
                    if active_trace.is_some()
                        || !used_refs.is_empty()
                        || retrieval_planner_trace.is_some()
                    {
                        let mut meta = serde_json::Map::new();
                        if let Some(trace) = active_trace {
                            meta.insert(
                                session::ATTACHMENT_META_KEY_ACTIVE_MEMORY.to_string(),
                                serde_json::to_value(&*trace).unwrap_or(serde_json::Value::Null),
                            );
                        }
                        if !used_refs.is_empty() {
                            meta.insert(
                                session::ATTACHMENT_META_KEY_USED_MEMORY_REFS.to_string(),
                                serde_json::to_value(used_refs).unwrap_or(serde_json::Value::Null),
                            );
                        }
                        if let Some(trace) = retrieval_planner_trace {
                            meta.insert(
                                session::ATTACHMENT_META_KEY_RETRIEVAL_PLANNER.to_string(),
                                serde_json::to_value(trace).unwrap_or(serde_json::Value::Null),
                            );
                        }
                        assistant_msg.attachments_meta =
                            serde_json::to_string(&serde_json::Value::Object(meta)).ok();
                    }
                    let usage = durability.usage();
                    let mut ledger_event =
                        crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_CHAT)
                            .with_usage(
                                usage.input_tokens.unwrap_or(0) as u64,
                                usage.output_tokens.unwrap_or(0) as u64,
                                usage.cache_creation_input_tokens.unwrap_or(0) as u64,
                                usage.cache_read_input_tokens.unwrap_or(0) as u64,
                            )
                            .with_context_usage(
                                usage
                                    .context_input_tokens
                                    .or(usage.input_tokens)
                                    .unwrap_or(0) as u64,
                                usage.fresh_input_tokens.or(usage.input_tokens).unwrap_or(0) as u64,
                            );
                    ledger_event.timestamp = Some(chrono::Utc::now().to_rfc3339());
                    ledger_event.operation = Some("chat".to_string());
                    ledger_event.source = Some(source.as_str().to_string());
                    ledger_event.provider_id = Some(model_ref.provider_id.clone());
                    ledger_event.provider_name = Some(prov.name.clone());
                    ledger_event.model_id = Some(
                        usage
                            .model
                            .clone()
                            .unwrap_or_else(|| model_ref.model_id.clone()),
                    );
                    ledger_event.session_id = Some(session_id.clone());
                    ledger_event.agent_id = Some(agent_id.clone());
                    ledger_event.duration_ms = Some(duration_ms);
                    ledger_event.ttft_ms = usage.ttft_ms.map(|value| value.max(0) as u64);
                    // Per-Provider-round accounting is recorded inside the
                    // streaming adapter. The durable aggregate still carries
                    // evaluation identity for traceability without counting a
                    // second model call.
                    crate::eval_context::enrich_usage_metadata(&mut ledger_event);

                    let context_json = serde_json::to_string(&agent.get_conversation_history())
                        .map_err(|error| format!("serialize final context failed: {error}"))?;
                    let commit = session::CommitAssistantTurn {
                        run_id: durability
                            .is_persistent()
                            .then(|| durability.persistence_run_id().to_string()),
                        attempt_no: durability.current_attempt_no(),
                        session_id: session_id.clone(),
                        assistant: assistant_msg,
                        trailing_placeholder_id,
                        context_json,
                        expected_context_revision: durability.context_revision(),
                        turn_id: turn_id.clone(),
                        usage: Some(ledger_event),
                        final_seq,
                    };
                    let committed = {
                        let db = db.clone();
                        db.run(move |db| db.commit_assistant_turn(&commit)).await
                    };
                    let committed = match committed {
                        Ok(committed) => committed,
                        Err(error) => {
                            let message = format!("final assistant transaction failed: {error}");
                            // Do not terminalize the persistence run here.
                            // Its journal is the only recovery source after a
                            // failed final transaction; startup must still see
                            // the run as recoverable.
                            durability.mark_interrupted("failed");
                            stream_lifecycle.set_terminal(
                                session::ChatTurnStatus::Failed,
                                Some(session::ChatTurnInterruptReason::Unknown),
                                Some(message.clone()),
                            );
                            stream_lifecycle.finish();
                            return Err(message.into());
                        }
                    };
                    let assistant_id = Some(committed.assistant_message_id);
                    durability.mark_committed(committed.committed_seq);

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
                    let terminal_status = session::ChatTurnStatus::Completed;
                    let interrupt_reason = None;
                    stream_lifecycle.set_terminal(terminal_status, interrupt_reason, None);
                    stream_lifecycle.finish();
                    schedule_browser_turn_finalize(source, &session_id);

                    // Stop hook: the agent finished responding. `terminal_status`
                    // distinguishes a natural `completed` from an interrupt —
                    // block-to-continue is honored ONLY on `completed`
                    // (fire_stop guards on it), never on a user interrupt.
                    crate::hooks::fire_stop(
                        &session_id,
                        Some(&agent_id),
                        terminal_status.as_str(),
                        None,
                    );

                    if terminal_status == session::ChatTurnStatus::Completed {
                        let continuation = {
                            let session_id = session_id.clone();
                            let agent_id = agent_id.clone();
                            let turn_id = turn_id.clone();
                            db.run(move |db| {
                                crate::goal::maybe_schedule_goal_continuation(
                                    db,
                                    &session_id,
                                    &agent_id,
                                    source,
                                    turn_id.as_deref(),
                                    assistant_id,
                                )
                            })
                            .await
                        };
                        if let Err(e) = continuation {
                            app_warn!(
                                "goal",
                                "auto_continue",
                                "Failed to schedule goal continuation for session {}: {}",
                                session_id,
                                e
                            );
                        }
                    }

                    if post_turn_effects {
                        crate::session_title::maybe_schedule_after_success(
                            db.clone(),
                            session_id.clone(),
                            agent_id.clone(),
                            model_ref.clone(),
                        );
                        {
                            let usage_snapshot = durability.usage();
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
                        )
                        .await;

                        // Skill auto-review trigger (gate 1 of the five-gate
                        // waterfall). Feed tool_use_count from this round's
                        // conversation slice — pure-chat turns yield 0 and
                        // are filtered by `require_tool_use` in the config.
                        // `history_tail_stats` walks the slice under one lock
                        // without cloning the whole history.
                        {
                            let round_tokens = {
                                let u = durability.usage();
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
                        usage: durability.usage(),
                        agent: Some(agent),
                    });
                }

                Err(ExecutorError::NeedsCompaction { last_profile }) => {
                    if let Some((status, interrupt, error)) =
                        terminal_turn_state(&db, turn_id.as_deref())
                    {
                        stream_lifecycle.set_terminal(status, interrupt, error);
                        stream_lifecycle.finish();
                        schedule_browser_turn_finalize(source, &session_id);
                        return Ok(ChatEngineResult {
                            response: String::new(),
                            model_used: Some(model_ref.clone()),
                            usage: Default::default(),
                            agent: None,
                        });
                    }

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

                    let mut progress_extra = serde_json::Map::new();
                    progress_extra.insert(
                        "attempt".to_string(),
                        serde_json::json!(compaction_attempts),
                    );
                    progress_extra.insert(
                        "max_attempts".to_string(),
                        serde_json::json!(MAX_COMPACTION_RETRIES),
                    );
                    progress_extra.insert(
                        "provider_id".to_string(),
                        serde_json::json!(model_ref.provider_id),
                    );
                    progress_extra.insert(
                        "model_id".to_string(),
                        serde_json::json!(model_ref.model_id),
                    );
                    let _ = emit_context_compaction_progress(
                        &db,
                        &event_sink,
                        &session_id,
                        source,
                        turn_id.as_deref(),
                        "preparing",
                        "emergency",
                        Some(progress_extra),
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
                            // The "preparing"/emergency spinner was already emitted
                            // above; emit a terminal "failed" so the GUI banner
                            // resolves instead of spinning forever on this break.
                            let _ = emit_context_compaction_progress(
                                &db,
                                &event_sink,
                                &session_id,
                                source,
                                turn_id.as_deref(),
                                "failed",
                                "emergency",
                                None,
                            );
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
                        db.clone(),
                        resolved_temperature,
                        extra_system_context.as_deref(),
                        &skill_allowed_tools,
                        &denied_tools,
                        tool_scope,
                        subagent_depth,
                        steer_run_id.clone(),
                        plan_resolved.clone(),
                        plan_context_locked,
                        auto_approve_tools,
                        follow_global_reasoning_effort,
                        source,
                        kb_origin,
                        channel_kb_context.clone(),
                    );
                    restore_agent_context(&db, &session_id, &compact_agent);

                    let mut history = compact_agent.get_conversation_history();
                    // Incognito parity with the Tier-3 path (agent/context.rs): an
                    // incognito session must NOT have its runtime ledger (job /
                    // subagent ids) built or injected into history — that history is
                    // both sent to the model and persisted via save_agent_context
                    // below. Fail-closed: a missing/burned session row counts as
                    // incognito. Gating lives in `emergency_runtime_ledger` (unit-tested).
                    let emergency_ledger = crate::agent::runtime_ledger::emergency_runtime_ledger(
                        &session_id,
                        crate::session::is_session_incognito(Some(&session_id)),
                    );
                    let emergency_ctx = crate::context_compact::EmergencyCompactionContext {
                        config: &compact_config,
                        runtime_ledger: emergency_ledger.as_ref(),
                    };
                    let compact_result = compact_agent
                        .context_engine()
                        .emergency_compact(&mut history, &emergency_ctx);
                    compact_agent.set_conversation_history(history);
                    if let Some((status, interrupt, error)) =
                        terminal_turn_state(&db, turn_id.as_deref())
                    {
                        stream_lifecycle.set_terminal(status, interrupt, error);
                        stream_lifecycle.finish();
                        schedule_browser_turn_finalize(source, &session_id);
                        return Ok(ChatEngineResult {
                            response: String::new(),
                            model_used: Some(model_ref.clone()),
                            usage: Default::default(),
                            agent: None,
                        });
                    }
                    let compact_history = compact_agent.get_conversation_history();
                    if let Err(error) = durability
                        .checkpoint_context(&compact_history, durability.context_revision())
                        .await
                    {
                        let _ = emit_context_compaction_progress(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            "failed",
                            "emergency",
                            None,
                        );
                        last_error =
                            Some(format!("Emergency compaction context CAS failed: {error}"));
                        break;
                    }
                    if let Err(error) = durability.adopt_attempt_base_context(&compact_history) {
                        last_error =
                            Some(format!("Emergency compaction retry base failed: {error}"));
                        break;
                    }

                    let mut progress_extra = serde_json::Map::new();
                    progress_extra.insert(
                        "attempt".to_string(),
                        serde_json::json!(compaction_attempts),
                    );
                    progress_extra.insert(
                        "max_attempts".to_string(),
                        serde_json::json!(MAX_COMPACTION_RETRIES),
                    );
                    let _ = emit_context_compaction_progress(
                        &db,
                        &event_sink,
                        &session_id,
                        source,
                        turn_id.as_deref(),
                        "finalizing",
                        "emergency",
                        Some(progress_extra),
                    );

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
                            "manifest": compact_result.manifest,
                        },
                    })) {
                        // The coordinator journals this event and materializes
                        // it exactly once with the final turn transaction.
                        emit_stream_event(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &event_str,
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

                Err(ExecutorError::Cancelled) => {
                    last_reason = None;
                    last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                    last_was_no_profile = false;
                    break;
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
                                &db,
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

    // The journal, rather than legacy placeholder rows, is the truth source
    // for failed/aborted turns. Keep the last visible attempt and converge the
    // partial assistant + context + turn status atomically.
    let terminal_status = reason.to_chat_turn_status();
    let terminal_interrupt = reason.to_chat_turn_interrupt_reason();
    let durability_result: anyhow::Result<()> = async {
        let durable_seq = durability.flush(FlushReason::Stop).await?;
        if durability.is_persistent() {
            durability.reconcile_spool_to_sqlite().await?;
        }

        let (attempt_no, commit_seq, visible_events, integrity_error, provider_kind) =
            if durability.is_persistent() {
                let run_id = durability.persistence_run_id().to_string();
                let db_for_snapshot = db.clone();
                let snapshot = db_for_snapshot
                    .run(move |db| db.stream_run_snapshot(&run_id))
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("persistence run disappeared"))?;
                let (attempt_no, commit_seq, events, integrity_error) =
                    session::select_recoverable_attempt_prefix(&snapshot);
                let attempt = snapshot
                    .attempts
                    .iter()
                    .find(|attempt| attempt.attempt_no == attempt_no);
                let provider_kind = attempt
                    .and_then(|attempt| attempt.provider_shape.as_deref())
                    .or(snapshot.run.provider_shape.as_deref())
                    .and_then(finalize::ProviderApiKind::from_shape);
                (
                    attempt_no,
                    commit_seq,
                    events,
                    integrity_error,
                    provider_kind,
                )
            } else {
                let snapshot = durability.snapshot();
                (
                    durability.current_attempt_no(),
                    durable_seq,
                    snapshot.events,
                    None,
                    durability
                        .current_provider_shape()
                        .as_deref()
                        .and_then(finalize::ProviderApiKind::from_shape),
                )
            };
        let trailing_text = session::trailing_text_from_journal_events(&visible_events);
        let assistant = session::journal_events_have_assistant_output(&visible_events).then(|| {
            let mut message = session::NewMessage::assistant(&trailing_text);
            message.source = Some(source.as_str().to_string());
            message
        });
        let (context_json, context_checkpoint_seq, context_revision, has_context_checkpoint) =
            if durability.is_persistent() {
                let run_id = durability.persistence_run_id().to_string();
                db.clone()
                    .run(move |db| {
                        let (context, checkpoint_seq, revision) =
                            db.recovery_context_for_prefix(&run_id, attempt_no, commit_seq)?;
                        let has_checkpoint =
                            db.stream_context_checkpoint_exists(&run_id, attempt_no, commit_seq)?;
                        Ok::<_, anyhow::Error>((context, checkpoint_seq, revision, has_checkpoint))
                    })
                    .await?
            } else {
                let session_id_for_context = session_id.clone();
                let (context, revision) = db
                    .clone()
                    .run(move |db| db.load_context_with_revision(&session_id_for_context))
                    .await?;
                (context, 0, revision, false)
            };
        let mut history: Vec<serde_json::Value> = context_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        if !has_context_checkpoint {
            let user_message = message.trim();
            if !user_message.is_empty() {
                history.push(serde_json::json!({
                    "role": "user",
                    "content": user_message,
                }));
            }
        }
        finalize::rebuild::append_journal_suffix_to_history(
            &mut history,
            &visible_events,
            context_checkpoint_seq,
            provider_kind,
        )?;
        history.push(serde_json::json!({
            "role": "assistant",
            "content": finalize::copy::model_marker(&reason),
        }));
        let context_json = serde_json::to_string(&history)?;
        let recovery_event = persist_final_error_event.then(|| {
            let mut event = if terminal_status == session::ChatTurnStatus::Failed {
                session::NewMessage::error_event(&finalize::copy::user_notice(&reason))
            } else {
                session::NewMessage::event(&finalize::copy::user_notice(&reason))
            };
            event.source = Some(source.as_str().to_string());
            event
        });
        let commit = session::CommitInterruptedTurn {
            run_id: durability
                .is_persistent()
                .then(|| durability.persistence_run_id().to_string()),
            attempt_no,
            session_id: session_id.clone(),
            assistant,
            context_json,
            expected_context_revision: context_revision,
            turn_id: turn_id.clone(),
            final_seq: commit_seq,
            status: terminal_status,
            interrupt_reason: Some(terminal_interrupt.as_str().to_string()),
            error: integrity_error.or_else(|| {
                (terminal_status == session::ChatTurnStatus::Failed).then(|| final_error.clone())
            }),
            recovery_event,
        };
        let db_for_commit = db.clone();
        db_for_commit
            .run(move |db| db.commit_interrupted_turn(&commit))
            .await?;
        durability.mark_interrupted(terminal_status.as_str());
        Ok(())
    }
    .await;

    if let Err(error) = durability_result {
        app_error!(
            "chat",
            "stream_durability",
            "failed to converge terminal stream {}: {}",
            durability.persistence_run_id(),
            error
        );
        // Leave the DB run recoverable, but release the live coordinator so
        // the UI is not reported as indefinitely active in this process.
        durability.mark_interrupted("persistence_unavailable");
    }
    if let Some(state) = im_mirror.take() {
        tokio::spawn(async move {
            finalize_im_live_mirror(state, "").await;
        });
    }
    stream_lifecycle.set_terminal(
        terminal_status,
        Some(terminal_interrupt),
        (terminal_status == session::ChatTurnStatus::Failed).then(|| final_error.clone()),
    );

    if matches!(reason, TerminationReason::UserStop) && !abort_on_cancel {
        stream_lifecycle.finish();
        schedule_browser_turn_finalize(source, &session_id);
        return Ok(ChatEngineResult {
            response: String::new(),
            model_used: None,
            usage: Default::default(),
            agent: None,
        });
    }

    schedule_browser_turn_finalize(source, &session_id);
    stream_lifecycle.finish();
    let failure_kind = match &reason {
        TerminationReason::UserStop | TerminationReason::RuntimeCancel => {
            ChatEngineFailureKind::Cancelled
        }
        TerminationReason::ProviderFailed { .. } | TerminationReason::NoProfileAvailable => {
            ChatEngineFailureKind::ProviderExhausted
        }
        TerminationReason::CompactionFailed { .. }
        | TerminationReason::Other { .. }
        | TerminationReason::Shutdown
        | TerminationReason::Crash => ChatEngineFailureKind::Infrastructure,
    };
    Err(ChatEngineFailure::new(failure_kind, final_error))
}

fn build_durable_assistant_message(
    durability: &super::durability::StreamCoordinator,
    content: &str,
    thinking: Option<String>,
    duration_ms: u64,
    source: stream_seq::ChatSource,
) -> session::NewMessage {
    let usage = durability.usage();
    let mut message = session::NewMessage::assistant(content);
    message.tool_duration_ms = Some(duration_ms.min(i64::MAX as u64) as i64);
    if !durability.had_thinking() {
        message.thinking = thinking;
    }
    message.tokens_in = usage.input_tokens;
    message.tokens_out = usage.output_tokens;
    message.tokens_in_last = usage.last_context_input_tokens.or(usage.last_input_tokens);
    message.model = usage.model;
    message.ttft_ms = usage.ttft_ms;
    message.tokens_cache_creation = usage
        .last_cache_creation_input_tokens
        .or(usage.cache_creation_input_tokens);
    message.tokens_cache_read = usage
        .last_cache_read_input_tokens
        .or(usage.cache_read_input_tokens);
    message.source = Some(source.as_str().to_string());
    message
}

// ── Termination reason derivation ────────────────────────────────────

/// Map runtime convergence state to a [`TerminationReason`].
///
/// A set cancel flag is the positive signal for `UserStop`; user-facing
/// desktop / HTTP / IM paths all preserve partial state and converge through
/// the same interrupted finalizer. `last_reason == None` after a non-cancel
/// path means we never even reached an executor call → `NoProfileAvailable`.
/// Everything else is `ProviderFailed` carrying the classified reason.
fn derive_termination_reason(
    _abort_on_cancel: bool,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_reason: Option<failover::FailoverReason>,
    last_error: Option<&str>,
    last_is_codex_auth: bool,
    compaction_failed: Option<&str>,
    last_was_no_profile: bool,
) -> TerminationReason {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
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
#[allow(dead_code)] // legacy placeholder finalize compatibility
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

/// Map the chat-engine turn source to a knowledge-base access source (design
/// D10). IM (`Channel`) turns are denied KB access in Phase 1 even on a
/// project-attached session; `ParentInjection` is treated conservatively.
/// `Cron` is owner-internal (user-configured scheduled task): it maps to the
/// `Cron` bucket, which is NOT IM-capped, so a cron run reaches `note_*` /
/// `[[note]]` / `knowledge_recall` on its attached/project KBs the same way an
/// owner turn does — incognito still zeroes it via the `effective_kb_access`
/// short-circuit.
fn kb_access_source(source: stream_seq::ChatSource) -> crate::knowledge::KbAccessSource {
    use crate::knowledge::KbAccessSource;
    use stream_seq::ChatSource;
    match source {
        ChatSource::Desktop => KbAccessSource::Gui,
        ChatSource::Http => KbAccessSource::Http,
        ChatSource::Channel => KbAccessSource::Im,
        ChatSource::Subagent => KbAccessSource::Subagent,
        ChatSource::ParentInjection => KbAccessSource::Other,
        ChatSource::Cron => KbAccessSource::Cron,
        ChatSource::Acp => KbAccessSource::Other,
    }
}

/// Schedule turn-end browser cleanup, skipping `ParentInjection` turns.
///
/// Background-job / wakeup completions inject into the PARENT session and run a
/// turn under that session_id. Running the turn-end finalize there would tear
/// down the parent's live browser scope (close agent tabs, drop claim leases)
/// mid-task while the user may still be working in that session. The parent's
/// own foreground turns and session teardown handle cleanup, so injection turns
/// must skip it. Other sources (`Desktop`/`Http`/`Channel`/`Subagent`/`Cron`)
/// finalize their own session scope, which matches the documented turn-end
/// release.
fn schedule_browser_turn_finalize(source: stream_seq::ChatSource, session_id: &str) {
    if matches!(source, stream_seq::ChatSource::ParentInjection) {
        return;
    }
    crate::browser::schedule_extension_turn_finalize(session_id);
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
    session_db: Arc<session::SessionDB>,
    temperature: Option<f64>,
    extra_system_context: Option<&str>,
    skill_allowed_tools: &[String],
    denied_tools: &[String],
    tool_scope: Option<crate::tools::ToolScope>,
    subagent_depth: u32,
    steer_run_id: Option<String>,
    plan_resolved: crate::agent::PlanResolvedContext,
    plan_locked: bool,
    auto_approve_tools: bool,
    follow_global_reasoning_effort: bool,
    source: stream_seq::ChatSource,
    kb_origin: crate::knowledge::KbAccessSource,
    channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
) {
    agent.set_agent_id(agent_id);
    agent.set_session_db(session_db);
    agent.set_session_id(session_id);
    agent.set_chat_source(kb_access_source(source));
    agent.set_origin_chat_source(kb_origin);
    agent.set_channel_kb_context(channel_kb_context);
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
        let db = Arc::new(SessionDB::open_ephemeral_for_test(&path).unwrap());
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
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db) = temp_db();
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
        let (_dir, db) = temp_db();
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
        let (_dir, db) = temp_db();
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
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db) = temp_db();
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

        let result = run_chat_engine_classified(params(
            db.clone(),
            session.id.clone(),
            vec![model],
            vec![provider],
        ))
        .await;
        assert!(matches!(
            result,
            Err(ChatEngineFailure {
                kind: ChatEngineFailureKind::ProviderExhausted,
                ..
            })
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
    async fn abort_on_cancel_preserves_durable_partial_without_changing_error_semantics() {
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
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant && msg.content == "partial before cancel"
        }));
    }

    #[tokio::test]
    async fn abort_on_cancel_after_tool_call_preserves_side_effect_barrier_record() {
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
    async fn failure_before_first_context_checkpoint_keeps_user_prompt() {
        let (_dir, db) = temp_db();
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
