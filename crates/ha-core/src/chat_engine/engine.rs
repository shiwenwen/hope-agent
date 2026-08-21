use std::sync::Arc;

use anyhow::Context;

use crate::agent::{AssistantAgent, CurrentUserMessageState};
use crate::failover::{
    self,
    executor::{execute_with_failover_observed, ExecutorError, FailoverPolicy, RetryProgress},
};
use crate::provider::{ActiveModel, ApiType, AuthProfile, ProviderConfig};
use crate::session;
use crate::turn_durability::{FlushReason, TurnDurabilitySink};

use super::context::*;
use super::finalize::{self, PartialMeta, TerminationReason};
use super::sink_registry;
use super::stream_broadcast;
use super::stream_seq;
use super::types::*;

const CHAT_CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// Once a turn has emitted runtime-visible output we first give the provider /
/// tool loop a bounded window to propagate cancellation and clean up owned
/// resources.  The previous unbounded await meant one cancellation-unaware
/// tool or hook could keep the caller future alive forever, even though the
/// stop watchdog had already made the turn look terminal to the UI.
const CHAT_CANCEL_COOPERATIVE_GRACE: std::time::Duration = std::time::Duration::from_secs(6);
const CHAT_CANCELLED_BY_CALLER: &str = "chat cancelled by caller";

/// Deletes durable typed-resource snapshots unless the Initial Context event
/// that references them has crossed the durability barrier. A backend run UUID
/// in every basename gives crash recovery the same deterministic cleanup scope
/// if the process exits before this guard can run.
struct PendingTypedResourceSnapshots {
    session_id: String,
    snapshot_names: Vec<String>,
    refs_committed: Arc<std::sync::atomic::AtomicBool>,
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

fn should_retry_model_chain(
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

fn chain_reason_after_missing_provider(
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

fn fallback_event_reason(
    typed_reason: Option<failover::FailoverReason>,
    display_error: Option<&str>,
) -> failover::FailoverReason {
    typed_reason
        .or_else(|| display_error.map(failover::classify_error))
        .unwrap_or(failover::FailoverReason::Unknown)
}

fn has_resolvable_fallback(
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

fn resolve_slash_skill_binding<'a>(
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

fn ensure_explicit_slash_skill_requirements(
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

struct MaterializedSlashSkill {
    content: String,
    tool_ceiling: crate::skills::SkillToolCeiling,
}

fn require_explicit_mention_skill_activation(
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

fn require_explicit_slash_skill_materialization(
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

/// Consume an attached GUI / HTTP mirror and terminate its existing preview
/// through the channel-owned abort path. The engine never waits on remote IM
/// I/O: desktop completion remains independent, while the owned mirror state
/// guarantees the same Message / Card / Native identity is used for the
/// terminal mutation.
///
/// Returning the task makes the helper directly testable. Production callers
/// intentionally detach it because these paths never replay the logical turn.
fn abort_im_mirror_in_background(
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

fn abort_im_mirror_after_internal_error(
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
fn finalize_im_mirror_in_background(
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
fn mirror_reason_from_terminal_state(
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
pub enum ChatEngineFailureKind {
    ProviderExhausted,
    /// Application-level terminal outcome. Retrying the provider chain cannot
    /// make the same logical request legal (for example, a current tool-result
    /// group whose protocol-minimal envelope still exceeds capacity).
    Terminal,
    Cancelled,
    Infrastructure,
}

#[derive(Debug)]
pub struct ChatEngineFailure {
    pub(crate) kind: ChatEngineFailureKind,
    reason: Option<failover::FailoverReason>,
    is_codex_auth: bool,
    message: String,
}

impl ChatEngineFailure {
    fn new(kind: ChatEngineFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            reason: None,
            is_codex_auth: false,
            message: message.into(),
        }
    }

    fn classified(
        kind: ChatEngineFailureKind,
        reason: Option<failover::FailoverReason>,
        is_codex_auth: bool,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            reason,
            is_codex_auth,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ChatEngineFailureKind {
        self.kind
    }

    /// Typed reason from the final engine attempt. Callers must prefer this
    /// over re-classifying `to_string()`, which is intentionally only
    /// display text and may omit the evidence that established the verdict.
    pub fn reason(&self) -> Option<failover::FailoverReason> {
        self.reason
    }

    pub fn is_codex_auth(&self) -> bool {
        self.is_codex_auth
    }

    pub(crate) fn cancelled(message: impl Into<String>) -> Self {
        Self::new(ChatEngineFailureKind::Cancelled, message)
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

impl std::error::Error for ChatEngineFailure {}

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
/// follow-up replies are visible while they stream. Cron turns also use the
/// main bus because their ordinary Sessions can be opened mid-run. Channel and
/// child Subagent turns stay off it; IM uses `ChannelStreamSink` to emit
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

fn merge_explicit_skill_ceiling(
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
fn typed_note_byte_budget(
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
fn message_without_typed_note_spans(
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

fn validate_engine_typed_resource_boundary(
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

fn prepare_typed_resource_mentions_for_session(
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

/// Structured sibling of [`run_chat_engine`] for callers that make recovery
/// or user-notice decisions. Display strings are intentionally not a stable
/// failure-classification protocol.
pub async fn run_chat_engine_classified(
    params: ChatEngineParams,
) -> Result<ChatEngineResult, ChatEngineFailure> {
    let ChatEngineParams {
        session_id,
        agent_id,
        turn_id,
        mut message,
        incoming_turn,
        display_text,
        mut attachments,
        session_db: db,
        model_chain,
        providers,
        codex_token,
        resolved_temperature,
        compact_config,
        run_context,
        reasoning_effort,
        cancel,
        foreground_stop_admission,
        plan_context_override,
        mut skill_allowed_tools,
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
        ui_surface: _,
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

    // Freeze the typed composer contract once, before any provider/profile
    // attempt. Every failover serializes the same resolved bindings and user
    // envelope; no attempt re-reads display labels or reparses pasted tokens.
    let canonical_user_message = message.clone();
    validate_engine_typed_resource_boundary(
        &canonical_user_message,
        incoming_turn.as_ref(),
        &attachments,
    )?;
    let (
        mut turn_context_builder,
        agent_binding_refs,
        mut mention_receipts,
        mention_wire_version,
        mut legacy_compatibility,
    ) = if let Some(ref wire) = incoming_turn {
        let (builder, bindings, receipts) = crate::prompt_context::resolve_typed_turn_context(
            &canonical_user_message,
            wire,
            &session_id,
            turn_id.as_deref(),
            &agent_id,
        )
        .map_err(|error| format!("Invalid typed mention context: {error}"))?;
        (
            builder,
            bindings,
            receipts,
            Some(wire.mention_wire_version),
            false,
        )
    } else {
        (
            crate::prompt_context::TurnContextBuilder::default(),
            Vec::new(),
            Vec::new(),
            None,
            true,
        )
    };

    // Repository/project skill discovery is session-scoped. Resolve the
    // effective workspace from this engine's DB snapshot rather than the
    // process-global DB or process cwd, because one daemon can serve several
    // unrelated projects concurrently.
    let has_typed_skill_mentions = incoming_turn.as_ref().is_some_and(|wire| {
        wire.mentions
            .iter()
            .any(|mention| mention.kind == crate::prompt_context::MentionKind::Skill)
    });
    let skill_working_dir = if has_typed_skill_mentions {
        let snapshot_db = db.clone();
        let snapshot_session_id = session_id.clone();
        crate::blocking::run_blocking(move || {
            let session = snapshot_db
                .get_session(&snapshot_session_id)?
                .with_context(|| "typed skill mention session no longer exists")?;
            anyhow::Ok(crate::session::effective_working_dir_for_meta(&session))
        })
        .await
        .map_err(|error| format!("Cannot resolve typed skill workspace: {error}"))?
    } else {
        None
    };

    // A typed file binding is only resolved when the same turn also carries a
    // matching attachment. Resolve the canonical target beneath the session
    // working directory and read its bytes exactly once. This phase is
    // deliberately read-only: durable publication happens only after the
    // stream run exists and owns a staged materialization journal fact.
    let mut prepared_resource_mentions = None;
    if let Some(ref wire) = incoming_turn {
        let file_targets = wire
            .mentions
            .iter()
            .filter(|mention| mention.kind == crate::prompt_context::MentionKind::File)
            .map(|mention| mention.target_id.clone())
            .collect::<Vec<_>>();
        let plan_targets = wire
            .mentions
            .iter()
            .filter(|mention| mention.kind == crate::prompt_context::MentionKind::Plan)
            .map(|mention| mention.target_id.clone())
            .collect::<Vec<_>>();
        if !file_targets.is_empty() || !plan_targets.is_empty() {
            let snapshot_db = db.clone();
            let snapshot_session_id = session_id.clone();
            let snapshot_attachments = attachments;
            let (prepared_attachments, session_incognito, prepared) =
                crate::blocking::run_blocking(move || {
                    let session = snapshot_db
                        .get_session(&snapshot_session_id)?
                        .with_context(|| "typed resource mention session no longer exists")?;
                    let prepared = prepare_typed_resource_mentions_for_session(
                        &session,
                        &file_targets,
                        &plan_targets,
                        &snapshot_attachments,
                    )?;
                    anyhow::Ok((snapshot_attachments, session.incognito, prepared))
                })
                .await
                .map_err(|error| format!("Cannot freeze typed resource mentions: {error}"))?;
            attachments = prepared_attachments;
            prepared_resource_mentions = Some((session_incognito, prepared));
        }
    }
    if let Some(ref wire) = incoming_turn {
        let skill_ids = wire
            .mentions
            .iter()
            .filter(|mention| {
                mention.kind == crate::prompt_context::MentionKind::Skill
                    && mention.origin
                        != crate::prompt_context::StructuredMentionOrigin::SlashCommandAst
            })
            .map(|mention| mention.target_id.clone())
            .collect::<Vec<_>>();
        if !skill_ids.is_empty() {
            let activation = require_explicit_mention_skill_activation(
                &skill_ids,
                crate::skills_hooks::resolve_named_skill_mentions(
                    &skill_ids,
                    Some(&agent_id),
                    skill_working_dir.as_deref().map(std::path::Path::new),
                ),
            )
            .map_err(|error| format!("Invalid typed mention context: {error}"))?;
            turn_context_builder.user_instruction(
                crate::prompt_context::UserInstructionSource::ExplicitSkillMention,
                activation.content,
            );
            merge_explicit_skill_ceiling(&mut skill_allowed_tools, activation.tool_ceiling.clone());
            for receipt in &mut mention_receipts {
                if receipt.kind == crate::prompt_context::MentionKind::Skill
                    && activation
                        .resolved_names
                        .iter()
                        .any(|name| name == &receipt.target_id)
                {
                    receipt.status = crate::prompt_context::MentionResolutionStatus::Resolved;
                } else if receipt.kind == crate::prompt_context::MentionKind::Skill
                    && activation
                        .rejected_names
                        .iter()
                        .any(|name| name == &receipt.target_id)
                {
                    receipt.status = crate::prompt_context::MentionResolutionStatus::Rejected;
                }
            }
        }

        // Slash skills use the same typed binding/receipt channel but are not
        // restricted to the composer's curated @skill allowlist. Re-resolve
        // the canonical skill id against the live invocable catalog and render
        // it here, so a client cannot smuggle prompt content or tool grants.
        let slash_skill_mentions = wire
            .mentions
            .iter()
            .filter(|mention| {
                mention.kind == crate::prompt_context::MentionKind::Skill
                    && mention.origin
                        == crate::prompt_context::StructuredMentionOrigin::SlashCommandAst
            })
            .collect::<Vec<_>>();
        if !slash_skill_mentions.is_empty() {
            let cfg = crate::config::cached_config();
            let env_check = crate::skills::skill_env_check_enabled_for_agent(
                Some(&agent_id),
                cfg.skill_env_check,
            );
            let skill_env = cfg.skill_env.clone();
            let entries = crate::skills_hooks::invocable_skills(
                &cfg.extra_skills_dirs,
                &cfg.disabled_skills,
                skill_working_dir.as_deref().map(std::path::Path::new),
            );
            // Typed ownership must be rebuilt from the same session workspace
            // catalog as help/dispatch. Agent-specific requirement checks
            // still run after the collision-resolved binding is matched.
            let entries = crate::skills::filter_catalog_eligible_skills(
                entries,
                cfg.skill_env_check,
                &cfg.skill_env,
            );
            drop(cfg);

            for mention in slash_skill_mentions {
                let Some(entry) = resolve_slash_skill_binding(
                    &entries,
                    &mention.target_id,
                    &mention.display_label,
                ) else {
                    return Err(format!(
                        "Invalid typed mention context: slash command '{}' is not owned by skill '{}'",
                        mention.display_label, mention.target_id
                    )
                    .into());
                };
                ensure_explicit_slash_skill_requirements(entry, env_check, &skill_env)
                    .map_err(|error| format!("Invalid typed mention context: {error}"))?;
                let args =
                    crate::prompt_context::slash_skill_args(&canonical_user_message, mention);
                let rendered = match args.as_deref() {
                    Some(args) => match crate::skills::resolve_skill_slash_dispatch(entry, args) {
                        crate::skills::SkillSlashDispatch::ModelTemplate { message } => Ok(message),
                        crate::skills::SkillSlashDispatch::ModelInline => {
                            crate::skills_hooks::render_skill_inline(entry, args).await
                        }
                        crate::skills::SkillSlashDispatch::Fork
                        | crate::skills::SkillSlashDispatch::Tool => Err(anyhow::anyhow!(
                            "typed slash binding targets a non-model Skill dispatch"
                        )),
                    },
                    None => Err(anyhow::anyhow!(
                        "validated slash command binding has no canonical arguments"
                    )),
                };
                let activation =
                    require_explicit_slash_skill_materialization(entry, args, rendered)
                        .map_err(|error| format!("Invalid typed mention context: {error}"))?;
                turn_context_builder.user_instruction(
                    crate::prompt_context::UserInstructionSource::ExplicitSlashSkill,
                    activation.content,
                );
                merge_explicit_skill_ceiling(&mut skill_allowed_tools, activation.tool_ceiling);
                if let Some(receipt) = mention_receipts
                    .iter_mut()
                    .find(|receipt| receipt.mention_id == mention.id)
                {
                    receipt.status = crate::prompt_context::MentionResolutionStatus::Resolved;
                }
            }
        }
    }

    if model_chain.is_empty() {
        return Err("No model configured for chat execution".to_string().into());
    }

    // Resolve the Plan-mode bundle once at turn start. Spawn-supplied
    // overrides win (their child sessions have backend `plan_mode = Off`
    // even though they're meant to run as PlanAgent); otherwise read this
    // session's backend state. The `plan_context_locked` flag rides along
    // so configure_agent picks the right setter and the streaming loop's
    // mid-turn probe knows whether to leave the bundle alone.
    //
    // Plan's fixed platform contract and user/model-authored document occupy
    // separate slots. A mid-turn state flip can replace both without losing
    // caller framing, and adapters keep the document out of developer roles.
    let plan_context_locked = plan_context_override.is_some();
    let plan_resolved = match plan_context_override {
        Some(o) => o,
        None => crate::chat_engine::resolve_plan_context_for_session(&session_id).await,
    };

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
        foreground_stop_admission,
    )
    .await
    {
        Ok(coordinator) => coordinator,
        Err(error) => {
            let message = format!("Cannot initialize durable chat stream: {error}");
            let stopped_by_fence = error
                .to_string()
                .contains(session::FOREGROUND_STOP_FENCE_ERROR);
            let terminal_status = if stopped_by_fence {
                session::ChatTurnStatus::Interrupted
            } else {
                session::ChatTurnStatus::Failed
            };
            let interrupt_reason = if stopped_by_fence {
                session::ChatTurnInterruptReason::UserStop
            } else {
                session::ChatTurnInterruptReason::Unknown
            };
            if let Some(turn_id) = turn_id.as_deref() {
                if let Err(finish_error) = db.finish_chat_turn_once(
                    turn_id,
                    terminal_status,
                    Some(interrupt_reason),
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
                terminal_status,
                Some(interrupt_reason),
                Some(message.clone()),
            );
            stream_lifecycle.finish();
            return Err(if stopped_by_fence {
                ChatEngineFailure::cancelled(message)
            } else {
                message.into()
            });
        }
    };
    stream_lifecycle
        .arm_abandoned_recovery(db.clone(), durability.persistence_run_id().to_string());

    // Codex OAuth refresh can perform network I/O. Admit the durable run and
    // atomically resolve any prior SendUnknown request first, so an explicit
    // foreground retry cannot emit even an authentication request before its
    // ambiguous predecessor is terminalized as a brand-new manual request.
    // Callers may pass None; the on-disk token remains the shared source of
    // truth for desktop / HTTP / IM channel entry points.
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

    {
        // Title generation may spawn a one-shot model request. Keep it after
        // durable foreground admission/manual-retry convergence, while its
        // synchronous classification reads still use the blocking pool.
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

    // Durable basenames are owned by the already-persisted stream run. Crash
    // recovery reconciles this exact backend UUID prefix against every
    // durable Initial Context event for the run; Incognito writes no file.
    let mut durable_snapshot_names = Vec::new();
    if let Some((session_incognito, prepared)) = prepared_resource_mentions.as_mut() {
        if !*session_incognito {
            prepared.bind_persistence_run(durability.persistence_run_id())?;
            durable_snapshot_names = prepared.durable_snapshot_names()?;
        }
    }
    if !durable_snapshot_names.is_empty() {
        // Ownership must be durable before filesystem publication. The ledger
        // deliberately survives later run deletion, letting GC/edit retries
        // unlink the exact backend-minted basenames before acknowledging them.
        let ownership_db = db.clone();
        let ownership_run_id = durability.persistence_run_id().to_string();
        let ownership_session_id = session_id.clone();
        let ownership_snapshot_names = durable_snapshot_names.clone();
        ownership_db
            .run(move |db| {
                db.register_typed_resource_snapshots(
                    &ownership_run_id,
                    &ownership_session_id,
                    &ownership_snapshot_names,
                )
            })
            .await
            .map_err(|error| format!("Cannot register typed resource snapshots: {error}"))?;
    }

    let (published_attachments, frozen_resource_mentions) =
        if let Some((session_incognito, prepared)) = prepared_resource_mentions.take() {
            let publication_db = db.clone();
            let publication_run_id = durability.persistence_run_id().to_string();
            let snapshot_session_id = session_id.clone();
            let publication_snapshot_names = durable_snapshot_names;
            let mut snapshot_attachments = attachments;
            publication_db
                .run(move |db| {
                    let publish_files = || {
                        crate::attachments::publish_typed_resource_snapshot_files(
                            &snapshot_session_id,
                            prepared,
                            session_incognito,
                        )
                    };
                    let published = if session_incognito {
                        publish_files()?
                    } else {
                        db.publish_registered_typed_resource_snapshots(
                            &publication_run_id,
                            &snapshot_session_id,
                            &publication_snapshot_names,
                            publish_files,
                        )?
                    };
                    let frozen = crate::attachments::finalize_typed_resource_mentions(
                        published,
                        &mut snapshot_attachments,
                    );
                    anyhow::Ok((snapshot_attachments, frozen))
                })
                .await
                .map_err(|error| format!("Cannot publish typed resource mentions: {error}"))?
        } else {
            (attachments, Vec::new())
        };
    attachments = published_attachments;
    let frozen_resource_mentions = Arc::new(frozen_resource_mentions);
    let snapshot_names = frozen_resource_mentions
        .iter()
        .filter_map(|snapshot| snapshot.snapshot_name.clone())
        .collect::<Vec<_>>();
    let snapshot_refs_committed = Arc::new(std::sync::atomic::AtomicBool::new(
        snapshot_names.is_empty(),
    ));
    let _pending_snapshot_cleanup = PendingTypedResourceSnapshots {
        session_id: session_id.clone(),
        snapshot_names,
        refs_committed: snapshot_refs_committed.clone(),
    };

    let context_resource_turn_budget =
        Arc::new(crate::prompt_context::ContextResourceTurnBudget::default());
    let context_resource_refs = frozen_resource_mentions
        .iter()
        .filter_map(|snapshot| {
            let mention_id = incoming_turn.as_ref()?.mentions.iter().find(|mention| {
                matches!(
                    mention.kind,
                    crate::prompt_context::MentionKind::File
                        | crate::prompt_context::MentionKind::Plan
                ) && mention.target_id == snapshot.target_id
            })?;
            turn_context_builder.untrusted_data(
                crate::prompt_context::UntrustedDataSource::FileAttachment,
                serde_json::json!({
                    "mentionId": mention_id.id,
                    "resourceRef": snapshot.resource_ref,
                    "path": snapshot.target_id,
                    "sourceBytes": snapshot.source_bytes,
                    "continuationTool": crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
                })
                .to_string(),
            );
            Some(crate::prompt_context::ContextResourceRef {
                resource_ref: snapshot.resource_ref.clone(),
                mention_id: mention_id.id.clone(),
                target_id: snapshot.target_id.clone(),
                file_name: snapshot.file_name.clone(),
                mime_type: snapshot.mime_type.clone(),
                parent_session_id: session_id.clone(),
                parent_turn_id: turn_id.clone(),
                principal_agent_id: agent_id.clone(),
                bytes: snapshot.bytes.clone(),
                turn_budget: context_resource_turn_budget.clone(),
            })
        })
        .collect::<Vec<_>>();

    if incoming_turn.is_some() {
        for receipt in &mut mention_receipts {
            if !matches!(
                receipt.kind,
                crate::prompt_context::MentionKind::File | crate::prompt_context::MentionKind::Plan
            ) {
                continue;
            }
            let snapshot = frozen_resource_mentions
                .iter()
                .find(|snapshot| snapshot.target_id == receipt.target_id);
            receipt.status = if snapshot.is_some() {
                crate::prompt_context::MentionResolutionStatus::Resolved
            } else {
                crate::prompt_context::MentionResolutionStatus::Unavailable
            };
            receipt.materialization = snapshot.map(|snapshot| {
                crate::prompt_context::MentionMaterialization::FrozenSnapshot {
                    source_bytes: snapshot.source_bytes,
                    persistence: if snapshot.durable {
                        crate::prompt_context::ContextPersistence::DurableSnapshot
                    } else {
                        crate::prompt_context::ContextPersistence::IncognitoMemoryOnly
                    },
                }
            });
        }
    }

    // Wrap attachments in Arc<[T]> only after the staged typed-resource batch
    // has been published and its attachment paths/data have been frozen.
    // Failover closure clones are then pointer bumps even for MB-sized data.
    let attachments: std::sync::Arc<[crate::agent::Attachment]> = std::sync::Arc::from(attachments);

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

    // SessionStart hook (startup / resume). Observation output is frozen into
    // this turn's untrusted data envelope and survives failover retries (which
    // rebuild the agent from this same local). The helper is shared with ACP
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
            turn_context_builder.untrusted_data(
                crate::prompt_context::UntrustedDataSource::HookContext,
                extra,
            );
        }
    }

    // UserPromptSubmit hook context: the preflight chokepoint stashed any
    // `additionalContext` from the UserPromptSubmit hook keyed by session;
    // drain it here so it rides this turn's user-owned context next to SessionStart
    // (and survives failover for the same reason — it lives in this run-local).
    // Drained exactly once per turn.
    if let Some(extra) = crate::hooks::take_user_prompt_context(&session_id) {
        turn_context_builder.untrusted_data(
            crate::prompt_context::UntrustedDataSource::HookContext,
            extra,
        );
    }

    // Knowledge read bridge channel ① (D7): deterministically inject notes the
    // user referenced inline with `[[ ]]`, scoped by `effective_kb_access` (D10)
    // and wrapped as untrusted external data (#7). Skipped for incognito inside
    // the resolver (zero KB access).
    let bound_notes = incoming_turn
        .as_ref()
        .map(crate::prompt_context::bound_note_refs)
        .unwrap_or_default();
    let typed_notes = if !bound_notes.is_empty() {
        let per_note_budget = typed_note_byte_budget(&model_chain, &providers, bound_notes.len());
        crate::knowledge_hooks::resolve_bound_notes(
            &bound_notes,
            &session_id,
            kb_access_source(source),
            kb_origin,
            channel_kb_context.clone(),
            per_note_budget,
        )
    } else {
        None
    };
    let legacy_note_message = incoming_turn
        .as_ref()
        .map(|wire| message_without_typed_note_spans(&canonical_user_message, wire))
        .unwrap_or_else(|| canonical_user_message.clone());
    let legacy_note_slots = 5usize.saturating_sub(bound_notes.len().min(5));
    let legacy_notes = (legacy_note_slots > 0)
        .then(|| {
            crate::knowledge_hooks::resolve_inline_injections(
                &legacy_note_message,
                &session_id,
                kb_access_source(source),
                kb_origin,
                channel_kb_context.clone(),
                legacy_note_slots,
            )
        })
        .flatten();
    if legacy_notes.is_some() {
        // A current typed wire may legitimately coexist with the explicit
        // read-only `[[note]]` compatibility syntax (for example a typed
        // @agent plus a manually entered wikilink). Record that the legacy
        // parser actually contributed context.
        legacy_compatibility = true;
    }
    let referenced_notes = match (typed_notes, legacy_notes) {
        (Some(mut typed), Some(legacy)) => {
            typed.content.push_str("\n\n");
            typed.content.push_str(&legacy);
            Some(typed)
        }
        (Some(typed), None) => Some(typed),
        (None, Some(content)) => Some(crate::knowledge_hooks::ResolvedBoundNotes {
            content,
            resolved_refs: Vec::new(),
        }),
        (None, None) => None,
    };
    if let Some(resolved_notes) = referenced_notes {
        turn_context_builder.untrusted_data(
            crate::prompt_context::UntrustedDataSource::KnowledgeNote,
            resolved_notes.content,
        );
        for receipt in &mut mention_receipts {
            if receipt.kind == crate::prompt_context::MentionKind::Note
                && receipt
                    .target_id
                    .split_once("::")
                    .is_some_and(|(kb_id, rel_path)| {
                        resolved_notes.resolved_refs.iter().any(|resolved| {
                            resolved.kb_id == kb_id && resolved.rel_path == rel_path
                        })
                    })
            {
                receipt.status = crate::prompt_context::MentionResolutionStatus::Resolved;
                if let Some(resolved) = resolved_notes.resolved_refs.iter().find(|resolved| {
                    receipt
                        .target_id
                        .split_once("::")
                        .is_some_and(|(kb_id, rel_path)| {
                            resolved.kb_id == kb_id && resolved.rel_path == rel_path
                        })
                }) {
                    receipt.materialization =
                        Some(if resolved.source_bytes == resolved.delivered_bytes {
                            crate::prompt_context::MentionMaterialization::Complete {
                                source_bytes: resolved.source_bytes,
                                delivered_bytes: resolved.delivered_bytes,
                            }
                        } else {
                            crate::prompt_context::MentionMaterialization::Preview {
                                source_bytes: resolved.source_bytes,
                                delivered_bytes: resolved.delivered_bytes,
                                continuation_tool: crate::tools::TOOL_NOTE_READ.to_string(),
                            }
                        });
                }
            }
        }
    }

    // Raw/pasted markdown that merely resembles `@skill` or `@agent` remains
    // ordinary user text. Only a validated typed binding above may activate a
    // Skill or mint an opaque Agent reference. `[[note]]` is the deliberately
    // retained read-only legacy syntax handled by the knowledge bridge.

    crate::prompt_context::append_unresolved_mention_statuses(
        &mut turn_context_builder,
        &mention_receipts,
    );

    let resolved_turn_context = crate::prompt_context::finalize_turn_context(
        &canonical_user_message,
        turn_context_builder,
        agent_binding_refs,
        mention_wire_version,
        legacy_compatibility,
        mention_receipts,
    );
    message = resolved_turn_context.model_message.clone();
    let agent_binding_refs = resolved_turn_context.agent_bindings.clone();
    let prompt_context_receipt = std::sync::Arc::new(resolved_turn_context.receipt);

    // IM-mirror prefers the friendly `display_text` (e.g. `Using skill **X**...`
    // rendered for `/skill` invocations) so attached IM chats see what the
    // desktop user saw, not the internal structured turn envelope.
    // A normal Desktop / HTTP turn has a durable `turn_id`; the stream id is
    // the stable per-run fallback for internal callers that do not create a
    // chat-turn row. Pass it explicitly so the channel layer never has to
    // race the active-turn registry to infer this mirror generation.
    let im_mirror_generation = turn_id
        .clone()
        .map(crate::channel_hooks::ImLiveMirrorGeneration::Turn)
        .or_else(|| {
            stream_lifecycle
                .stream_id
                .clone()
                .map(crate::channel_hooks::ImLiveMirrorGeneration::Stream)
        })
        .unwrap_or_else(|| {
            crate::channel_hooks::ImLiveMirrorGeneration::Stream(
                durability.persistence_run_id().to_string(),
            )
        });
    let mut im_mirror = crate::channel_hooks::attach_live_mirror(
        &session_id,
        source,
        im_mirror_generation,
        Some(crate::channel_hooks::LastUserSnapshot {
            source: source.as_str().to_string(),
            text: crate::util::non_empty_trim_or(display_text.as_deref(), &canonical_user_message)
                .to_owned(),
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

    // A complete second pass is reserved for timeout/unknown failures that may
    // self-heal after every configured model has had a chance. Rate-limit and
    // overload already consume the larger per-profile retry budget and rotate
    // keys; auth/billing/model-not-found are deterministic. Never replay a
    // whole chain after any tool boundary, where another pass could duplicate
    // an external side effect.
    const MAX_MODEL_CHAIN_ROUNDS: u32 = 2;
    const MODEL_CHAIN_RETRY_BASE_MS: u64 = 4_000;
    const MODEL_CHAIN_RETRY_MAX_MS: u64 = 10_000;
    let mut model_chain_round = 1_u32;
    let mut model_index = 0_usize;
    // The initial attempt base is the pre-turn session context. A successful
    // Tier-4 recovery adopts a compacted post-user checkpoint instead; from
    // that point every retry and fallback model must preserve, not re-append,
    // the current user item.
    let mut current_user_message_state = CurrentUserMessageState::MissingFromHistory;

    loop {
        if model_index >= model_chain.len() {
            let can_retry_whole_chain = should_retry_model_chain(
                model_chain_round,
                MAX_MODEL_CHAIN_ROUNDS,
                last_reason,
                last_was_no_profile,
                compaction_failed.is_some(),
                durability.had_tool_activity(),
            ) && !cancel.load(std::sync::atomic::Ordering::SeqCst);
            if !can_retry_whole_chain {
                break;
            }

            let delay_ms = failover::retry_delay_ms(
                model_chain_round - 1,
                MODEL_CHAIN_RETRY_BASE_MS,
                MODEL_CHAIN_RETRY_MAX_MS,
            );
            let next_round = model_chain_round + 1;
            app_info!(
                "provider",
                "retry_chain",
                "Restarting model fallback chain for session {} (round {}/{}, delay={}ms)",
                session_id,
                next_round,
                MAX_MODEL_CHAIN_ROUNDS,
                delay_ms
            );
            let recovery_wait = crate::recovery_control::register(&session_id);
            if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                "type": "model_chain_retry",
                "reason": last_reason,
                "attempt": next_round,
                "total": MAX_MODEL_CHAIN_ROUNDS,
                "delay_ms": delay_ms,
                "recovery_id": recovery_wait.id(),
                "can_switch_model": false,
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
            match recovery_wait
                .wait(std::time::Duration::from_millis(delay_ms), Some(&cancel))
                .await
            {
                crate::recovery_control::RecoveryWaitOutcome::Cancelled => {
                    last_reason = None;
                    last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                    break;
                }
                crate::recovery_control::RecoveryWaitOutcome::Elapsed
                | crate::recovery_control::RecoveryWaitOutcome::SkipWait
                | crate::recovery_control::RecoveryWaitOutcome::SwitchModel => {}
            }
            model_chain_round = next_round;
            model_index = 0;
            continue;
        }

        let idx = model_index;
        let model_ref = &model_chain[idx];
        let mut manual_model_switch = false;
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
                    "Provider not found: {} for model {}",
                    model_ref.provider_id, model_ref.model_id
                );
                // A stale fallback is deterministic, but it must not erase a
                // transient failure from an earlier usable model. Reaching the
                // chain boundary should still give that model its bounded
                // second-round opportunity.
                last_reason = Some(chain_reason_after_missing_provider(last_reason));
                last_error = Some(msg);
                model_index += 1;
                continue;
            }
        };

        // Build the fallback event now, but enqueue it only after the next
        // attempt has been opened. Otherwise it would land in the previous
        // attempt and be correctly discarded together with that superseded
        // output during replay/materialization.
        let fallback_event_json = if idx > 0 {
            let display = format!("{} / {}", prov.name, model_ref.model_id);
            // `last_reason` is the executor's typed verdict. Display text is a
            // lossy fallback only for legacy paths that do not provide one;
            // in particular, typed ContextOverflow must not become Unknown
            // merely because free-text overflow matching is intentionally off.
            let reason_str = fallback_event_reason(last_reason, last_error.as_deref());
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

            let retry_model_display = format!("{} / {}", prov.name, model_ref.model_id);
            let can_switch_model = has_resolvable_fallback(&model_chain, &providers, idx);
            let on_retry = |progress: &RetryProgress| {
                app_info!(
                    "provider",
                    "retry",
                    "Retrying {}::{} after {:?} (attempt {}/{}, delay={}ms)",
                    model_provider_id,
                    model_id,
                    progress.reason,
                    progress.attempt,
                    progress.max_attempts,
                    progress.delay_ms
                );
                if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                    "type": "model_retry",
                    "provider_id": model_provider_id,
                    "model_id": model_id,
                    "model": retry_model_display,
                    "reason": progress.reason,
                    "attempt": progress.attempt,
                    "total": progress.max_attempts,
                    "delay_ms": progress.delay_ms,
                    "recovery_id": progress.recovery_id,
                    "can_switch_model": can_switch_model,
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
            let can_replay_operation = || !durability.had_tool_activity();

            // Capture refs / clones the closure needs. `move` consumes per-
            // call clones; the original chat_engine values stay borrowable
            // for the next compaction-retry iteration.
            let providers_ref = &providers;
            let compact_config_ref = &compact_config;
            let agent_id_ref = &agent_id;
            let session_id_ref = &session_id;
            let channel_kb_context_ref = &channel_kb_context;
            let run_context_ref = &run_context;
            let agent_binding_refs_ref = &agent_binding_refs;
            let context_resource_refs_ref = &context_resource_refs;
            let skill_allowed_tools_ref = &skill_allowed_tools;
            let plan_resolved_ref = &plan_resolved;
            let message_ref = &message;
            let canonical_user_message_ref = &canonical_user_message;
            let attachments_ref = &attachments;
            let effort_str_ref = &effort_str;
            let cancel_ref = &cancel;
            let event_sink_ref = &event_sink;
            let db_ref = &db;
            let model_ref_for_op = model_ref;
            let codex_token_ref = &codex_token;
            let durability_ref = durability.clone();
            let prompt_context_receipt_ref = prompt_context_receipt.clone();
            let frozen_resource_mentions_ref = frozen_resource_mentions.clone();
            let snapshot_refs_committed_ref = snapshot_refs_committed.clone();
            let fallback_event_ref = fallback_event_json.as_deref();
            let current_user_message_state_for_attempt = current_user_message_state;

            let exec_result = execute_with_failover_observed(
                prov,
                &session_id,
                FailoverPolicy::chat_engine_default().with_cancel(cancel.clone()),
                Some(&on_rotate),
                Some(&on_retry),
                Some(&can_replay_operation),
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
                    let run_context_owned = run_context_ref.clone();
                    let agent_bindings_owned = agent_binding_refs_ref.clone();
                    let context_resources_owned = context_resource_refs_ref.clone();
                    let skill_tools_owned = skill_allowed_tools_ref.clone();
                    let denied_tools_owned = denied_tools.clone();
                    let steer_run_id_owned = steer_run_id.clone();
                    let plan_resolved_owned = plan_resolved_ref.clone();
                    let channel_kb_context_owned = channel_kb_context_ref.clone();
                    let message_owned = message_ref.clone();
                    let canonical_user_message_owned = canonical_user_message_ref.clone();
                    // Arc<[Attachment]> clone is a pointer bump regardless
                    // of attachment size. See param destructure for the wrap.
                    let attachments_owned = attachments_ref.clone();
                    let effort_owned = effort_str_ref.clone();
                    let db_owned = db_ref.clone();
                    let provider_id_for_err = model_ref_for_op.provider_id.clone();
                    let model_id_for_err = model_ref_for_op.model_id.clone();
                    let codex_token_owned = codex_token_ref.clone();
                    let durability_owned = durability_ref.clone();
                    let prompt_context_receipt_owned = prompt_context_receipt_ref.clone();
                    let frozen_resource_mentions_owned = frozen_resource_mentions_ref.clone();
                    let snapshot_refs_committed_owned = snapshot_refs_committed_ref.clone();
                    let fallback_event_owned = fallback_event_ref.map(ToOwned::to_owned);
                    async move {
                        let provider_shape = match &prov.api_type {
                            ApiType::Anthropic => "anthropic",
                            ApiType::OpenaiChat => "openai_chat",
                            ApiType::OpenaiResponses => "openai_responses",
                            ApiType::Codex => "codex",
                        };
                        let attempt_no = durability_owned
                            .begin_attempt(
                                Some(&model_ref_for_op.provider_id),
                                Some(&model_ref_for_op.model_id),
                                Some(provider_shape),
                            )
                            .await?;
                        let current_user_message_state_for_op = if durability_owned
                            .attempt_base_contains_current_user()
                        {
                            CurrentUserMessageState::AlreadyInHistory
                        } else {
                            current_user_message_state_for_attempt
                        };
                        // Attempts are separate recovery prefixes. Re-commit a
                        // reference to the exact same frozen revision in every
                        // attempt so superseding attempt 1 cannot orphan the
                        // Agent/resource bindings. No resolver, Hook, or source
                        // read is repeated here.
                        let event = serde_json::json!({
                            "type": "initial_context_committed",
                            "revision": 0,
                            "attemptNo": attempt_no,
                            "replayed": attempt_no > 1,
                            "receipt": &*prompt_context_receipt_owned,
                            "agentBindings": &agent_bindings_owned,
                            // Compatibility key retained for journal/readers;
                            // v2 entries may represent typed Plan resources too.
                            "fileSnapshots": &*frozen_resource_mentions_owned,
                            "resourceSnapshotVersion": 2,
                            "skillAllowedTools": &skill_tools_owned,
                            "runContextSource": run_context_owned.as_ref().map(|context| context.source()),
                        })
                        .to_string();
                        durability_owned.accept_event(&event)?;
                        let source_journal_seq = durability_owned
                            .flush(crate::turn_durability::FlushReason::RoleSwitch)
                            .await?;
                        snapshot_refs_committed_owned
                            .store(true, std::sync::atomic::Ordering::Release);
                        if let (Some(turn_id), Some(projection)) = (
                            turn_id_for_cb.as_deref(),
                            crate::prompt_context::resolved_typed_mention_receipt_projection(
                                &canonical_user_message_owned,
                                &prompt_context_receipt_owned,
                                source_journal_seq,
                            ),
                        ) {
                            let receipt_db = db_owned.clone();
                            let receipt_session_id = session_for_cb.clone();
                            let receipt_turn_id = turn_id.to_string();
                            if let Err(error) = receipt_db
                                .run(move |db| {
                                    db.merge_chat_turn_typed_mention_receipt(
                                        &receipt_session_id,
                                        &receipt_turn_id,
                                        &projection,
                                    )
                                })
                                .await
                            {
                                // This projection is UI provenance, not model
                                // authority. A persistence failure must never
                                // fabricate a chip or fail an otherwise valid
                                // model turn; history simply has no receipt.
                                crate::app_warn!(
                                    "chat_engine",
                                    "typed_mention_receipt_projection",
                                    "failed to persist typed mention receipt: {}",
                                    error
                                );
                            }
                        }
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
                            turn_id_for_cb.as_deref(),
                            db_owned.clone(),
                            resolved_temperature,
                            run_context_owned.as_ref(),
                            &agent_bindings_owned,
                            &context_resources_owned,
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
                            Some(durability_owned.stop_admission()),
                            channel_kb_context_owned,
                        );
                        agent.set_retrieval_query(canonical_user_message_owned);
                        agent.set_turn_durability(durability_owned.clone());
                        restore_agent_context(&db_owned, &session_id_owned, &agent);

                        let history_len_before = agent.get_conversation_history().len();
                        let chat_start = std::time::Instant::now();
                        let allow_hard_cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
                        let allow_hard_cancel_for_cb = allow_hard_cancel.clone();

                        let mut chat_future = Box::pin(agent.chat_with_user_message_state(
                            &message_owned,
                            &attachments_owned,
                            current_user_message_state_for_op,
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
                            None => match tokio::time::timeout(
                                CHAT_CANCEL_COOPERATIVE_GRACE,
                                chat_future.as_mut(),
                            )
                            .await
                            {
                                Ok(result) => result,
                                Err(_) => {
                                    app_warn!(
                                        "chat",
                                        "cancel",
                                        "Force-dropping session {} model/tool loop after {}ms cancellation grace",
                                        session_id_owned,
                                        CHAT_CANCEL_COOPERATIVE_GRACE.as_millis()
                                    );
                                    Err(anyhow::anyhow!(CHAT_CANCELLED_BY_CALLER))
                                }
                            },
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
                                        request_plan: durability.interrupted_request_plan_commit(
                                            session::RequestPlanResponseOutcome::ResponseIncomplete,
                                        ),
                                    };
                                    let db_for_commit = db.clone();
                                    db_for_commit
                                        .run(move |db| db.commit_interrupted_turn(&commit))
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    durability
                                        .finalize_interrupted_request_after_turn_commit(
                                            session::RequestPlanResponseOutcome::ResponseIncomplete,
                                        )
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
                                    let _ = abort_im_mirror_after_internal_error(
                                        &mut im_mirror,
                                        &session_id,
                                        &message,
                                    );
                                    return Err(message.into());
                                }
                                durability.mark_interrupted(terminal.as_str());
                                let mirror_reason = mirror_reason_from_terminal_state(
                                    terminal,
                                    Some(interrupt),
                                    turn.error.as_deref(),
                                );
                                stream_lifecycle.set_terminal(
                                    terminal,
                                    Some(interrupt),
                                    turn.error.clone(),
                                );
                                stream_lifecycle.finish();
                                schedule_browser_turn_finalize(source, &session_id);
                                let _ = abort_im_mirror_in_background(
                                    &mut im_mirror,
                                    &session_id,
                                    &mirror_reason,
                                );
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
                        if let Err(error) = durability.flush(FlushReason::FinalEnd).await {
                            let message = format!("pre-final durability barrier failed: {error}");
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
                        if let Err(error) = durability.reconcile_spool_to_sqlite().await {
                            let message = format!("pre-final spool import failed: {error}");
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
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
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
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
                        let _ = abort_im_mirror_after_internal_error(
                            &mut im_mirror,
                            &session_id,
                            &message,
                        );
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
                        crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_CHAT);
                    if let Some(input_tokens) = usage.input_tokens {
                        ledger_event.input_tokens = Some(input_tokens.max(0) as u64);
                        ledger_event.cache_creation_input_tokens = usage
                            .cache_creation_input_tokens
                            .map(|value| value.max(0) as u64);
                        ledger_event.cache_read_input_tokens = usage
                            .cache_read_input_tokens
                            .map(|value| value.max(0) as u64);
                        ledger_event.context_input_tokens = usage
                            .context_input_tokens
                            .or(usage.input_tokens)
                            .map(|value| value.max(0) as u64);
                        ledger_event.fresh_input_tokens = usage
                            .fresh_input_tokens
                            .or(usage.input_tokens)
                            .map(|value| value.max(0) as u64);
                    }
                    ledger_event.output_tokens =
                        usage.output_tokens.map(|value| value.max(0) as u64);
                    ledger_event.metadata = Some(serde_json::json!({
                        "tokenAccounting": {
                            "inputCoverage": usage.input_coverage,
                            "outputCoverage": usage.output_coverage,
                            "observations": usage.token_accounting_observations,
                        }
                    }));
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

                    let context_json =
                        match serde_json::to_string(&agent.get_conversation_history()) {
                            Ok(context_json) => context_json,
                            Err(error) => {
                                let message = format!("serialize final context failed: {error}");
                                let _ = abort_im_mirror_after_internal_error(
                                    &mut im_mirror,
                                    &session_id,
                                    &message,
                                );
                                return Err(message.into());
                            }
                        };
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
                        tier3_recovery: if agent.tier3_summary_applied_this_turn() {
                            session::Tier3RecoveryCommit::ClearAfterSummary
                        } else {
                            session::Tier3RecoveryCommit::Unchanged
                        },
                        request_plan: durability.successful_request_plan_commit()?,
                    };
                    let committed = {
                        let db = db.clone();
                        db.run(move |db| db.commit_assistant_turn(&commit)).await
                    };
                    let committed = match committed {
                        Ok(committed) => committed,
                        Err(_) if cancel.load(std::sync::atomic::Ordering::SeqCst) => {
                            // Stop may win after the final in-memory cancel
                            // check but before the atomic success transaction.
                            // The DB refuses to overwrite `cancelling`; converge
                            // the durable journal through the normal UserStop
                            // finalizer instead of misclassifying that CAS as a
                            // persistence failure.
                            last_reason = None;
                            last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                            last_was_no_profile = false;
                            break;
                        }
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
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
                    };
                    durability
                        .finalize_successful_request_after_turn_commit()
                        .await?;
                    let assistant_id = Some(committed.assistant_message_id);
                    durability.mark_committed(committed.committed_seq);

                    // GUI / HTTP turns mirror into the attached IM chat via
                    // the live stream sink. Kick the final IM flush before
                    // ending the frontend lifecycle and before running
                    // post-turn side effects so title/memory work cannot
                    // delay the remote chat's finalization. It runs in the
                    // background so slow IM network calls never hold the GUI
                    // path open.
                    let _ = finalize_im_mirror_in_background(&mut im_mirror, response.clone());

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
                    // `response` is the turn's final assistant text
                    // (`last_assistant_message`), so a Stop hook can inspect it.
                    crate::hooks::fire_stop(
                        &session_id,
                        Some(&agent_id),
                        terminal_status.as_str(),
                        Some(&response),
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
                            let round_tokens = usage_snapshot
                                .best_effort_total_tokens()
                                .min(u64::from(u32::MAX))
                                as u32;
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
                                u.best_effort_total_tokens().min(usize::MAX as u64) as usize
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
                            // 闸 1 起的整条瀑布（trigger → spawn(run_review_cycle)
                            // → sweep_stale）在 ha-skills；kernel 只算这四个
                            // 信号标量——`user_correction` 需要 SessionDB。
                            crate::skills_hooks::auto_review_post_turn(
                                &session_id,
                                &cfg,
                                round_tokens,
                                round_messages,
                                tool_use_count,
                                user_correction,
                            );
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

                Err(ExecutorError::NeedsCompaction {
                    last_profile,
                    evidence,
                }) => {
                    if !evidence.is_high_confidence() {
                        let msg = format!(
                            "Refusing emergency compaction without high-confidence overflow evidence: {evidence:?}"
                        );
                        app_warn!("context", "compact_evidence", "{}", msg);
                        last_reason = Some(failover::FailoverReason::Unknown);
                        last_error = Some(msg);
                        break;
                    }
                    let capacity_proof = match &evidence {
                        failover::ContextOverflowEvidence::LocalPreflight {
                            input_tokens,
                            max_input_tokens,
                            capacity_proof: Some(proof),
                            ..
                        } if proof.original_local_upper_bound == *input_tokens
                            && proof.max_input_tokens == *max_input_tokens =>
                        {
                            proof.clone()
                        }
                        failover::ContextOverflowEvidence::LocalPreflight { .. }
                        | failover::ContextOverflowEvidence::StructuredProvider { .. }
                        | failover::ContextOverflowEvidence::TextHint { .. } => {
                            let msg = format!(
                                "Refusing emergency compaction on {}::{} without an immutable complete-request capacity proof",
                                model_ref.provider_id, model_ref.model_id,
                            );
                            app_warn!("context", "compact_capacity_unproven", "{}", msg);
                            last_reason = Some(failover::FailoverReason::ContextOverflow);
                            last_error = Some(msg.clone());
                            compaction_failed.get_or_insert(msg);
                            break;
                        }
                    };
                    // From this point onward the initiating failure is a typed,
                    // high-confidence overflow. Recovery-step display errors
                    // may add detail, but must not erase that verdict.
                    last_reason = Some(failover::FailoverReason::ContextOverflow);
                    if let Some((status, interrupt, error)) =
                        terminal_turn_state(&db, turn_id.as_deref())
                    {
                        let mirror_reason =
                            mirror_reason_from_terminal_state(status, interrupt, error.as_deref());
                        stream_lifecycle.set_terminal(status, interrupt, error);
                        stream_lifecycle.finish();
                        schedule_browser_turn_finalize(source, &session_id);
                        let _ = abort_im_mirror_in_background(
                            &mut im_mirror,
                            &session_id,
                            &mirror_reason,
                        );
                        return Ok(ChatEngineResult {
                            response: String::new(),
                            model_used: Some(model_ref.clone()),
                            usage: Default::default(),
                            agent: None,
                        });
                    }

                    if durability.had_non_replayable_tool_activity() {
                        let msg = format!(
                            "Context overflow on {}::{} after non-replayable tool activity; refusing to replay the turn",
                            model_ref.provider_id, model_ref.model_id
                        );
                        app_warn!("provider", "recovery_blocked", "{}", msg);
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg);
                        break;
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
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
                        break;
                    }
                    compaction_attempts += 1;

                    app_info!(
                        "context",
                        "compact",
                        "Context overflow on {}::{}, attempting emergency compaction (evidence={:?})",
                        model_ref.provider_id,
                        model_ref.model_id,
                        evidence
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
                            last_reason = Some(failover::FailoverReason::ContextOverflow);
                            last_error = Some(msg);
                            break;
                        }
                    };
                    configure_agent(
                        &mut compact_agent,
                        &agent_id,
                        &session_id,
                        turn_id.as_deref(),
                        db.clone(),
                        resolved_temperature,
                        run_context.as_ref(),
                        &agent_binding_refs,
                        &context_resource_refs,
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
                        Some(durability.stop_admission()),
                        channel_kb_context.clone(),
                    );
                    restore_agent_context(&db, &session_id, &compact_agent);

                    let mut history = compact_agent.get_conversation_history();
                    let original_history_for_capacity =
                        crate::context_compact::prepare_messages_for_api(&history);
                    // Capture the exact provider-native item before destructive
                    // projection. A text comparison is insufficient here because
                    // attachments and provider metadata are part of the request.
                    let current_user_anchor: Option<
                        crate::context_compact::LatestUserRequestAnchor,
                    > = crate::context_compact::latest_user_request_anchor(&history);
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
                    if !current_user_anchor
                        .as_ref()
                        .is_some_and(|anchor| anchor.is_preserved_exactly_once(&history))
                    {
                        let msg = format!(
                            "Emergency compaction could not preserve the current user request exactly once on {}::{}; refusing to publish or retry",
                            model_ref.provider_id, model_ref.model_id,
                        );
                        app_warn!("context", "compact_user_anchor_lost", "{}", msg);
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
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
                        break;
                    }
                    if compact_result.messages_affected == 0
                        || compact_result.tokens_after >= compact_result.tokens_before
                    {
                        let msg = format!(
                            "Emergency compaction made no measurable progress on {}::{} (before={}, after={}, affected={}); refusing to publish or retry the same oversized request",
                            model_ref.provider_id,
                            model_ref.model_id,
                            compact_result.tokens_before,
                            compact_result.tokens_after,
                            compact_result.messages_affected,
                        );
                        app_warn!("context", "compact_no_progress", "{}", msg);
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
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
                        break;
                    }
                    let compacted_history_for_capacity =
                        crate::context_compact::prepare_messages_for_api(&history);
                    let projected_input_upper = match crate::token_accounting::service()
                        .verify_compacted_capacity(
                            &capacity_proof,
                            &original_history_for_capacity,
                            &compacted_history_for_capacity,
                        ) {
                        Ok(projected_input_upper) => projected_input_upper,
                        Err(error) => {
                            let msg = format!(
                                "Emergency compaction capacity proof failed on {}::{}: {}; refusing to publish or retry",
                                model_ref.provider_id, model_ref.model_id, error,
                            );
                            app_warn!("context", "compact_capacity_unproven", "{}", msg);
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
                            last_reason = Some(failover::FailoverReason::ContextOverflow);
                            last_error = Some(msg.clone());
                            compaction_failed.get_or_insert(msg);
                            break;
                        }
                    };
                    app_info!(
                        "context",
                        "compact_capacity_proven",
                        "Emergency compaction complete-request capacity proven on {}::{}: input_upper={} max_input={}",
                        model_ref.provider_id,
                        model_ref.model_id,
                        projected_input_upper,
                        capacity_proof.max_input_tokens,
                    );
                    compact_agent.set_conversation_history(history);
                    if let Some((status, interrupt, error)) =
                        terminal_turn_state(&db, turn_id.as_deref())
                    {
                        let mirror_reason =
                            mirror_reason_from_terminal_state(status, interrupt, error.as_deref());
                        stream_lifecycle.set_terminal(status, interrupt, error);
                        stream_lifecycle.finish();
                        schedule_browser_turn_finalize(source, &session_id);
                        let _ = abort_im_mirror_in_background(
                            &mut im_mirror,
                            &session_id,
                            &mirror_reason,
                        );
                        return Ok(ChatEngineResult {
                            response: String::new(),
                            model_used: Some(model_ref.clone()),
                            usage: Default::default(),
                            agent: None,
                        });
                    }
                    let compact_history = compact_agent.get_conversation_history();
                    if let Err(error) = durability
                        .checkpoint_emergency_context(
                            &compact_history,
                            durability.context_revision(),
                        )
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
                    current_user_message_state = CurrentUserMessageState::AlreadyInHistory;

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

                Err(ExecutorError::SwitchModel {
                    last_reason: r,
                    last_error: err_str,
                }) => {
                    app_info!(
                        "provider",
                        "manual_model_switch",
                        "Skipping remaining retries for {}::{} at user request",
                        model_ref.provider_id,
                        model_ref.model_id
                    );
                    last_reason = Some(r);
                    last_error = Some(err_str);
                    last_was_no_profile = false;
                    manual_model_switch = true;
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

        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        // Every model/profile retry rebuilds the Agent from the turn's stable
        // base context. Once a tool ran, doing so would replay its external
        // side effects instead of resuming after its result.
        if durability.had_non_replayable_tool_activity() {
            app_warn!(
                "provider",
                "recovery_blocked",
                "Not switching models for session {} after tool activity",
                session_id
            );
            break;
        }

        if last_reason.is_some_and(|reason| reason.is_terminal()) {
            break;
        }

        model_index += 1;
        // "Switch model" means leave the current model immediately. If there
        // is no later configured model, do not reinterpret it as permission to
        // restart the same chain from its first model.
        if model_index >= model_chain.len() && manual_model_switch {
            break;
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
            request_plan: durability.interrupted_request_plan_commit(
                if terminal_interrupt == session::ChatTurnInterruptReason::UserStop {
                    session::RequestPlanResponseOutcome::CancelledAfterResponse
                } else {
                    session::RequestPlanResponseOutcome::ResponseIncomplete
                },
            ),
        };
        let db_for_commit = db.clone();
        db_for_commit
            .run(move |db| db.commit_interrupted_turn(&commit))
            .await?;
        durability
            .finalize_interrupted_request_after_turn_commit(
                if terminal_interrupt == session::ChatTurnInterruptReason::UserStop {
                    session::RequestPlanResponseOutcome::CancelledAfterResponse
                } else {
                    session::RequestPlanResponseOutcome::ResponseIncomplete
                },
            )
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
    let _ = abort_im_mirror_in_background(&mut im_mirror, &session_id, &reason);
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
    let (failure_kind, failure_reason, failure_is_codex_auth) =
        classify_chat_engine_failure(&reason);
    Err(ChatEngineFailure::classified(
        failure_kind,
        failure_reason,
        failure_is_codex_auth,
        final_error,
    ))
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

fn classify_chat_engine_failure(
    reason: &TerminationReason,
) -> (
    ChatEngineFailureKind,
    Option<failover::FailoverReason>,
    bool,
) {
    match reason {
        TerminationReason::UserStop | TerminationReason::RuntimeCancel => {
            (ChatEngineFailureKind::Cancelled, None, false)
        }
        TerminationReason::ProviderFailed {
            last_kind,
            is_codex_auth,
            ..
        } => (
            if last_kind.is_terminal() {
                ChatEngineFailureKind::Terminal
            } else {
                ChatEngineFailureKind::ProviderExhausted
            },
            Some(*last_kind),
            *is_codex_auth,
        ),
        TerminationReason::NoProfileAvailable => {
            (ChatEngineFailureKind::ProviderExhausted, None, false)
        }
        // Preserve the typed overflow class for structured consumers even
        // though a failed recovery operation remains an infrastructure-class
        // engine outcome rather than a provider-chain exhaustion.
        TerminationReason::CompactionFailed { .. } => (
            ChatEngineFailureKind::Infrastructure,
            Some(failover::FailoverReason::ContextOverflow),
            false,
        ),
        TerminationReason::Other { .. }
        | TerminationReason::Shutdown
        | TerminationReason::Crash => (ChatEngineFailureKind::Infrastructure, None, false),
    }
}

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
        ChatSource::SessionTool => KbAccessSource::Other,
        ChatSource::Cron => KbAccessSource::Cron,
        ChatSource::Acp => KbAccessSource::Other,
    }
}

fn tool_turn_provenance(source: stream_seq::ChatSource) -> crate::tool_defs::ToolTurnProvenance {
    use crate::tool_defs::ToolTurnProvenance;
    if source.carries_foreground_user_intent() {
        ToolTurnProvenance::ForegroundUser
    } else {
        ToolTurnProvenance::Autonomous
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
    // 特征钩子（未 wire no-op：无 extension tab 可 finalize；wrapper 首次未
    // 命中打一次 warn，避免每轮刷屏）。
    crate::browser_hooks::schedule_turn_finalize(session_id);
}

/// Apply common agent configuration. Extracted to avoid duplication between
/// initial agent setup and profile-rotation rebuild.
///
/// `plan_resolved` is the full Plan-mode bundle (state + mode + allow_paths,
/// fixed run instruction, and user/model-authored plan data). `plan_locked`
/// so the streaming loop's mid-turn probe knows whether it's free to re-sync.
#[allow(clippy::too_many_arguments)]
fn configure_agent(
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
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
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
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
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

        let current_group = mirror_reason_from_terminal_state(
            session::ChatTurnStatus::Failed,
            Some(session::ChatTurnInterruptReason::CurrentToolGroupOverflow),
            Some("display text intentionally has no classifier token"),
        );
        assert!(matches!(
            current_group,
            TerminationReason::ProviderFailed {
                last_kind: failover::FailoverReason::CurrentToolGroupOverflow,
                is_codex_auth: false,
                ..
            }
        ));

        let dispatch_unknown = mirror_reason_from_terminal_state(
            session::ChatTurnStatus::Failed,
            Some(session::ChatTurnInterruptReason::DispatchUnknown),
            Some("display text intentionally has no classifier token"),
        );
        assert!(matches!(
            dispatch_unknown,
            TerminationReason::ProviderFailed {
                last_kind: failover::FailoverReason::DispatchUnknown,
                is_codex_auth: false,
                ..
            }
        ));
    }

    #[test]
    fn current_tool_group_overflow_survives_as_typed_terminal_failure() {
        let termination = TerminationReason::ProviderFailed {
            last_kind: failover::FailoverReason::CurrentToolGroupOverflow,
            last_message: "display text intentionally has no classifier token".to_string(),
            is_codex_auth: false,
        };
        assert_eq!(
            termination.to_chat_turn_interrupt_reason(),
            session::ChatTurnInterruptReason::CurrentToolGroupOverflow
        );
        let (kind, reason, is_codex_auth) = classify_chat_engine_failure(&termination);
        let failure = ChatEngineFailure::classified(
            kind,
            reason,
            is_codex_auth,
            "display text intentionally has no classifier token",
        );

        assert_eq!(failure.kind(), ChatEngineFailureKind::Terminal);
        assert_eq!(
            failure.reason(),
            Some(failover::FailoverReason::CurrentToolGroupOverflow)
        );
        assert!(!failure.is_codex_auth());
        assert_eq!(
            failover::classify_error(&failure.to_string()),
            failover::FailoverReason::Unknown,
            "the typed outcome must not depend on classifying display text"
        );
    }

    #[test]
    fn dispatch_unknown_survives_as_typed_terminal_failure() {
        let termination = TerminationReason::ProviderFailed {
            last_kind: failover::FailoverReason::DispatchUnknown,
            last_message: "display text intentionally has no classifier token".to_string(),
            is_codex_auth: false,
        };
        assert_eq!(
            termination.to_chat_turn_interrupt_reason(),
            session::ChatTurnInterruptReason::DispatchUnknown
        );
        let (kind, reason, is_codex_auth) = classify_chat_engine_failure(&termination);
        let failure = ChatEngineFailure::classified(
            kind,
            reason,
            is_codex_auth,
            "display text intentionally has no classifier token",
        );

        assert_eq!(failure.kind(), ChatEngineFailureKind::Terminal);
        assert_eq!(
            failure.reason(),
            Some(failover::FailoverReason::DispatchUnknown)
        );
        assert!(!failure.is_codex_auth());
        assert_eq!(
            failover::classify_error(&failure.to_string()),
            failover::FailoverReason::Unknown,
            "the typed outcome must not depend on classifying display text"
        );
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
        assert_eq!(
            chain_reason_after_missing_provider(Some(failover::FailoverReason::ContextOverflow)),
            failover::FailoverReason::ContextOverflow,
        );
    }

    #[test]
    fn fallback_event_prefers_typed_context_overflow_over_display_text() {
        assert_eq!(
            fallback_event_reason(
                Some(failover::FailoverReason::ContextOverflow),
                Some("Context overflow after emergency compaction"),
            ),
            failover::FailoverReason::ContextOverflow,
        );
        assert_eq!(
            fallback_event_reason(None, Some("429 rate limit")),
            failover::FailoverReason::RateLimit,
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
        let delta = serde_json::to_string(text).expect("serialize SSE text delta");
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":{}}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            delta
        )
    }

    fn valid_compaction_summary() -> &'static str {
        concat!(
            "## Primary Request and Success Criteria\nsummary ok\n\n",
            "## Current Execution State\nsummary ok\n\n",
            "## Decisions and Rationale\nsummary ok\n\n",
            "## Files, Symbols, and Artifacts\nsummary ok\n\n",
            "## Tool Results Worth Preserving\nsummary ok\n\n",
            "## Errors, Failed Attempts, and Fixes\nsummary ok\n\n",
            "## User Feedback and Constraints\nsummary ok\n\n",
            "## Pending Work and Next Action\nsummary ok\n\n",
            "## Trust Boundaries and Security Notes\nsummary ok",
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

    fn current_user_occurrences_in_responses_body(body: &serde_json::Value, prompt: &str) -> usize {
        fn content_occurrences(content: &serde_json::Value, prompt: &str) -> usize {
            match content {
                serde_json::Value::String(text) => text.matches(prompt).count(),
                serde_json::Value::Array(parts) => parts
                    .iter()
                    .map(|part| content_occurrences(part, prompt))
                    .sum(),
                serde_json::Value::Object(fields) => fields
                    .get("text")
                    .map(|text| content_occurrences(text, prompt))
                    .unwrap_or(0),
                _ => 0,
            }
        }

        body.get("input")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("role").and_then(serde_json::Value::as_str) == Some("user"))
            .map(|item| content_occurrences(&item["content"], prompt))
            .sum()
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
            incoming_turn: None,
            display_text: None,
            attachments: Vec::new(),
            session_db: db,
            model_chain,
            providers,
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
            ui_surface: None,
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
    async fn local_preflight_emergency_retry_sends_current_user_exactly_once() {
        let (_lock, _dir, db) = temp_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let prompt = "TIER4_CURRENT_USER_SENTINEL_73f2";
        db.append_message(&session.id, &NewMessage::user(prompt))
            .unwrap();
        db.save_context(
            &session.id,
            &serde_json::json!([
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "old request"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "x".repeat(120_000)
                    }]
                }
            ])
            .to_string(),
        )
        .unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_text_then_done("recovered answer")),
            )
            .mount(&server)
            .await;

        let mut provider = openai_provider(server.uri(), "m1");
        provider.models[0].context_window = 32_000;
        let model = ActiveModel {
            provider_id: provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let mut chat_params = params(db.clone(), session.id.clone(), vec![model], vec![provider]);
        chat_params.message = prompt.to_string();
        // Keep this contract focused on the reactive Tier 4 path. The old
        // history intentionally exceeds the local complete-request capacity;
        // proactive Tier 3 is disabled so the immutable preflight certificate
        // drives the recovery.
        chat_params.compact_config.enabled = false;

        let result = run_chat_engine(chat_params)
            .await
            .expect("Tier-4 retry should recover");
        assert_eq!(result.response, "recovered answer");

        let requests = server.received_requests().await.unwrap();
        let provider_bodies = requests
            .iter()
            .filter(|request| request.url.path() == "/v1/responses")
            .map(|request| {
                serde_json::from_slice::<serde_json::Value>(&request.body)
                    .expect("Responses request body")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            provider_bodies.len(),
            1,
            "the oversized first attempt is rejected locally; only the proven retry reaches the Provider"
        );
        assert_eq!(
            provider_bodies
                .iter()
                .map(|body| current_user_occurrences_in_responses_body(body, prompt))
                .collect::<Vec<_>>(),
            vec![1],
            "the retry base already contains the current user and must not append it again"
        );
        let final_history = serde_json::json!({
            "input": result
                .agent
                .as_ref()
                .expect("successful retry agent")
                .get_conversation_history()
        });
        assert_eq!(
            current_user_occurrences_in_responses_body(&final_history, prompt),
            1,
            "the successful retry must keep a single canonical user item"
        );
        assert_eq!(
            db.tier3_recovery_state(&session.id).unwrap(),
            Some(crate::session::Tier3RecoveryState::Required),
            "the same-turn Tier-4 retry must leave forced Tier 3 for the next user turn"
        );

        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({
                        "output": [{
                            "type": "message",
                            "content": [{
                                "type": "output_text",
                                "text": valid_compaction_summary(),
                            }],
                        }],
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 1,
                        },
                    })),
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
                    .set_body_string(sse_text_then_done("answer after forced summary")),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let next_prompt = "NEXT_USER_TURN_SENTINEL_96c1";
        db.append_message(&session.id, &NewMessage::user(next_prompt))
            .unwrap();
        let mut next_provider = openai_provider(server.uri(), "m1");
        next_provider.models[0].context_window = 32_000;
        let next_model = ActiveModel {
            provider_id: next_provider.id.clone(),
            model_id: "m1".to_string(),
        };
        let mut next_params = params(
            db.clone(),
            session.id.clone(),
            vec![next_model],
            vec![next_provider],
        );
        next_params.message = next_prompt.to_string();
        next_params.compact_config.enabled = false;

        let next_result = run_chat_engine(next_params)
            .await
            .expect("the next user turn should publish forced Tier 3 before its main request");
        assert_eq!(next_result.response, "answer after forced summary");

        let next_requests = server.received_requests().await.unwrap();
        let next_provider_bodies = next_requests
            .iter()
            .filter(|request| request.url.path() == "/v1/responses")
            .map(|request| {
                serde_json::from_slice::<serde_json::Value>(&request.body)
                    .expect("Responses request body")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            next_provider_bodies.len(),
            2,
            "the next turn must send exactly one forced summary and one main request"
        );
        assert_eq!(
            next_provider_bodies
                .iter()
                .filter(|body| {
                    body.get("instructions")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|instructions| {
                            instructions.contains("context compaction assistant")
                        })
                })
                .count(),
            1,
            "the durable recovery marker authorizes one forced summary attempt"
        );
        assert_eq!(
            next_provider_bodies
                .iter()
                .map(|body| current_user_occurrences_in_responses_body(body, next_prompt))
                .sum::<usize>(),
            1,
            "only the main request may contain the next turn's current user"
        );
        assert_eq!(
            db.tier3_recovery_state(&session.id).unwrap(),
            None,
            "publishing the winning Tier 3 summary must atomically clear the marker"
        );
        assert!(
            db.load_context(&session.id)
                .unwrap()
                .is_some_and(|context| context.contains("summary ok")),
            "the cleared marker must have a durable summarized context winner"
        );
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
