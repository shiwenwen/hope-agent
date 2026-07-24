//! Unified turn-finalize system.
//!
//! Single entry point [`finalize_turn_context`] that converges every
//! "non-natural completion" path (user stop / provider failure /
//! compaction give-up / graceful shutdown / crash / no profile) into
//! three coherent outputs:
//!
//! 1. `context_json` — model-facing history with a rebuilt
//!    provider-native partial round + a `[系统事件]` marker so the
//!    model can perceive what happened.
//! 2. `messages` table `role=event` row — user-facing notice rendered
//!    by the GUI's existing event banner pipeline.
//! 3. IM channel notice (when attached) — reuses the existing
//!    `im_error_message` copy bank.
//!
//! Subagent runs keep their `abort_on_cancel=true` return/error semantics, but
//! the durability coordinator still atomically preserves every acknowledged
//! partial/tool record.

pub mod copy;
pub mod rebuild;
pub mod sentinel;

use serde::{Deserialize, Serialize};

use crate::failover::FailoverReason;
use crate::session::{ChatTurnInterruptReason, ChatTurnStatus};

pub use sentinel::StartupCause;

// ── ProviderApiKind ───────────────────────────────────────────────────
//
// Lightweight provider-shape tag for partial reconstruction. Decoupled
// from `crate::agent::types::LlmProvider` so the finalize path doesn't
// need to carry API keys / OAuth tokens around just to know which
// history shape to emit.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderApiKind {
    Anthropic,
    OpenAIChat,
    OpenAIResponses,
    Codex,
}

impl ProviderApiKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAIChat => "openai_chat",
            Self::OpenAIResponses => "openai_responses",
            Self::Codex => "codex",
        }
    }

    /// Parse the stable provider-shape tag stored in stream attempts.
    pub fn from_shape(shape: &str) -> Option<Self> {
        match shape.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Some(Self::Anthropic),
            "openai_chat" | "openai-chat" | "openai" => Some(Self::OpenAIChat),
            "openai_responses" | "openai-responses" | "responses" => Some(Self::OpenAIResponses),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

impl From<crate::provider::ApiType> for ProviderApiKind {
    fn from(api: crate::provider::ApiType) -> Self {
        match api {
            crate::provider::ApiType::Anthropic => Self::Anthropic,
            crate::provider::ApiType::OpenaiChat => Self::OpenAIChat,
            crate::provider::ApiType::OpenaiResponses => Self::OpenAIResponses,
            crate::provider::ApiType::Codex => Self::Codex,
        }
    }
}

// ── TerminationReason ─────────────────────────────────────────────────

/// All non-natural turn endings the finalize path handles.
///
/// `Shutdown` covers both `SIGTERM`/`SIGINT` graceful exit and dev-mode
/// hot-reload — user-visible behavior is identical (the app is going
/// down with partial preserved).
#[derive(Debug, Clone)]
pub enum TerminationReason {
    /// User pressed the stop button in GUI/HTTP/IM.
    UserStop,
    /// The owning runtime/request task was dropped (for example an HTTP
    /// client disconnected) before the engine returned normally.
    RuntimeCancel,
    /// No usable auth profile (all disabled, cooled-down, or missing).
    /// Zero API calls were attempted — this is a configuration error.
    NoProfileAvailable,
    /// All model_chain attempts failed. `last_kind` is the
    /// `FailoverReason` of the final attempt; `last_message` is the
    /// raw error text (already redacted by caller); `is_codex_auth`
    /// triggers the Codex re-auth UI hint.
    ProviderFailed {
        last_kind: FailoverReason,
        last_message: String,
        is_codex_auth: bool,
    },
    /// Emergency context compaction was attempted but the history
    /// still exceeds the hard threshold — model cannot continue.
    CompactionFailed { detail: String },
    /// `SIGTERM`/`SIGINT` graceful exit, including dev-mode hot reload.
    /// User-facing copy is identical for both.
    Shutdown,
    /// Panic, SIGKILL, OOM kill, power loss — the previous process did
    /// not get to run signal handlers. Detected on next launch by
    /// absence of the shutdown sentinel.
    Crash,
    /// Catch-all for unexpected internal errors that don't fit a
    /// specific class — DB write failures, serialization bugs,
    /// internal panics caught by `catch_unwind`, etc.
    Other { message: String },
}

impl TerminationReason {
    /// Map this reason into the `chat_turns.status` value to persist.
    ///
    /// `Interrupted` covers reasons where the user's mental model is
    /// "stopped" rather than "errored" — explicit cancel, graceful
    /// shutdown, and crash (process exit). Everything else (provider
    /// failures, configuration errors, compaction give-up) surfaces as
    /// `Failed` with an error string attached.
    pub fn to_chat_turn_status(&self) -> ChatTurnStatus {
        match self {
            Self::UserStop | Self::RuntimeCancel | Self::Shutdown | Self::Crash => {
                ChatTurnStatus::Interrupted
            }
            _ => ChatTurnStatus::Failed,
        }
    }

    /// Map this reason into the `chat_turns.interrupt_reason` value.
    pub fn to_chat_turn_interrupt_reason(&self) -> ChatTurnInterruptReason {
        match self {
            Self::UserStop => ChatTurnInterruptReason::UserStop,
            Self::RuntimeCancel => ChatTurnInterruptReason::RuntimeCancel,
            Self::Shutdown => ChatTurnInterruptReason::Shutdown,
            Self::Crash => ChatTurnInterruptReason::CrashRecovery,
            Self::NoProfileAvailable => ChatTurnInterruptReason::NoProfile,
            Self::ProviderFailed { .. } => ChatTurnInterruptReason::ProviderFailed,
            Self::CompactionFailed { .. } => ChatTurnInterruptReason::CompactionFailed,
            Self::Other { .. } => ChatTurnInterruptReason::Unknown,
        }
    }

    /// Only `UserStop` is initiated by the user; everything else is
    /// surfaced as an error event in the UI (`is_error = true`).
    pub fn is_user_initiated(&self) -> bool {
        matches!(self, Self::UserStop)
    }

    /// Error text persisted into `chat_turns.error`. `None` for
    /// non-error reasons so the column stays NULL.
    pub fn to_error_text(&self) -> Option<String> {
        match self {
            Self::UserStop | Self::RuntimeCancel | Self::Shutdown | Self::Crash => None,
            Self::NoProfileAvailable => Some("no auth profile available".to_string()),
            Self::ProviderFailed { last_message, .. } => Some(last_message.clone()),
            Self::CompactionFailed { detail } => Some(detail.clone()),
            Self::Other { message } => Some(message.clone()),
        }
    }

    /// Short label used in logs and the `category` axis of metrics.
    pub fn category_label(&self) -> &'static str {
        match self {
            Self::UserStop => "user_stop",
            Self::RuntimeCancel => "runtime_cancel",
            Self::Shutdown => "shutdown",
            Self::Crash => "crash",
            Self::NoProfileAvailable => "no_profile",
            Self::ProviderFailed { .. } => "provider_failed",
            Self::CompactionFailed { .. } => "compaction_failed",
            Self::Other { .. } => "other",
        }
    }
}

impl StartupCause {
    /// Map the sentinel state observed at startup to the reason that
    /// stale turns should be finalized with.
    pub fn to_termination_reason(self) -> TerminationReason {
        match self {
            Self::Clean => TerminationReason::Shutdown,
            Self::Crash => TerminationReason::Crash,
        }
    }
}

// ── PartialMeta ───────────────────────────────────────────────────────

/// A tool_use block that may or may not have a matching tool_result.
/// Used by [`rebuild`] to emit the assistant-side block; the matching
/// (synthetic if needed) tool_result is emitted in a separate step.
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    pub call_id: String,
    pub name: String,
    /// JSON-encoded arguments string (matches what each provider
    /// stores in history — Chat / Responses / Codex keep arguments as
    /// string, Anthropic parses to value at emit time).
    pub arguments: String,
    /// `true` if this call already has a result in `executed_tools`.
    /// Used by [`rebuild::synthesize_tool_results`] to decide whether
    /// to emit a synthetic interrupted-marker result.
    pub has_result: bool,
}

/// A tool_use + tool_result pair that completed before the turn
/// terminated.
#[derive(Debug, Clone)]
pub struct ExecutedTool {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    /// The tool's stored result string (raw, not API-massaged).
    pub result: String,
    pub is_error: bool,
}

/// Everything finalize needs to rebuild the partial round into a
/// provider-native shape.
///
/// Built in two contexts:
/// - **Runtime convergence** (engine.rs failure / cancel branch):
///   collected from `StreamPersister` + `failed_attempt_partial` +
///   already-known inputs.
/// - **Startup sweep** (app_init.rs): reverse-rebuilt from `messages`
///   table via [`rebuild::collect_partial_from_messages`].
#[derive(Debug, Clone, Default)]
pub struct PartialMeta {
    /// Original user message text for this turn (so finalize can
    /// back-fill into `context_json` if it's missing — the early-user-
    /// persist landing in engine.rs makes this rarely needed at
    /// runtime, but the sweep path can't assume it ran).
    pub user_message: Option<String>,
    /// Which provider shape the partial blocks should be emitted in.
    /// `None` when no LLM call was attempted (e.g. NoProfileAvailable).
    pub provider_kind: Option<ProviderApiKind>,
    /// Accumulated assistant text, in stream order.
    pub text: Option<String>,
    /// Accumulated thinking / reasoning content.
    pub thinking: Option<String>,
    /// All tool_uses emitted in this round, in stream order.
    pub tool_calls: Vec<PendingToolCall>,
    /// Tool calls that completed with a result; same order as
    /// `tool_calls` (or a subset of it).
    pub executed_tools: Vec<ExecutedTool>,
    /// `_oc_round` to stamp on rebuilt items. `None` → use
    /// `recovered_round_id()` from `round_grouping`.
    pub round_id: Option<String>,
    /// The active turn id (so finalize can write `chat_turns`).
    pub turn_id: Option<String>,
    /// If a partial assistant row was already persisted (engine.rs
    /// `persist_failed_partial_assistant` path), link it.
    pub assistant_message_id: Option<i64>,
}

// ── FinalizeOutcome ───────────────────────────────────────────────────

/// What [`finalize_turn_context`] actually accomplished, for telemetry
/// and stream-end event payload. All fields are optional because
/// individual steps may have failed (finalize never panics — it
/// records best-effort).
#[derive(Debug, Clone, Default)]
pub struct FinalizeOutcome {
    pub event_row_id: Option<i64>,
    pub context_assistant_appended: bool,
    pub im_notice_dispatched: bool,
    pub turn_status: Option<ChatTurnStatus>,
    pub interrupt_reason: Option<ChatTurnInterruptReason>,
    /// `true` when this finalize call was a no-op because the turn was
    /// already finalized — caller can use this to avoid duplicate
    /// `chat:stream_end` emission.
    pub was_already_finalized: bool,
}

// ── Entry points ─────────────────────────────────────────────────────
//
// Both shapes share `apply_finalize` for everything except IM
// dispatch. The async version optionally fans out to the IM channel
// (when the caller knows a mirror is attached); the blocking version
// is for signal handlers and the startup sweep (cannot hold a tokio
// runtime open arbitrarily) and skips IM — the next launch will sync
// IM via the inbound message handler if needed.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::session::SessionDB;

/// Async finalize. The canonical entry point used by GUI/HTTP/IM/Cron
/// runtime convergence.
///
/// When `im_mirror` is `Some`, spawns a background task that drains
/// the live IM stream pipeline and (if a user-quote prefix was already
/// emitted into the IM chat) sends an IM-rendered notice for `reason`
/// so the IM thread doesn't show a dangling quote. The IM task runs
/// detached so slow remote API calls cannot gate the engine return.
pub(crate) async fn finalize_turn_context(
    db: &Arc<SessionDB>,
    session_id: &str,
    reason: TerminationReason,
    partial: PartialMeta,
    source: crate::chat_engine::ChatSource,
    im_mirror: Option<crate::chat_engine::im_mirror::ImLiveMirrorState>,
) -> FinalizeOutcome {
    let mut outcome = apply_finalize(db, session_id, &reason, &partial, source);
    if !outcome.was_already_finalized {
        // Stop / StopFailure hook. A user-initiated stop is a normal `Stop` with
        // status "interrupted" — fire_stop deliberately does NOT honor
        // block-to-continue on an interrupt (never resurrects a cancelled turn).
        // Every other termination reason (provider / compaction failure,
        // shutdown, crash, …) is a `StopFailure` carrying the failure category +
        // error text. Mutually exclusive with the success-path `Stop` fired in
        // the engine, so a turn fires exactly one.
        if reason.is_user_initiated() {
            crate::hooks::fire_stop(session_id, None, "interrupted", None);
        } else {
            crate::hooks::fire_stop_failure(
                session_id,
                reason.category_label(),
                reason.to_error_text().as_deref(),
            );
        }
        if let Some(state) = im_mirror {
            let body = copy::im_notice(&reason);
            tokio::spawn(async move {
                crate::chat_engine::im_mirror::abort_im_live_mirror_with_body(state, Some(body))
                    .await;
            });
            outcome.im_notice_dispatched = true;
        }
    }
    outcome
}

/// Synchronous finalize for signal handlers and the startup sweep.
/// Skips IM dispatch (the IM channel resyncs on next launch).
pub fn finalize_turn_context_blocking(
    db: &Arc<SessionDB>,
    session_id: &str,
    reason: TerminationReason,
    partial: PartialMeta,
    source: crate::chat_engine::ChatSource,
) -> FinalizeOutcome {
    apply_finalize(db, session_id, &reason, &partial, source)
}

fn apply_finalize(
    db: &Arc<SessionDB>,
    session_id: &str,
    reason: &TerminationReason,
    partial: &PartialMeta,
    source: crate::chat_engine::ChatSource,
) -> FinalizeOutcome {
    // 1. Re-entry guard. `mark_finalized(None)` always returns true so
    //    sweep paths without a turn_id are not gated here — they
    //    handle idempotency via the DB UPDATE `WHERE status NOT IN
    //    terminal` clause inside `finish_chat_turn_once`.
    if !crate::chat_engine::active_turn::mark_finalized(partial.turn_id.as_deref()) {
        let mut outcome = FinalizeOutcome {
            was_already_finalized: true,
            ..Default::default()
        };
        if let Some(turn_id) = partial.turn_id.as_deref() {
            if let Ok(Some(turn)) = db.get_chat_turn(turn_id) {
                if turn.status.is_terminal() {
                    outcome.turn_status = Some(turn.status);
                    outcome.interrupt_reason = turn.interrupt_reason;
                } else {
                    outcome.turn_status = Some(reason.to_chat_turn_status());
                    outcome.interrupt_reason = Some(reason.to_chat_turn_interrupt_reason());
                }
            }
        }
        return outcome;
    }

    let mut outcome = FinalizeOutcome::default();

    if let Err(e) = rebuild_and_save_context(db, session_id, reason, partial) {
        app_warn!(
            "chat_engine",
            "finalize",
            "context_json save failed for session {} reason {}: {}",
            session_id,
            reason.category_label(),
            e
        );
    } else {
        outcome.context_assistant_appended = true;
    }

    match write_user_event_row(db, session_id, reason, source) {
        Ok(id) => outcome.event_row_id = Some(id),
        Err(e) => app_warn!(
            "chat_engine",
            "finalize",
            "event row append failed for session {} reason {}: {}",
            session_id,
            reason.category_label(),
            e
        ),
    }
    if outcome.event_row_id.is_some() {
        match db.mark_current_turn_orphaned_rows_recovered(session_id) {
            Ok(0) => {}
            Ok(n) => app_info!(
                "chat_engine",
                "finalize",
                "marked {} orphaned stream row(s) recovered for session {}",
                n,
                session_id
            ),
            Err(e) => app_warn!(
                "chat_engine",
                "finalize",
                "failed to mark orphaned stream rows recovered for session {}: {}",
                session_id,
                e
            ),
        }
    }

    let turn_status = reason.to_chat_turn_status();
    let interrupt = reason.to_chat_turn_interrupt_reason();
    if let Some(turn_id) = partial.turn_id.as_deref() {
        match db.finish_chat_turn_once(
            turn_id,
            turn_status,
            Some(interrupt),
            reason.to_error_text().as_deref(),
            partial.assistant_message_id,
        ) {
            Ok(_) => {
                outcome.turn_status = Some(turn_status);
                outcome.interrupt_reason = Some(interrupt);
            }
            Err(e) => app_warn!(
                "chat_engine",
                "finalize",
                "finish_chat_turn_once failed for turn {}: {}",
                turn_id,
                e
            ),
        }
    } else {
        outcome.turn_status = Some(turn_status);
        outcome.interrupt_reason = Some(interrupt);
    }

    app_info!(
        "chat_engine",
        "finalize",
        "session={} reason={} status={} event_row={:?} context_appended={}",
        session_id,
        reason.category_label(),
        turn_status.as_str(),
        outcome.event_row_id,
        outcome.context_assistant_appended,
    );

    outcome
}

fn rebuild_and_save_context(
    db: &Arc<SessionDB>,
    session_id: &str,
    reason: &TerminationReason,
    partial: &PartialMeta,
) -> anyhow::Result<()> {
    // Mirror the synthetic tool_results we're about to splice into
    // `context_json` into the `messages` table so the GUI doesn't
    // perpetually render those tool rows as `running`. Independent of
    // context_json so a save failure later doesn't leave the UI stuck.
    mark_pending_tool_rows_as_interrupted(db, session_id, partial);

    let (stored_context, context_revision) = db.load_context_with_revision(session_id)?;
    let mut history: Vec<Value> = stored_context
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let round_id = partial
        .round_id
        .clone()
        .unwrap_or_else(crate::context_compact::recovered_round_id);

    // Step 3: back-fill user message if missing.
    if let Some(user_msg) = partial.user_message.as_deref() {
        let user_msg = user_msg.trim();
        if !user_msg.is_empty() && !history_already_has_user(&history, user_msg) {
            push_stamped(
                &mut history,
                json!({"role": "user", "content": user_msg}),
                &round_id,
            );
        }
    }

    for block in rebuild::rebuild_partial_assistant_blocks(partial) {
        push_stamped(&mut history, block, &round_id);
    }

    // tool_results must immediately follow tool_use per Anthropic contract.
    for block in rebuild::synthesize_tool_results(partial, rebuild::INTERRUPTED_TOOL_RESULT) {
        push_stamped(&mut history, block, &round_id);
    }

    let marker = copy::model_marker(reason);
    push_stamped(
        &mut history,
        json!({"role": "assistant", "content": marker}),
        &round_id,
    );

    let json_str = serde_json::to_string(&history)?;
    db.save_context_at_revision(session_id, context_revision, &json_str, None)?;
    Ok(())
}

fn write_user_event_row(
    db: &Arc<SessionDB>,
    session_id: &str,
    reason: &TerminationReason,
    source: crate::chat_engine::ChatSource,
) -> anyhow::Result<i64> {
    let body = copy::user_notice(reason);
    let msg = if reason.is_user_initiated() {
        crate::session::NewMessage::event(&body)
    } else {
        crate::session::NewMessage::error_event(&body)
    };
    let msg = msg.with_source(source);
    db.append_message(session_id, &msg)
}

fn history_already_has_user(history: &[Value], user_msg: &str) -> bool {
    // The early-user-persist path already pushed the user during the
    // current turn; without it (sweep on a turn that crashed before
    // the persist call) we add. Cheap last-N scan is enough.
    history
        .iter()
        .rev()
        .take(4)
        .any(|item| value_role_is(item, "user") && value_text_contains(item, user_msg))
}

fn value_role_is(item: &Value, role: &str) -> bool {
    item.get("role").and_then(|r| r.as_str()) == Some(role)
}

fn value_text_contains(item: &Value, needle: &str) -> bool {
    match item.get("content") {
        Some(Value::String(s)) => s.contains(needle),
        Some(Value::Array(arr)) => arr.iter().any(|b| {
            b.get("text")
                .and_then(|v| v.as_str())
                .map(|t| t.contains(needle))
                .unwrap_or(false)
                || b.get("content")
                    .and_then(|v| v.as_str())
                    .map(|t| t.contains(needle))
                    .unwrap_or(false)
        }),
        _ => false,
    }
}

fn push_stamped(history: &mut Vec<Value>, mut value: Value, round_id: &str) {
    crate::context_compact::stamp_round(&mut value, round_id);
    history.push(value);
}

/// Update each `tool` row whose `tool_use` had no real result yet so
/// the GUI stops rendering it as in-flight. Mirrors what the synthetic
/// `tool_result` blocks added to `context_json` say to the model:
/// the tool's body is the same `INTERRUPTED_TOOL_RESULT` constant and
/// `is_error=true` flips the GUI's tool card to the failure state.
///
/// Skips entries that already have a result — those are real completed
/// tools whose response we'd otherwise clobber. Best-effort: a DB
/// failure logs and continues; the context_json marker is the
/// authoritative model-facing signal.
fn mark_pending_tool_rows_as_interrupted(
    db: &Arc<SessionDB>,
    session_id: &str,
    partial: &PartialMeta,
) {
    for tc in &partial.tool_calls {
        if tc.has_result {
            continue;
        }
        if let Err(e) = db.update_tool_result_with_metadata(
            session_id,
            &tc.call_id,
            rebuild::INTERRUPTED_TOOL_RESULT,
            Some(0),
            true,
            None,
        ) {
            app_warn!(
                "chat_engine",
                "finalize",
                "failed to mark tool row interrupted (session={} call_id={}): {}",
                session_id,
                tc.call_id,
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_engine::active_turn::{reset_finalized_for_test, test_lock};
    use crate::chat_engine::ChatSource;
    use crate::session::SessionDB;

    fn temp_db() -> Arc<SessionDB> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        std::mem::forget(dir);
        Arc::new(SessionDB::open_ephemeral_for_test(&path).unwrap())
    }

    fn fresh_session(db: &Arc<SessionDB>) -> (String, String) {
        // Tests in this module share the `FINALIZED_TURNS` global; the
        // `test_lock` guard held by each test serializes access, and
        // `reset_finalized_for_test` ensures no leftover entries from
        // earlier suites.
        reset_finalized_for_test();
        let session = db
            .create_session_with_project("ha-main", None, None)
            .unwrap();
        let _ = db.append_message(&session.id, &crate::session::NewMessage::user("hello"));
        let turn = db
            .create_chat_turn(&session.id, "desktop", Some("s-1"), Some(1))
            .unwrap();
        (session.id, turn.id)
    }

    #[test]
    fn user_stop_writes_event_row_and_finishes_turn_as_interrupted() {
        let _lock = test_lock();
        let db = temp_db();
        let (sid, turn_id) = fresh_session(&db);

        let outcome = finalize_turn_context_blocking(
            &db,
            &sid,
            TerminationReason::UserStop,
            PartialMeta {
                user_message: Some("hello".into()),
                provider_kind: Some(ProviderApiKind::Anthropic),
                text: Some("partial".into()),
                turn_id: Some(turn_id.clone()),
                ..Default::default()
            },
            ChatSource::Desktop,
        );

        assert!(outcome.event_row_id.is_some());
        assert!(outcome.context_assistant_appended);
        assert_eq!(outcome.turn_status, Some(ChatTurnStatus::Interrupted));
        assert_eq!(
            outcome.interrupt_reason,
            Some(ChatTurnInterruptReason::UserStop)
        );

        // chat_turn row reflects Interrupted/UserStop.
        let persisted = db.get_chat_turn(&turn_id).unwrap().unwrap();
        assert_eq!(persisted.status, ChatTurnStatus::Interrupted);
        assert_eq!(
            persisted.interrupt_reason,
            Some(ChatTurnInterruptReason::UserStop)
        );
        assert!(persisted.error.is_none());

        // context_json contains the marker + partial text as separate items.
        let ctx_json = db.load_context(&sid).unwrap().unwrap();
        let history: Vec<Value> = serde_json::from_str(&ctx_json).unwrap();
        let marker = history
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .and_then(|m| m.get("content").and_then(|c| c.as_str()))
            .unwrap_or("");
        assert!(marker.contains("用户主动停止"), "marker missing: {marker}");
    }

    #[test]
    fn provider_failed_synthesizes_tool_result_anthropic() {
        let _lock = test_lock();
        let db = temp_db();
        let (sid, turn_id) = fresh_session(&db);

        let partial = PartialMeta {
            user_message: Some("hello".into()),
            provider_kind: Some(ProviderApiKind::Anthropic),
            text: None,
            thinking: None,
            tool_calls: vec![PendingToolCall {
                call_id: "c1".into(),
                name: "exec".into(),
                arguments: r#"{"cmd":"ls"}"#.into(),
                has_result: false,
            }],
            turn_id: Some(turn_id),
            ..Default::default()
        };
        let _ = finalize_turn_context_blocking(
            &db,
            &sid,
            TerminationReason::ProviderFailed {
                last_kind: crate::failover::FailoverReason::Auth,
                last_message: "401".into(),
                is_codex_auth: false,
            },
            partial,
            ChatSource::Desktop,
        );

        let ctx_json = db.load_context(&sid).unwrap().unwrap();
        let history: Vec<Value> = serde_json::from_str(&ctx_json).unwrap();
        // Anthropic: assistant message with tool_use, then user message
        // with tool_result block.
        let has_tool_use = history.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|b| b.get("type") == Some(&json!("tool_use")))
                })
                .unwrap_or(false)
        });
        let has_tool_result = history.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter().any(|b| {
                        b.get("type") == Some(&json!("tool_result"))
                            && b.get("tool_use_id") == Some(&json!("c1"))
                    })
                })
                .unwrap_or(false)
        });
        assert!(has_tool_use, "no tool_use synthesized");
        assert!(
            has_tool_result,
            "no tool_result synthesized — Anthropic would 400"
        );
    }

    // Invariant: a failed turn must leave the user message persisted in
    // `context_json`. Without this, the next `restore_agent_context` call
    // hands the model an empty history and the LLM has no clue what the
    // user just asked — the classic "retry lost my prompt" symptom.
    #[test]
    fn provider_failed_keeps_user_message_in_context() {
        let _lock = test_lock();
        let db = temp_db();
        let (sid, turn_id) = fresh_session(&db);

        let partial = PartialMeta {
            user_message: Some("read kefu source and compare with visitor-next".into()),
            provider_kind: Some(ProviderApiKind::OpenAIResponses),
            turn_id: Some(turn_id),
            ..Default::default()
        };
        let outcome = finalize_turn_context_blocking(
            &db,
            &sid,
            TerminationReason::ProviderFailed {
                last_kind: crate::failover::FailoverReason::Unknown,
                last_message: "rs_xxx not found".into(),
                is_codex_auth: false,
            },
            partial,
            ChatSource::Desktop,
        );
        assert!(outcome.context_assistant_appended);

        let ctx_json = db.load_context(&sid).unwrap().unwrap();
        let history: Vec<Value> = serde_json::from_str(&ctx_json).unwrap();
        let user_text_present = history.iter().any(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("user")
                && m.get("content")
                    .and_then(|c| c.as_str())
                    .map(|s| s.contains("read kefu"))
                    .unwrap_or(false)
        });
        assert!(
            user_text_present,
            "context_json must keep the user message after a failed turn; got: {ctx_json}"
        );
    }

    #[test]
    fn reentry_is_noop() {
        let _lock = test_lock();
        let db = temp_db();
        let (sid, turn_id) = fresh_session(&db);

        let make_partial = || PartialMeta {
            user_message: Some("hello".into()),
            provider_kind: Some(ProviderApiKind::Anthropic),
            text: Some("partial".into()),
            turn_id: Some(turn_id.clone()),
            ..Default::default()
        };

        let first = finalize_turn_context_blocking(
            &db,
            &sid,
            TerminationReason::UserStop,
            make_partial(),
            ChatSource::Desktop,
        );
        assert!(first.event_row_id.is_some());

        let second = finalize_turn_context_blocking(
            &db,
            &sid,
            TerminationReason::UserStop,
            make_partial(),
            ChatSource::Desktop,
        );
        assert!(second.was_already_finalized);
        assert!(second.event_row_id.is_none());
    }

    #[test]
    fn reentry_returns_terminal_status_even_if_db_is_still_cancelling() {
        let _lock = test_lock();
        let db = temp_db();
        let (sid, turn_id) = fresh_session(&db);
        assert!(crate::chat_engine::active_turn::mark_finalized(Some(
            &turn_id
        )));
        db.mark_chat_turn_cancelling(&turn_id, ChatTurnInterruptReason::UserStop)
            .unwrap();

        let outcome = finalize_turn_context_blocking(
            &db,
            &sid,
            TerminationReason::UserStop,
            PartialMeta {
                user_message: Some("hello".into()),
                turn_id: Some(turn_id),
                ..Default::default()
            },
            ChatSource::Desktop,
        );

        assert!(outcome.was_already_finalized);
        assert_eq!(outcome.turn_status, Some(ChatTurnStatus::Interrupted));
        assert_eq!(
            outcome.interrupt_reason,
            Some(ChatTurnInterruptReason::UserStop)
        );
    }

    #[test]
    fn user_stop_maps_to_interrupted_status() {
        let r = TerminationReason::UserStop;
        assert_eq!(r.to_chat_turn_status(), ChatTurnStatus::Interrupted);
        assert_eq!(
            r.to_chat_turn_interrupt_reason(),
            ChatTurnInterruptReason::UserStop
        );
        assert!(r.is_user_initiated());
        assert_eq!(r.to_error_text(), None);
    }

    #[test]
    fn provider_failed_maps_to_failed_status() {
        let r = TerminationReason::ProviderFailed {
            last_kind: FailoverReason::Auth,
            last_message: "401 Unauthorized".into(),
            is_codex_auth: false,
        };
        assert_eq!(r.to_chat_turn_status(), ChatTurnStatus::Failed);
        assert_eq!(
            r.to_chat_turn_interrupt_reason(),
            ChatTurnInterruptReason::ProviderFailed
        );
        assert!(!r.is_user_initiated());
        assert_eq!(r.to_error_text().as_deref(), Some("401 Unauthorized"));
    }

    #[test]
    fn startup_cause_maps_to_termination_reason() {
        assert!(matches!(
            StartupCause::Clean.to_termination_reason(),
            TerminationReason::Shutdown
        ));
        assert!(matches!(
            StartupCause::Crash.to_termination_reason(),
            TerminationReason::Crash
        ));
    }
}
