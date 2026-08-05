use futures_util::FutureExt;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

use super::dispatcher::deliver_media_to_chat;
use ha_core::channel::traits::{ChannelPlugin, ChannelReplyStream};
use ha_core::channel::types::*;
use ha_core::chat_engine::RoundTextAccumulator;

/// Cardkit single-element character ceiling, per Feishu docs (100,000
/// characters per markdown element). Counted in `chars()` not bytes —
/// CJK glyphs are 3 bytes UTF-8, so a byte-based limit would silently
/// truncate at ~33k Chinese characters. Independent of IM-text
/// `streaming_preview_max_bytes` (cardkit elements aren't subject to
/// that gate) so streaming previews keep flowing on responses larger
/// than the channel's text-message cap.
pub(super) const CARD_ELEMENT_MAX_CHARS: usize = 100_000;
pub(super) const STREAM_PREVIEW_FIRST_FLUSH_DELAY: Duration = Duration::from_millis(300);
pub(super) const STREAM_PREVIEW_FLUSH_INTERVAL: Duration = Duration::from_millis(1000);
/// Keep native cleanup shorter than the callers' three-second cancellation
/// window. Timing out only stops the caller from waiting: the spawned provider
/// abort remains detached and continues owning the consumed stream.
const NATIVE_ABORT_WAIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(super) struct StreamPreviewFlushSchedule {
    next_at: Instant,
}

impl StreamPreviewFlushSchedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            next_at: now + STREAM_PREVIEW_FIRST_FLUSH_DELAY,
        }
    }

    pub(super) fn next_at(&self) -> Instant {
        self.next_at
    }

    pub(super) fn should_flush(&self, dirty: bool, has_text: bool, now: Instant) -> bool {
        dirty && has_text && now >= self.next_at
    }

    pub(super) fn mark_flushed(&mut self, now: Instant) {
        self.next_at = now + STREAM_PREVIEW_FLUSH_INTERVAL;
    }
}

pub(crate) type SharedNativeReplySession =
    Arc<tokio::sync::Mutex<Option<Box<dyn ChannelReplyStream>>>>;

pub(super) const NATIVE_SELECTED: u8 = 0;
pub(super) const NATIVE_OPENING: u8 = 1;
pub(super) const NATIVE_ACTIVE: u8 = 2;
pub(super) const NATIVE_BROKEN: u8 = 3;
pub(super) const NATIVE_AMBIGUOUS: u8 = 4;
pub(super) const NATIVE_TERMINAL: u8 = 5;
pub(super) const NATIVE_ABORTING: u8 = 6;
const NATIVE_TERMINAL_UNCLAIMED: u8 = 0;
const NATIVE_TERMINAL_FINAL: u8 = 1;
const NATIVE_TERMINAL_ABORT: u8 = 2;

#[derive(Clone)]
pub(super) enum StreamPreviewTransport {
    /// Canonical provider-owned streaming lifecycle. This is selected before
    /// all legacy transports when the fully resolved target is eligible.
    Native {
        target: ReplyStreamTarget,
        capabilities: NativeReplyCapabilities,
        legacy_capabilities: ChannelCapabilities,
        session: SharedNativeReplySession,
        state: Arc<AtomicU8>,
        /// Shared with the owning pipeline so a task failure after a safe
        /// native-open rejection cannot resurrect the stale native handle,
        /// whether the replacement is legacy preview or no preview.
        native_preview_relinquished: Arc<AtomicBool>,
        /// Exactly one durable terminal owner is allowed: final delivery or
        /// explicit abort/error. This is separate from preview lifecycle state
        /// because both paths can legitimately leave that state `Terminal`.
        terminal_owner: Arc<AtomicU8>,
    },
    /// Telegram-style draft API: `send_draft` repeatedly with the same
    /// `draft_id`. Free of edit-rate limits, leaves no "edited" marker.
    Draft,
    /// Standard `send_message` + `edit_message` cycle. Works on most
    /// channels but typically flags the host message as edited.
    Message,
    /// Card-streaming API (currently Feishu cardkit). Creates an
    /// interactive card and updates a single element in place — the host
    /// message is never edited, so no "edited" marker appears.
    Card,
    /// Native preview was safely rejected and this adapter exposes no legacy
    /// preview transport. Keep accumulating; final delivery uses the ordinary
    /// standalone lane.
    Disabled,
}

impl std::fmt::Debug for StreamPreviewTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native { state, .. } => f
                .debug_struct("Native")
                .field("state", &state.load(Ordering::Acquire))
                .finish_non_exhaustive(),
            Self::Draft => f.write_str("Draft"),
            Self::Message => f.write_str("Message"),
            Self::Card => f.write_str("Card"),
            Self::Disabled => f.write_str("Disabled"),
        }
    }
}

impl PartialEq for StreamPreviewTransport {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Draft, Self::Draft)
                | (Self::Message, Self::Message)
                | (Self::Card, Self::Card)
                | (Self::Disabled, Self::Disabled)
                | (Self::Native { .. }, Self::Native { .. })
        )
    }
}

impl Eq for StreamPreviewTransport {}

impl StreamPreviewTransport {
    pub(super) fn native_preview_handle(&self) -> Option<PreviewHandle> {
        match self {
            Self::Native {
                session,
                state,
                terminal_owner,
                capabilities,
                ..
            } => Some(PreviewHandle::Native {
                session: session.clone(),
                state: state.clone(),
                terminal_owner: terminal_owner.clone(),
                preview_persistence: capabilities.preview_persistence,
            }),
            _ => None,
        }
    }

    pub(super) fn native_preview_relinquished_flag(&self) -> Option<Arc<AtomicBool>> {
        match self {
            Self::Native {
                native_preview_relinquished,
                ..
            } => Some(native_preview_relinquished.clone()),
            _ => None,
        }
    }
}

/// Persistent identity for the rendered preview, returned to the caller so
/// `send_final_reply` can finalize using the matching path.
///
/// Visibility is `pub(crate)` so reused-by-attach-sync helpers in the
/// dispatcher can take an `Option<&PreviewHandle>` parameter without
/// dragging the worker's internal types into the public API surface.
#[derive(Clone)]
pub(crate) enum PreviewHandle {
    Native {
        session: SharedNativeReplySession,
        state: Arc<AtomicU8>,
        terminal_owner: Arc<AtomicU8>,
        preview_persistence: ReplyStreamPreviewPersistence,
    },
    /// `edit_message` rewrites this message_id at finalization.
    Message { message_id: String },
    /// Card-stream session. `broken=true` means a visible update became
    /// ambiguous; finalization must fail closed rather than open a fresh
    /// message that could duplicate accepted content.
    Card {
        card_id: String,
        element_id: String,
        sequence: i64,
        broken: bool,
    },
}

impl std::fmt::Debug for PreviewHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native {
                state,
                terminal_owner,
                ..
            } => f
                .debug_struct("Native")
                .field("state", &state.load(Ordering::Acquire))
                .field("terminal_owner", &terminal_owner.load(Ordering::Acquire))
                .finish_non_exhaustive(),
            Self::Message { message_id } => f
                .debug_struct("Message")
                .field("message_id", message_id)
                .finish(),
            Self::Card {
                card_id,
                element_id,
                sequence,
                broken,
            } => f
                .debug_struct("Card")
                .field("card_id", card_id)
                .field("element_id", element_id)
                .field("sequence", sequence)
                .field("broken", broken)
                .finish(),
        }
    }
}

/// Claim the terminal mutation for final delivery. A failed claim means a
/// final or abort path already owns the turn and the caller must send nothing.
pub(super) fn try_claim_native_final(terminal_owner: &AtomicU8) -> bool {
    terminal_owner
        .compare_exchange(
            NATIVE_TERMINAL_UNCLAIMED,
            NATIVE_TERMINAL_FINAL,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

/// Claim explicit abort/error ownership. Re-entry is deliberately rejected:
/// cleanup may be idempotent, but the caller can also emit a durable plain
/// error after this returns and that notification must remain exactly once.
fn claim_native_abort(terminal_owner: &AtomicU8) -> bool {
    terminal_owner
        .compare_exchange(
            NATIVE_TERMINAL_UNCLAIMED,
            NATIVE_TERMINAL_ABORT,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

enum NativeTerminalAction {
    Abort(ReplyAbortReason),
    Error(String),
}

impl NativeTerminalAction {
    fn allows_ephemeral_expiry(&self) -> bool {
        matches!(self, Self::Abort(_))
    }
}

fn native_terminal_is_safe(
    action: &NativeTerminalAction,
    preview_persistence: ReplyStreamPreviewPersistence,
    provider_acknowledged: bool,
) -> bool {
    // This answers whether the provider-owned terminal settled (or a plain
    // abort only touched an ephemeral preview). A Persistent acknowledgement
    // does not prove its already-appended partial is invisible, so replay
    // callers must additionally check `preview_persistence`.
    provider_acknowledged
        || (action.allows_ephemeral_expiry()
            && matches!(
                preview_persistence,
                ReplyStreamPreviewPersistence::Ephemeral
            ))
}

fn spawn_native_terminal(
    stream: Box<dyn ChannelReplyStream>,
    action: NativeTerminalAction,
    state: Arc<AtomicU8>,
    success_state: u8,
) -> tokio::task::JoinHandle<bool> {
    state.store(NATIVE_ABORTING, Ordering::Release);
    tokio::spawn(async move {
        let terminal = match action {
            NativeTerminalAction::Abort(reason) => {
                std::panic::AssertUnwindSafe(stream.abort(reason))
                    .catch_unwind()
                    .await
            }
            NativeTerminalAction::Error(error_text) => {
                std::panic::AssertUnwindSafe(stream.fail(&error_text))
                    .catch_unwind()
                    .await
            }
        };
        match terminal {
            Ok(Ok(())) => {
                state.store(success_state, Ordering::Release);
                true
            }
            Ok(Err(error)) => {
                state.store(NATIVE_AMBIGUOUS, Ordering::Release);
                app_warn!(
                    "channel",
                    "worker",
                    "Native reply terminal mutation failed: {}",
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
                false
            }
            Err(_) => {
                state.store(NATIVE_AMBIGUOUS, Ordering::Release);
                // Panic payloads can contain arbitrary adapter data; never
                // format them into logs or the public delivery error.
                app_warn!(
                    "channel",
                    "worker",
                    "Native reply terminal adapter panicked; outcome is ambiguous"
                );
                false
            }
        }
    })
}

fn spawn_native_abort(
    stream: Box<dyn ChannelReplyStream>,
    reason: ReplyAbortReason,
    state: Arc<AtomicU8>,
    success_state: u8,
) -> tokio::task::JoinHandle<bool> {
    spawn_native_terminal(
        stream,
        NativeTerminalAction::Abort(reason),
        state,
        success_state,
    )
}

async fn await_native_abort_result(state: &AtomicU8, wait_timeout: Duration) -> bool {
    let result = tokio::time::timeout(wait_timeout, async {
        loop {
            match state.load(Ordering::Acquire) {
                NATIVE_TERMINAL => return true,
                NATIVE_AMBIGUOUS | NATIVE_BROKEN => return false,
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await;
    match result {
        Ok(safely_aborted) => safely_aborted,
        Err(_) => {
            app_warn!(
                "channel",
                "worker",
                "Timed out waiting {:?} for an in-flight native reply abort; cleanup remains detached",
                wait_timeout
            );
            false
        }
    }
}

async fn await_native_abort_task(
    abort_task: tokio::task::JoinHandle<bool>,
    state: &AtomicU8,
    wait_timeout: Duration,
) -> bool {
    match tokio::time::timeout(wait_timeout, abort_task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            state.store(NATIVE_AMBIGUOUS, Ordering::Release);
            app_warn!(
                "channel",
                "worker",
                "Native reply abort task failed ambiguously: {}",
                error
            );
            false
        }
        Err(_) => {
            // Dropping a Tokio JoinHandle detaches rather than cancels its task.
            // Keep NATIVE_ABORTING so another caller cannot mistake timeout for
            // provider acknowledgement while the consumed stream finishes.
            app_warn!(
                "channel",
                "worker",
                "Timed out waiting {:?} for native reply abort; cleanup remains detached",
                wait_timeout
            );
            false
        }
    }
}

pub(crate) async fn abort_native_preview(
    preview: &PreviewHandle,
    reason: ReplyAbortReason,
) -> bool {
    abort_native_preview_with_timeout(preview, reason, NATIVE_ABORT_WAIT_TIMEOUT).await
}

pub(super) async fn abort_native_preview_with_timeout(
    preview: &PreviewHandle,
    reason: ReplyAbortReason,
    wait_timeout: Duration,
) -> bool {
    finish_native_preview_with_timeout(preview, NativeTerminalAction::Abort(reason), wait_timeout)
        .await
}

/// Terminate a native preview with a visible error through the same consumed
/// stream handle. Unlike a plain abort, an ephemeral preview expiring is not
/// enough: the adapter must acknowledge the persistent error terminal.
pub(crate) async fn fail_native_preview(preview: &PreviewHandle, error_text: &str) -> bool {
    finish_native_preview_with_timeout(
        preview,
        NativeTerminalAction::Error(error_text.to_string()),
        NATIVE_ABORT_WAIT_TIMEOUT,
    )
    .await
}

/// Claim a native preview that provably never crossed the provider boundary.
/// Winning `Selected -> Terminal` excludes a concurrent open; only that winner
/// may use the standalone error lane. A failed terminal-owner claim is unsafe
/// and must not send anything.
pub(crate) fn claim_unopened_native_error(preview: &PreviewHandle) -> bool {
    let PreviewHandle::Native {
        state,
        terminal_owner,
        ..
    } = preview
    else {
        return false;
    };
    state
        .compare_exchange(
            NATIVE_SELECTED,
            NATIVE_TERMINAL,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
        && claim_native_abort(terminal_owner)
}

async fn finish_native_preview_with_timeout(
    preview: &PreviewHandle,
    action: NativeTerminalAction,
    wait_timeout: Duration,
) -> bool {
    let PreviewHandle::Native {
        session,
        state,
        terminal_owner,
        preview_persistence,
    } = preview
    else {
        return true;
    };
    if !claim_native_abort(terminal_owner) {
        return false;
    }
    loop {
        let current = state.load(Ordering::Acquire);
        match current {
            NATIVE_ABORTING => {
                return native_terminal_is_safe(
                    &action,
                    *preview_persistence,
                    await_native_abort_result(state, wait_timeout).await,
                );
            }
            NATIVE_BROKEN | NATIVE_AMBIGUOUS
                if matches!(
                    preview_persistence,
                    ReplyStreamPreviewPersistence::Ephemeral
                ) =>
            {
                if state
                    .compare_exchange(
                        current,
                        NATIVE_TERMINAL,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    break;
                }
            }
            NATIVE_BROKEN | NATIVE_AMBIGUOUS => return false,
            NATIVE_TERMINAL => return action.allows_ephemeral_expiry(),
            NATIVE_SELECTED | NATIVE_OPENING | NATIVE_ACTIVE => {
                if state
                    .compare_exchange(
                        current,
                        NATIVE_TERMINAL,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    break;
                }
            }
            _ => return false,
        }
    }

    let stream = match tokio::time::timeout(wait_timeout, session.lock()).await {
        Ok(mut guard) => guard.take(),
        Err(_) => {
            app_warn!(
                "channel",
                "worker",
                "Timed out waiting {:?} to acquire native reply session for terminal mutation; provider mutation remains detached",
                wait_timeout
            );
            return native_terminal_is_safe(&action, *preview_persistence, false);
        }
    };
    if let Some(stream) = stream {
        let allows_ephemeral_expiry = action.allows_ephemeral_expiry();
        let terminal_task = spawn_native_terminal(stream, action, state.clone(), NATIVE_TERMINAL);
        let provider_acknowledged =
            await_native_abort_task(terminal_task, state, wait_timeout).await;
        provider_acknowledged
            || (allows_ephemeral_expiry
                && matches!(
                    preview_persistence,
                    ReplyStreamPreviewPersistence::Ephemeral
                ))
    } else {
        let cleanup_acknowledged = match state.load(Ordering::Acquire) {
            NATIVE_ABORTING => await_native_abort_result(state, wait_timeout).await,
            NATIVE_BROKEN | NATIVE_AMBIGUOUS => false,
            NATIVE_TERMINAL => action.allows_ephemeral_expiry(),
            _ => false,
        };
        native_terminal_is_safe(&action, *preview_persistence, cleanup_acknowledged)
    }
}

#[derive(Debug, Default)]
pub(super) struct StreamPreviewOutcome {
    pub preview: Option<PreviewHandle>,
    /// Number of LLM rounds the stream task already finalized inline (only
    /// non-zero under `ImReplyMode::Split` on streaming-capable channels).
    /// The dispatcher must skip these in `deliver_split` to avoid sending
    /// duplicate text or media; the caller's `drained_rounds[finalized_rounds..]`
    /// slice is what's left for it to deliver.
    pub finalized_rounds: usize,
    /// Final-content delivery results for rounds completed inside the stream
    /// task. Preview refreshes are excluded; only rounds the dispatcher will
    /// skip as already delivered contribute here.
    pub delivery_report: super::dispatcher::DeliveryReport,
}

pub(super) fn append_preview_round_text(accumulated: &mut String, text: &str, new_round: bool) {
    if text.is_empty() {
        return;
    }
    if new_round
        && !accumulated.is_empty()
        && !accumulated.ends_with('\n')
        && !text.starts_with('\n')
    {
        accumulated.push('\n');
    }
    accumulated.push_str(text);
}

#[derive(Default)]
pub(super) struct NativeFrameState {
    pub(super) acknowledged_bytes: usize,
    pub(super) revision: u64,
    pub(super) phase: Option<ReplyStreamPhase>,
    pub(super) tasks: SafeTaskTracker,
    pub(super) last_acknowledged_at: Option<Instant>,
}

#[derive(Default)]
pub(super) struct SafeTaskTracker {
    pub(super) tasks: Vec<ReplyStreamTask>,
}

#[derive(serde::Deserialize)]
struct SafeToolEvent {
    #[serde(rename = "type")]
    kind: String,
    call_id: Option<String>,
    name: Option<String>,
    is_error: Option<bool>,
    duration_ms: Option<u64>,
}

impl SafeTaskTracker {
    pub(super) fn observe(&mut self, event_json: &str) -> bool {
        let Ok(event) = serde_json::from_str::<SafeToolEvent>(event_json) else {
            return false;
        };
        let Some(call_id) = event.call_id.filter(|id| !id.is_empty()) else {
            return false;
        };
        let id = format!(
            "tool-{}",
            &blake3::hash(call_id.as_bytes()).to_hex().as_str()[..16]
        );
        match event.kind.as_str() {
            "tool_call" => {
                if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                    task.status = ReplyStreamTaskStatus::InProgress;
                    task.details = Some("工具正在运行".to_string());
                    return true;
                }
                if self.tasks.len() >= 64 {
                    return false;
                }
                let raw_name = event.name.as_deref().unwrap_or("工具").trim();
                let title = if raw_name.is_empty() {
                    "工具".to_string()
                } else {
                    ha_core::truncate_utf8(raw_name, 128).to_string()
                };
                self.tasks.push(ReplyStreamTask {
                    id,
                    title,
                    status: ReplyStreamTaskStatus::InProgress,
                    details: Some("工具正在运行".to_string()),
                });
                true
            }
            "tool_result" => {
                let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
                    return false;
                };
                let failed = event.is_error.unwrap_or(false);
                task.status = if failed {
                    ReplyStreamTaskStatus::Error
                } else {
                    ReplyStreamTaskStatus::Complete
                };
                task.details = Some(match (failed, event.duration_ms) {
                    (true, Some(ms)) => format!("工具执行失败（耗时 {} 毫秒）", ms),
                    (true, None) => "工具执行失败".to_string(),
                    (false, Some(ms)) => format!("工具已完成（耗时 {} 毫秒）", ms),
                    (false, None) => "工具已完成".to_string(),
                });
                true
            }
            _ => false,
        }
    }

    fn snapshot(&self, supported: bool) -> Vec<ReplyStreamTask> {
        if supported {
            self.tasks.clone()
        } else {
            Vec::new()
        }
    }
}

fn truncate_chars(value: &str, max: Option<u32>) -> String {
    let Some(max) = max.map(|value| value as usize) else {
        return value.to_string();
    };
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect()
}

/// Spawn a background task that receives streaming events from the chat engine
/// and sends progressive previews to the IM channel.
///
/// Two distinct preview behaviors driven by `reply_mode`:
///
/// - **`Preview` mode**: legacy single-growing-message behavior. Text deltas
///   from every round accumulate into one buffer that the preview transport
///   keeps re-rendering. Caller commits via `send_final_reply` using the
///   `PreviewHandle` returned in `StreamPreviewOutcome`.
///
/// - **`Split` mode + streaming-capable channel**: per-round preview. Each
///   round gets its own preview message that streams typewriter-style; on
///   round boundary (next round's first `text_delta` after a `tool_call`)
///   the task finalizes the current preview, delivers that round's media,
///   and resets state for the next round. The final round's preview is left
///   open so the caller can finalize it via `send_final_reply` (matching
///   the canonical chunk-or-card path). `finalized_rounds` reports how many
///   rounds the task already shipped, so the dispatcher only delivers the
///   trailing round.
///
/// - **`Final` / `Split` mode + non-streaming channel**: events are drained
///   without rendering any preview. Dispatcher then ships rounds as one-shot
///   `send_message` calls.
///
/// Preview transport selection (when active):
/// - **Draft**: `send_draft` for Telegram private chats (no rate limit)
/// - **Card**: cardkit `create_card_stream` + `update_card_element` for
///   Feishu (host message never marked as edited)
/// - **Message**: `send_message` + `edit_message` for channels that only
///   support message edits (host message ends up showing "edited" badge)
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_channel_stream_task(
    mut event_rx: mpsc::UnboundedReceiver<String>,
    mut system_notice_rx: mpsc::UnboundedReceiver<String>,
    plugin: Arc<dyn ChannelPlugin>,
    target: ReplyStreamTarget,
    preview_transport: Option<StreamPreviewTransport>,
    max_msg_len: usize,
    reply_mode: ImReplyMode,
    round_texts: Arc<Mutex<RoundTextAccumulator>>,
    capabilities: ChannelCapabilities,
) -> tokio::task::JoinHandle<StreamPreviewOutcome> {
    tokio::spawn(async move {
        let account_id = target.account_id.clone();
        let chat_id = target.chat_id.clone();
        let thread_id = target.thread_id.clone();
        // Mutable: the reply quote belongs to the first legacy message of the
        // turn only. Native Split deliberately remains one stream for the
        // entire Agent turn and never enters per-round finalization.
        let mut reply_to_message_id = target.reply_to_message_id.clone();
        let Some(mut preview_transport) = preview_transport else {
            // No preview transport (Final mode or non-streaming channel):
            // drain `event_rx` while still shipping system notices as their
            // own one-shot messages, so the IM user still sees fallback /
            // compaction / thinking-auto-disabled notices.
            loop {
                tokio::select! {
                    notice = system_notice_rx.recv() => match notice {
                        Some(body) => send_system_notice_now(&plugin, &target, &body).await,
                        None => break,
                    },
                    event = event_rx.recv() => {
                        if event.is_none() { break; }
                    }
                }
            }
            // Drain anything still buffered after either channel closed.
            drain_system_notices(&mut system_notice_rx, &plugin, &target).await;
            while event_rx.recv().await.is_some() {}
            return StreamPreviewOutcome::default();
        };

        // Telegram animates draft updates that share the same `draft_id`.
        // Inbound turns reuse the user's incoming message id; live mirror
        // (no inbound to anchor against) falls back to the current
        // millisecond timestamp. Must be non-zero.
        let draft_id: i64 = reply_to_message_id
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|n| *n != 0)
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(1)
            });

        let mut accumulated = String::new();
        let mut preview_message_id: Option<String> = None;
        let mut message_preview_ack: Option<String> = None;
        let mut card_session: Option<CardSession> = None;
        let mut dirty = false;
        let mut native_frame_state = NativeFrameState::default();
        let mut native_phase = ReplyStreamPhase::Generating;
        // Tracks "saw a tool_call but not yet the next text_delta" — the
        // signal that the current round has closed and the next text_delta
        // (under split-streaming) must finalize this round before starting
        // the next preview.
        let mut in_tool_phase = false;
        // Number of rounds we've already shipped via per-round finalize.
        let mut finalized_rounds: usize = 0;
        let mut delivery_report = super::dispatcher::DeliveryReport::default();
        let mut flush_schedule = StreamPreviewFlushSchedule::new(Instant::now());

        loop {
            if native_keepalive_due(&preview_transport, &native_frame_state, Instant::now()) {
                if let Some(failure) = send_stream_preview(
                    &plugin,
                    &account_id,
                    &chat_id,
                    reply_to_message_id.as_deref(),
                    thread_id.as_deref(),
                    max_msg_len,
                    &accumulated,
                    draft_id,
                    &mut preview_transport,
                    &mut preview_message_id,
                    &mut message_preview_ack,
                    &mut card_session,
                    &mut native_frame_state,
                    native_phase,
                )
                .await
                {
                    delivery_report.merge(failure);
                    break;
                }
                continue;
            }
            // Check the clock before polling the receiver. If model deltas
            // arrive continuously, this prevents the preview flush from being
            // starved until EOF.
            if flush_schedule.should_flush(
                dirty,
                !accumulated.is_empty() || is_native_transport(&preview_transport),
                Instant::now(),
            ) {
                if let Some(failure) = send_stream_preview(
                    &plugin,
                    &account_id,
                    &chat_id,
                    reply_to_message_id.as_deref(),
                    thread_id.as_deref(),
                    max_msg_len,
                    &accumulated,
                    draft_id,
                    &mut preview_transport,
                    &mut preview_message_id,
                    &mut message_preview_ack,
                    &mut card_session,
                    &mut native_frame_state,
                    native_phase,
                )
                .await
                {
                    delivery_report.merge(failure);
                    break;
                }
                dirty = false;
                flush_schedule.mark_flushed(Instant::now());
                continue;
            }

            tokio::select! {
                notice = system_notice_rx.recv() => {
                    if let Some(body) = notice {
                        // Ship the notice as its own IM message — outside
                        // the per-round preview pipeline so it doesn't
                        // collide with `accumulated` / `preview_message_id`.
                        // Closed channel just means the engine dropped its
                        // sender; keep the loop running on `event_rx`.
                        send_system_notice_now(&plugin, &target, &body).await;
                    }
                }
                event = event_rx.recv() => {
                    match event {
                        Some(event_str) => {
                            // Detect round boundaries on the same cheap-string
                            // contract the sink uses (BTreeMap key order
                            // means `"type":"…"` lands mid-string). Order
                            // checks rarer-needle-first.
                            if event_str.contains("\"type\":\"tool_call\"") {
                                in_tool_phase = true;
                                if is_native_transport(&preview_transport) {
                                    native_frame_state.tasks.observe(&event_str);
                                    native_phase = ReplyStreamPhase::RunningTools;
                                    dirty = true;
                                }
                            } else if event_str.contains("\"type\":\"tool_result\"") {
                                if is_native_transport(&preview_transport) {
                                    native_frame_state.tasks.observe(&event_str);
                                    native_phase = ReplyStreamPhase::Generating;
                                    dirty = true;
                                }
                            } else if let Some(text) = extract_text_delta(&event_str) {
                                let split_streaming = matches!(reply_mode, ImReplyMode::Split)
                                    && !is_native_transport(&preview_transport);
                                if in_tool_phase && split_streaming {
                                    // Round just ended: flush + close current
                                    // preview, deliver this round's media,
                                    // then start a fresh preview for the new
                                    // round's first chunk.
                                    //
                                    // A round only *ships* a (quoted) message
                                    // when it had text — an empty round 0
                                    // (model calls a tool with no preamble)
                                    // sends nothing, so it must NOT spend the
                                    // quote, or the turn's first real message
                                    // (round 1+) would lose it.
                                    let round_shipped_text = !accumulated.is_empty();
                                    let report = finalize_split_round(
                                        &plugin, &target,
                                        reply_to_message_id.as_deref(), thread_id.as_deref(), max_msg_len,
                                        &accumulated, draft_id, &mut preview_transport,
                                        &mut preview_message_id, &mut message_preview_ack,
                                        &mut card_session,
                                        finalized_rounds, &round_texts, &capabilities,
                                    ).await;
                                    let halt_delivery = report.unsafe_to_continue;
                                    delivery_report.merge(report);
                                    flush_schedule.mark_flushed(Instant::now());
                                    accumulated.clear();
                                    finalized_rounds += 1;
                                    // Quote belongs to the turn's first shipped
                                    // message; once a round with text sends it,
                                    // later rounds reply un-quoted so a single
                                    // response doesn't stack a reply marker on
                                    // every round (Telegram / Feishu otherwise
                                    // quote each round's preview).
                                    if round_shipped_text {
                                        reply_to_message_id = None;
                                    }
                                    if halt_delivery {
                                        break;
                                    }
                                }
                                let new_preview_round = in_tool_phase && !split_streaming;
                                in_tool_phase = false;
                                native_phase = ReplyStreamPhase::Generating;
                                append_preview_round_text(&mut accumulated, &text, new_preview_round);
                                dirty = true;
                            }
                        }
                        None => {
                            let split_streaming = matches!(reply_mode, ImReplyMode::Split)
                                && !is_native_transport(&preview_transport);
                            if dirty && !accumulated.is_empty() {
                                if let Some(failure) = send_stream_preview(
                                    &plugin, &account_id, &chat_id,
                                    reply_to_message_id.as_deref(), thread_id.as_deref(), max_msg_len,
                                    &accumulated, draft_id, &mut preview_transport,
                                    &mut preview_message_id, &mut message_preview_ack,
                                    &mut card_session,
                                    &mut native_frame_state, native_phase,
                                ).await {
                                    delivery_report.merge(failure);
                                    break;
                                }
                            }
                            // Split mode + model ended on a tool_call: the
                            // last "round" has narration in `accumulated`
                            // and no further text will ever come. Finalize
                            // it inline so the dispatcher has nothing left
                            // to do.
                            if in_tool_phase && split_streaming {
                                let report = finalize_split_round(
                                    &plugin, &target,
                                    reply_to_message_id.as_deref(), thread_id.as_deref(), max_msg_len,
                                    &accumulated, draft_id, &mut preview_transport,
                                    &mut preview_message_id, &mut message_preview_ack,
                                    &mut card_session,
                                    finalized_rounds, &round_texts, &capabilities,
                                ).await;
                                let halt_delivery = report.unsafe_to_continue;
                                delivery_report.merge(report);
                                accumulated.clear();
                                preview_message_id = None;
                                message_preview_ack = None;
                                card_session = None;
                                finalized_rounds += 1;
                                if halt_delivery {
                                    break;
                                }
                            }
                            if is_native_transport(&preview_transport)
                                && (dirty
                                    || native_frame_state.revision > 0
                                    || !native_frame_state.tasks.tasks.is_empty())
                            {
                                native_phase = ReplyStreamPhase::Finalizing;
                                if let Some(failure) = send_stream_preview(
                                    &plugin, &account_id, &chat_id,
                                    reply_to_message_id.as_deref(), thread_id.as_deref(), max_msg_len,
                                    &accumulated, draft_id, &mut preview_transport,
                                    &mut preview_message_id, &mut message_preview_ack,
                                    &mut card_session,
                                    &mut native_frame_state, native_phase,
                                ).await {
                                    delivery_report.merge(failure);
                                    break;
                                }
                            }
                            drain_system_notices(
                                &mut system_notice_rx, &plugin, &target,
                            ).await;
                            break;
                        }
                    }
                }

                _ = tokio::time::sleep_until(flush_schedule.next_at()), if dirty && (!accumulated.is_empty() || is_native_transport(&preview_transport)) => {
                    if dirty && (!accumulated.is_empty() || is_native_transport(&preview_transport)) {
                        if let Some(failure) = send_stream_preview(
                            &plugin, &account_id, &chat_id,
                            reply_to_message_id.as_deref(), thread_id.as_deref(), max_msg_len,
                            &accumulated, draft_id, &mut preview_transport,
                            &mut preview_message_id, &mut message_preview_ack,
                            &mut card_session,
                            &mut native_frame_state, native_phase,
                        ).await {
                            delivery_report.merge(failure);
                            break;
                        }
                        dirty = false;
                        flush_schedule.mark_flushed(Instant::now());
                    }
                }

                _ = tokio::time::sleep(Duration::from_millis(500)), if is_native_transport(&preview_transport) => {}
            }
        }

        let preview = match &preview_transport {
            StreamPreviewTransport::Native {
                session,
                state,
                terminal_owner,
                capabilities,
                ..
            } => Some(PreviewHandle::Native {
                session: session.clone(),
                state: state.clone(),
                terminal_owner: terminal_owner.clone(),
                preview_persistence: capabilities.preview_persistence,
            }),
            StreamPreviewTransport::Card if card_session.is_some() => {
                let session = card_session.as_ref().expect("checked above");
                Some(PreviewHandle::Card {
                    card_id: session.card_id.clone(),
                    element_id: session.element_id.clone(),
                    sequence: session.sequence,
                    broken: session.broken,
                })
            }
            StreamPreviewTransport::Message if preview_message_id.is_some() => {
                Some(PreviewHandle::Message {
                    message_id: preview_message_id.as_ref().expect("checked above").clone(),
                })
            }
            StreamPreviewTransport::Draft if preview_message_id.is_some() => {
                Some(PreviewHandle::Message {
                    // Drafts normally do not persist a message id; retain the
                    // legacy defensive branch if an adapter supplied one.
                    message_id: preview_message_id.as_ref().expect("checked above").clone(),
                })
            }
            _ => None,
        };

        StreamPreviewOutcome {
            preview,
            finalized_rounds,
            delivery_report,
        }
    })
}

/// Ship a friendly system notice (model_fallback / profile_rotation /
/// context_compacted / thinking_auto_disabled) to the IM chat as its own
/// standalone message. Bypasses the per-round preview pipeline so notices
/// don't tangle with `accumulated` / `preview_message_id`. Failures only
/// log — system notices are best-effort UX, not data integrity.
async fn send_system_notice_now(
    plugin: &Arc<dyn ChannelPlugin>,
    native_target: &ReplyStreamTarget,
    body: &str,
) {
    let mut target = super::pipeline::DeliveryTarget::from(native_target);
    target.reply_to_message_id = None;
    super::dispatcher::send_text_chunks(plugin, &target, body, None, &[]).await;
}

/// Drain any system notices buffered when `event_rx` closed in the same
/// tick. Called from both the no-preview branch and the main loop's EOF
/// arm so a late notice still reaches the user.
async fn drain_system_notices(
    rx: &mut mpsc::UnboundedReceiver<String>,
    plugin: &Arc<dyn ChannelPlugin>,
    target: &ReplyStreamTarget,
) {
    while let Ok(body) = rx.try_recv() {
        send_system_notice_now(plugin, target, &body).await;
    }
}

/// Close the current round's preview and deliver its media. Called from
/// inside the stream task at split-streaming round boundaries (and at end
/// of stream when the model finished on a tool_call).
///
/// Delivery contract: always either ships the round's full narration via
/// the preview transport, or falls back to chunked `send_text_chunks`. The
/// preview path silently drops oversized text (`build_stream_preview_payload`
/// returns `None` when `text.len() > max_msg_len`) and turns transient
/// send/edit errors into log-only warnings, so the stream task can NOT
/// trust "preview ran" as proof of delivery. We detect that case explicitly
/// and fall back to chunked send so the dispatcher's `finalized_rounds`
/// skip is safe to act on.
///
/// Per transport:
/// - **Message**: if `accumulated` fits and the preview message exists,
///   the preview already wrote the final text; just drop `preview_message_id`.
///   Otherwise (oversized, or initial send never succeeded), chunk-send.
/// - **Card**: cardkit elements hold ~100k chars (`CARD_ELEMENT_MAX_CHARS`).
///   Before attach, creation failure can safely degrade to chunks. Once
///   visible, update failure/oversize is reported unsafe and never followed
///   by a fresh message; a confirmed full snapshot is closed best-effort.
/// - **Draft**: drafts are typing-indicators, not real messages. Always
///   chunk-send (handles oversized text correctly via `chunk_message`).
///
/// Then deliver this round's media items (read from `round_texts.completed`,
/// where the sink stashed them on tool_result events).
#[allow(clippy::too_many_arguments)]
async fn finalize_split_round(
    plugin: &Arc<dyn ChannelPlugin>,
    native_target: &ReplyStreamTarget,
    reply_to_message_id: Option<&str>,
    thread_id: Option<&str>,
    max_msg_len: usize,
    accumulated: &str,
    draft_id: i64,
    preview_transport: &mut StreamPreviewTransport,
    preview_message_id: &mut Option<String>,
    message_preview_ack: &mut Option<String>,
    card_session: &mut Option<CardSession>,
    round_idx: usize,
    round_texts: &Arc<Mutex<RoundTextAccumulator>>,
    capabilities: &ChannelCapabilities,
) -> super::dispatcher::DeliveryReport {
    let account_id = native_target.account_id.as_str();
    let chat_id = native_target.chat_id.as_str();
    let mut report = super::dispatcher::DeliveryReport::default();
    if !accumulated.is_empty() {
        let mut native_frame_state = NativeFrameState::default();
        if let Some(failure) = send_stream_preview(
            plugin,
            account_id,
            chat_id,
            reply_to_message_id,
            thread_id,
            max_msg_len,
            accumulated,
            draft_id,
            preview_transport,
            preview_message_id,
            message_preview_ack,
            card_session,
            &mut native_frame_state,
            ReplyStreamPhase::Generating,
        )
        .await
        {
            return failure;
        }
    }

    let accumulated_native = plugin.markdown_to_native(accumulated);
    let preview_carried_text = preview_carried_full_text(
        preview_transport,
        accumulated,
        &accumulated_native,
        preview_message_id.as_deref(),
        message_preview_ack.as_deref(),
        card_session.as_ref().map(|s| s.broken),
        max_msg_len,
    );

    let mut can_continue = true;
    if !preview_carried_text {
        let mut target = super::pipeline::DeliveryTarget::from(native_target);
        target.thread_id = thread_id;
        target.reply_to_message_id = reply_to_message_id;
        let message_preview = if matches!(preview_transport, StreamPreviewTransport::Message) {
            preview_message_id
                .as_ref()
                .map(|message_id| PreviewHandle::Message {
                    message_id: message_id.clone(),
                })
        } else {
            None
        };
        let text_report = super::dispatcher::send_text_chunks(
            plugin,
            &target,
            accumulated,
            message_preview.as_ref(),
            &[],
        )
        .await;
        can_continue = !text_report.unsafe_to_continue;
        report.merge(text_report);
    } else if !accumulated.is_empty() {
        // A preview handle only survives when the platform accepted the full
        // final text. Count that visible message as this round's delivery.
        report.attempted += 1;
        report.succeeded += 1;
    }

    // 3. Transport-specific close. Best-effort: any error here is
    //    cosmetic (the text is already delivered above), so log + continue.
    if let StreamPreviewTransport::Card = preview_transport {
        if let Some(session) = card_session.take() {
            if !session.broken {
                if let Err(e) = plugin
                    .close_card_stream(account_id, &session.card_id, session.sequence)
                    .await
                {
                    app_warn!(
                        "channel",
                        "worker",
                        "split-streaming close_card_stream failed (seq={}): {}",
                        session.sequence,
                        e
                    );
                }
            }
        }
    }
    *preview_message_id = None;
    *message_preview_ack = None;
    *card_session = None;

    if !can_continue {
        return report;
    }

    // 3. Deliver this round's media. The sink attached items to
    //    `round_texts.completed[round_idx]` on `tool_result` arrival.
    //    Dispatcher's end-of-turn `deliver_split` only iterates rounds
    //    past `finalized_rounds`, so this round's media won't be redelivered.
    let medias = {
        let guard = round_texts.lock().unwrap_or_else(|e| {
            app_warn!(
                "channel",
                "worker",
                "round_texts mutex poisoned in stream task: {}",
                e
            );
            e.into_inner()
        });
        guard.round_medias(round_idx)
    };
    if !medias.is_empty() {
        let media_target = super::pipeline::DeliveryTarget::from(native_target);
        report.merge(deliver_media_to_chat(plugin, &media_target, &medias, capabilities).await);
    }
    report
}

/// Pure helper for the split-streaming round-finalize delivery decision.
///
/// `accumulated_native` is `markdown_to_native(accumulated)` and must exactly
/// match the last acknowledged Message snapshot. `card_session_broken` is
/// `Some(broken_flag)` if a card session exists, `None` otherwise.
///
/// Returns `true` when the existing preview state has demonstrably carried
/// the full round narration — caller can stop. `false` means caller must
/// chunk-and-send `accumulated` itself; the preview either silently dropped
/// oversized content or never opened (initial send/edit error, oversized
/// from the first delta).
pub(super) fn preview_carried_full_text(
    transport: &StreamPreviewTransport,
    accumulated: &str,
    accumulated_native: &str,
    preview_message_id: Option<&str>,
    message_preview_ack: Option<&str>,
    card_session_broken: Option<bool>,
    max_msg_len: usize,
) -> bool {
    if accumulated.is_empty() {
        return true;
    }
    match transport {
        StreamPreviewTransport::Native { .. } => false,
        StreamPreviewTransport::Message => {
            preview_message_id.is_some()
                && accumulated_native.len() <= max_msg_len
                && message_preview_ack == Some(accumulated_native)
        }
        StreamPreviewTransport::Card => {
            // `Some(false)` = card exists and isn't broken
            matches!(card_session_broken, Some(false))
                && accumulated.chars().count() <= CARD_ELEMENT_MAX_CHARS
        }
        // Drafts are typing indicators, not real messages — always need a
        // real `send_message` (which the chunk fallback does, correctly
        // splitting oversized text).
        StreamPreviewTransport::Draft => false,
        StreamPreviewTransport::Disabled => false,
    }
}

/// Mutable state for an active card-streaming session. Only used inside
/// `spawn_channel_stream_task`; finalization-time fields are exported via
/// `PreviewHandle::Card`.
#[derive(Debug)]
struct CardSession {
    card_id: String,
    element_id: String,
    /// Next sequence number to use on `update_card_element`. Strictly
    /// monotonic per cardkit's contract.
    sequence: i64,
    /// True once an `update_card_element` failure made the visible outcome
    /// ambiguous. Finalization must stop instead of switching transports.
    broken: bool,
}

/// Extract text from a `text_delta` event JSON string.
pub(super) fn extract_text_delta(event_str: &str) -> Option<String> {
    let event: serde_json::Value = serde_json::from_str(event_str).ok()?;
    if event.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    event
        .get("content")
        .or_else(|| event.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub(super) fn select_stream_preview_transport(
    target: &ReplyStreamTarget,
    capabilities: &ChannelCapabilities,
    history_complete: bool,
) -> Option<StreamPreviewTransport> {
    if let Some(native) = capabilities.native_reply.as_ref() {
        let target_eligible = native.preview_chat_types.contains(&target.chat_type)
            && native.final_chat_types.contains(&target.chat_type)
            && (!native.requires_reply_anchor || target.reply_to_message_id.is_some())
            && (!native.requires_recipient_user_id || target.recipient_user_id.is_some())
            && (!native.requires_recipient_tenant_id || target.recipient_tenant_id.is_some());
        let history_eligible =
            history_complete || !matches!(native.update_mode, ReplyStreamUpdateMode::Append);
        if target_eligible && history_eligible {
            return Some(StreamPreviewTransport::Native {
                target: target.clone(),
                capabilities: native.clone(),
                legacy_capabilities: capabilities.clone(),
                session: Arc::new(tokio::sync::Mutex::new(None)),
                state: Arc::new(AtomicU8::new(NATIVE_SELECTED)),
                native_preview_relinquished: Arc::new(AtomicBool::new(false)),
                terminal_owner: Arc::new(AtomicU8::new(NATIVE_TERMINAL_UNCLAIMED)),
            });
        }
    }
    if matches!(target.chat_type, ChatType::Dm) && capabilities.supports_draft {
        return Some(StreamPreviewTransport::Draft);
    }
    if capabilities.supports_card_stream {
        return Some(StreamPreviewTransport::Card);
    }
    if capabilities.supports_edit {
        return Some(StreamPreviewTransport::Message);
    }
    None
}

fn select_legacy_preview_transport(
    target: &ReplyStreamTarget,
    capabilities: &ChannelCapabilities,
) -> Option<StreamPreviewTransport> {
    if matches!(target.chat_type, ChatType::Dm) && capabilities.supports_draft {
        return Some(StreamPreviewTransport::Draft);
    }
    if capabilities.supports_card_stream {
        return Some(StreamPreviewTransport::Card);
    }
    if capabilities.supports_edit {
        return Some(StreamPreviewTransport::Message);
    }
    None
}

fn is_native_transport(transport: &StreamPreviewTransport) -> bool {
    matches!(transport, StreamPreviewTransport::Native { .. })
}

fn native_keepalive_due(
    transport: &StreamPreviewTransport,
    frame_state: &NativeFrameState,
    now: Instant,
) -> bool {
    let StreamPreviewTransport::Native {
        capabilities,
        state,
        ..
    } = transport
    else {
        return false;
    };
    if state.load(Ordering::Acquire) != NATIVE_ACTIVE {
        return false;
    }
    let (Some(last), Some(refresh_secs)) = (
        frame_state.last_acknowledged_at,
        capabilities.refresh_after_secs,
    ) else {
        return false;
    };
    now >= last + Duration::from_secs(refresh_secs.max(1))
}

fn safe_open_fallback(kind: ReplyStreamErrorKind) -> bool {
    matches!(
        kind,
        ReplyStreamErrorKind::Unsupported
            | ReplyStreamErrorKind::InvalidTarget
            | ReplyStreamErrorKind::InvalidContent
            | ReplyStreamErrorKind::Rejected
            | ReplyStreamErrorKind::RateLimited
            | ReplyStreamErrorKind::Transient
    )
}

/// Claim the one legal `Selected -> Opening` transition without ever
/// overwriting a concurrent cancellation's terminal state.
pub(super) fn try_begin_native_open(state: &AtomicU8) -> bool {
    state
        .compare_exchange(
            NATIVE_SELECTED,
            NATIVE_OPENING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

enum NativeFlushResult {
    Acknowledged,
    Skipped,
    Fallback,
}

pub(super) fn build_native_frame(
    accumulated: &str,
    frame_state: &NativeFrameState,
    capabilities: &NativeReplyCapabilities,
    phase: ReplyStreamPhase,
) -> ReplyStreamFrame {
    let (markdown_snapshot, markdown_delta) = match capabilities.update_mode {
        ReplyStreamUpdateMode::Append => {
            let unacknowledged = accumulated
                .get(frame_state.acknowledged_bytes..)
                .unwrap_or_default();
            let delta = truncate_chars(unacknowledged, capabilities.max_delta_chars);
            let acknowledged_end = frame_state
                .acknowledged_bytes
                .saturating_add(delta.len())
                .min(accumulated.len());
            let snapshot = accumulated
                .get(..acknowledged_end)
                .unwrap_or(accumulated)
                .to_string();
            (snapshot, delta)
        }
        // Snapshot is the canonical full document by contract. Adapters with
        // a smaller visible window derive it locally; clipping here would
        // force them to consume `markdown_delta`. Keeping delta empty also
        // makes refresh-only revisions unambiguous.
        ReplyStreamUpdateMode::Snapshot => (accumulated.to_string(), String::new()),
    };
    ReplyStreamFrame {
        revision: frame_state.revision.saturating_add(1),
        markdown_snapshot,
        markdown_delta,
        phase,
        tasks: frame_state
            .tasks
            .snapshot(capabilities.supports_task_updates),
        plan_title: capabilities.supports_plan_updates.then(|| {
            match phase {
                ReplyStreamPhase::Generating => "正在生成回复",
                ReplyStreamPhase::RunningTools => "正在执行任务",
                ReplyStreamPhase::Finalizing => "正在整理结果",
            }
            .to_string()
        }),
    }
}

pub(super) fn acknowledge_native_frame(
    frame_state: &mut NativeFrameState,
    frame: &ReplyStreamFrame,
    update_mode: ReplyStreamUpdateMode,
) {
    frame_state.acknowledged_bytes = if matches!(update_mode, ReplyStreamUpdateMode::Append) {
        frame_state
            .acknowledged_bytes
            .saturating_add(frame.markdown_delta.len())
    } else {
        frame_state
            .acknowledged_bytes
            .max(frame.markdown_snapshot.len())
    };
    frame_state.revision = frame.revision;
    frame_state.phase = Some(frame.phase);
    frame_state.last_acknowledged_at = Some(Instant::now());
}

#[allow(clippy::too_many_arguments)]
async fn flush_native_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &ReplyStreamTarget,
    capabilities: &NativeReplyCapabilities,
    session: &SharedNativeReplySession,
    state: &Arc<AtomicU8>,
    accumulated: &str,
    phase: ReplyStreamPhase,
    frame_state: &mut NativeFrameState,
) -> NativeFlushResult {
    let lifecycle = state.load(Ordering::Acquire);
    if matches!(
        lifecycle,
        NATIVE_BROKEN | NATIVE_AMBIGUOUS | NATIVE_TERMINAL
    ) {
        return NativeFlushResult::Skipped;
    }

    let frame = build_native_frame(accumulated, frame_state, capabilities, phase);
    if lifecycle == NATIVE_SELECTED {
        if !try_begin_native_open(state) {
            return NativeFlushResult::Skipped;
        }
        // Hold the exact shared slot while opening. Cancellation waits on this
        // same lock (bounded by the pipeline's 3 second deadline), preventing
        // an accepted session from being installed after an abort completed.
        let mut guard = session.lock().await;
        if state.load(Ordering::Acquire) != NATIVE_OPENING {
            return NativeFlushResult::Skipped;
        }
        match plugin.open_reply_stream(target, &frame).await {
            Ok(stream) => {
                if state.load(Ordering::Acquire) == NATIVE_TERMINAL {
                    state.store(NATIVE_ABORTING, Ordering::Release);
                    drop(guard);
                    let abort_task = spawn_native_abort(
                        stream,
                        ReplyAbortReason::Cancelled,
                        state.clone(),
                        NATIVE_TERMINAL,
                    );
                    if !await_native_abort_task(abort_task, state, NATIVE_ABORT_WAIT_TIMEOUT).await
                    {
                        state.store(NATIVE_AMBIGUOUS, Ordering::Release);
                    }
                    return NativeFlushResult::Skipped;
                }
                *guard = Some(stream);
                state.store(NATIVE_ACTIVE, Ordering::Release);
                drop(guard);
                acknowledge_native_frame(frame_state, &frame, capabilities.update_mode);
                NativeFlushResult::Acknowledged
            }
            Err(error) if safe_open_fallback(error.kind) => {
                state.store(NATIVE_TERMINAL, Ordering::Release);
                drop(guard);
                app_warn!(
                    "channel",
                    "worker",
                    "Native reply open rejected safely; relinquishing native preview: {}",
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
                NativeFlushResult::Fallback
            }
            Err(error) => {
                if matches!(
                    capabilities.preview_persistence,
                    ReplyStreamPreviewPersistence::Ephemeral
                ) {
                    // A rich draft is only an expiring visualization. Even
                    // when acceptance is unknown, stop refreshing it while
                    // keeping the durable final lane available.
                    state.store(NATIVE_BROKEN, Ordering::Release);
                    drop(guard);
                    app_warn!(
                        "channel",
                        "worker",
                        "Ephemeral native preview open failed; final delivery remains available: {}",
                        ha_core::logging::redact_sensitive(&error.to_string())
                    );
                    return NativeFlushResult::Skipped;
                }
                // Ambiguous and every unlisted kind suppress fallback: a
                // provider-visible stream may already exist.
                state.store(NATIVE_AMBIGUOUS, Ordering::Release);
                drop(guard);
                app_warn!(
                    "channel",
                    "worker",
                    "Native reply open outcome is unsafe to retry: {}",
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
                NativeFlushResult::Skipped
            }
        }
    } else if lifecycle == NATIVE_ACTIVE {
        let mut guard = session.lock().await;
        if state.load(Ordering::Acquire) != NATIVE_ACTIVE {
            return NativeFlushResult::Skipped;
        }
        let Some(stream) = guard.as_mut() else {
            if state.load(Ordering::Acquire) != NATIVE_TERMINAL {
                state.store(NATIVE_BROKEN, Ordering::Release);
            }
            return NativeFlushResult::Skipped;
        };
        match stream.push(&frame).await {
            Ok(()) => {
                if state.load(Ordering::Acquire) == NATIVE_TERMINAL {
                    let stream = guard.take();
                    state.store(NATIVE_ABORTING, Ordering::Release);
                    drop(guard);
                    if let Some(stream) = stream {
                        // Detached terminal handoff: cancellation may stop
                        // polling this stream task immediately after the lock
                        // becomes available, but the provider abort continues.
                        drop(spawn_native_abort(
                            stream,
                            ReplyAbortReason::Cancelled,
                            state.clone(),
                            NATIVE_TERMINAL,
                        ));
                    }
                    return NativeFlushResult::Skipped;
                }
                drop(guard);
                acknowledge_native_frame(frame_state, &frame, capabilities.update_mode);
                NativeFlushResult::Acknowledged
            }
            Err(error) => {
                let cancelled = state.load(Ordering::Acquire) == NATIVE_TERMINAL;
                if !cancelled
                    && matches!(
                        capabilities.preview_persistence,
                        ReplyStreamPreviewPersistence::Ephemeral
                    )
                {
                    // Keep the stream object solely as the exactly-once
                    // terminal commit carrier. No more preview mutations are
                    // attempted, and the independent final remains available.
                    state.store(NATIVE_BROKEN, Ordering::Release);
                    drop(guard);
                    app_warn!(
                        "channel",
                        "worker",
                        "Ephemeral native preview update failed; final delivery remains available: {}",
                        ha_core::logging::redact_sensitive(&error.to_string())
                    );
                    return NativeFlushResult::Skipped;
                }
                let ambiguous = matches!(error.kind, ReplyStreamErrorKind::Ambiguous);
                let terminal_state = if cancelled {
                    NATIVE_TERMINAL
                } else if ambiguous {
                    NATIVE_AMBIGUOUS
                } else {
                    NATIVE_BROKEN
                };
                let stream = guard.take();
                state.store(NATIVE_ABORTING, Ordering::Release);
                drop(guard);
                if let Some(stream) = stream {
                    drop(spawn_native_abort(
                        stream,
                        if cancelled {
                            ReplyAbortReason::Cancelled
                        } else {
                            ReplyAbortReason::Failed
                        },
                        state.clone(),
                        terminal_state,
                    ));
                } else {
                    state.store(terminal_state, Ordering::Release);
                }
                app_warn!(
                    "channel",
                    "worker",
                    "Native reply push failed{}: {}",
                    if ambiguous { " ambiguously" } else { "" },
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
                NativeFlushResult::Skipped
            }
        }
    } else {
        NativeFlushResult::Skipped
    }
}

pub(super) fn should_fallback_from_draft_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("sendmessagedraft")
        && (lower.contains("unknown method")
            || lower.contains("not found")
            || lower.contains("not available")
            || lower.contains("not supported")
            || lower.contains("unsupported")
            || lower.contains("private chat")
            || lower.contains("can be used only"))
}

pub(super) fn build_stream_preview_payload(
    plugin: &Arc<dyn ChannelPlugin>,
    reply_to_message_id: Option<&str>,
    thread_id: Option<&str>,
    text: &str,
    draft_id: i64,
    max_msg_len: usize,
) -> Option<ReplyPayload> {
    let native_text = plugin.markdown_to_native(text);
    let text = native_text.trim_end();
    if text.is_empty() || text.len() > max_msg_len {
        return None;
    }

    Some(ReplyPayload {
        text: Some(text.to_string()),
        reply_to_message_id: reply_to_message_id.map(str::to_string),
        thread_id: thread_id.map(|s| s.to_string()),
        parse_mode: Some(ParseMode::Html),
        draft_id: Some(draft_id),
        ..ReplyPayload::text("")
    })
}

fn unsafe_preview_delivery(error: String) -> super::dispatcher::DeliveryReport {
    super::dispatcher::DeliveryReport {
        attempted: 1,
        succeeded: 0,
        failures: vec![error],
        unsafe_to_continue: true,
    }
}

async fn send_message_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    payload: &ReplyPayload,
    preview_message_id: &mut Option<String>,
    message_preview_ack: &mut Option<String>,
) -> Option<super::dispatcher::DeliveryReport> {
    if let Some(message_id) = preview_message_id.clone() {
        match plugin
            .edit_message(account_id, chat_id, &message_id, payload)
            .await
        {
            Ok(result) if result.success => {
                *message_preview_ack = payload.text.clone();
            }
            Ok(result) => {
                app_warn!(
                    "channel",
                    "worker",
                    "stream preview edit failed: {}",
                    ha_core::logging::redact_sensitive(
                        &result
                            .error
                            .unwrap_or_else(|| "platform rejected edit".to_string())
                    )
                );
                // Legacy DeliveryResult has no typed zero-delivery proof.
                // Retain the same id so a later revision can only retry the
                // idempotent edit, never create a duplicate preview message.
            }
            Err(error) => {
                app_warn!(
                    "channel",
                    "worker",
                    "stream preview edit failed: {}",
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
                // A transport error may arrive after the edit was accepted.
                // Keep the id and suppress any fresh-message fallback.
            }
        }
        return None;
    }

    match plugin.send_message(account_id, chat_id, payload).await {
        Ok(result) if result.success => {
            if let Some(message_id) = result.message_id.filter(|id| !id.trim().is_empty()) {
                *preview_message_id = Some(message_id);
                *message_preview_ack = payload.text.clone();
                None
            } else {
                let error =
                    "stream preview send was acknowledged without a message identifier".to_string();
                app_warn!("channel", "worker", "{}", error);
                Some(unsafe_preview_delivery(error))
            }
        }
        Ok(result) => {
            let error = ha_core::logging::redact_sensitive(
                result
                    .error
                    .as_deref()
                    .unwrap_or("platform rejected preview"),
            );
            app_warn!("channel", "worker", "stream preview send failed: {}", error);
            Some(unsafe_preview_delivery(error))
        }
        Err(e) => {
            let error = ha_core::logging::redact_sensitive(&e.to_string());
            app_warn!("channel", "worker", "stream preview send failed: {}", error);
            Some(unsafe_preview_delivery(error))
        }
    }
}

enum CardPreviewError {
    SafeBeforeAttach(String),
    AmbiguousAttach(String),
    VisibleMutation(String),
}

/// Lazy-create the card on first preview, then update its single
/// element on subsequent ticks. A create failure occurs before any visible
/// chat mutation and can safely degrade; an unacknowledged attach cannot.
/// Once the card is visible, an update error or an oversized snapshot is
/// terminal: a fresh text fallback could duplicate an accepted card update.
async fn send_card_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: Option<&str>,
    thread_id: Option<&str>,
    raw_text: &str,
    card_session: &mut Option<CardSession>,
) -> Result<(), CardPreviewError> {
    if raw_text.is_empty() {
        return Ok(());
    }

    let raw_chars = raw_text.chars().count();
    if raw_chars > CARD_ELEMENT_MAX_CHARS {
        return if card_session.is_some() {
            Err(CardPreviewError::VisibleMutation(format!(
                "visible card snapshot has {raw_chars} characters; maximum is {CARD_ELEMENT_MAX_CHARS}"
            )))
        } else {
            Err(CardPreviewError::SafeBeforeAttach(format!(
                "initial card snapshot has {raw_chars} characters; maximum is {CARD_ELEMENT_MAX_CHARS}"
            )))
        };
    }

    if let Some(session) = card_session.as_mut() {
        if session.broken {
            return Err(CardPreviewError::VisibleMutation(
                "visible card session is already in an ambiguous state".to_string(),
            ));
        }
        let next_seq = session.sequence;
        match plugin
            .update_card_element(
                account_id,
                &session.card_id,
                &session.element_id,
                raw_text,
                next_seq,
            )
            .await
        {
            Ok(()) => {
                session.sequence = next_seq + 1;
            }
            Err(e) => {
                let error = format!("update_card_element (seq={next_seq}): {e}");
                app_warn!(
                    "channel",
                    "worker",
                    "card stream update outcome is ambiguous (seq={}): {}",
                    next_seq,
                    e
                );
                session.broken = true;
                return Err(CardPreviewError::VisibleMutation(error));
            }
        }
        return Ok(());
    }

    let handle = plugin
        .create_card_stream(account_id, raw_text)
        .await
        .map_err(|e| CardPreviewError::SafeBeforeAttach(format!("create_card_stream: {e}")))?;
    let delivery = plugin
        .send_card_message(
            account_id,
            chat_id,
            &handle.card_id,
            reply_to_message_id,
            thread_id,
        )
        .await
        .map_err(|e| CardPreviewError::AmbiguousAttach(format!("send_card_message: {e}")))?;
    if !delivery.success {
        return Err(CardPreviewError::AmbiguousAttach(format!(
            "send_card_message failed: {}",
            delivery.error.unwrap_or_default()
        )));
    }
    *card_session = Some(CardSession {
        card_id: handle.card_id,
        element_id: handle.element_id,
        // Initial content was set during create. First explicit update
        // starts at sequence=1 (cardkit treats create as sequence-less).
        sequence: 1,
        broken: false,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_stream_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: Option<&str>,
    thread_id: Option<&str>,
    max_msg_len: usize,
    text: &str,
    draft_id: i64,
    preview_transport: &mut StreamPreviewTransport,
    preview_message_id: &mut Option<String>,
    message_preview_ack: &mut Option<String>,
    card_session: &mut Option<CardSession>,
    native_frame_state: &mut NativeFrameState,
    native_phase: ReplyStreamPhase,
) -> Option<super::dispatcher::DeliveryReport> {
    let native = match preview_transport {
        StreamPreviewTransport::Native {
            target,
            capabilities,
            legacy_capabilities,
            session,
            state,
            native_preview_relinquished,
            ..
        } => Some((
            target.clone(),
            capabilities.clone(),
            legacy_capabilities.clone(),
            session.clone(),
            state.clone(),
            native_preview_relinquished.clone(),
        )),
        _ => None,
    };
    if let Some((
        target,
        capabilities,
        legacy_capabilities,
        session,
        state,
        native_preview_relinquished,
    )) = native
    {
        match flush_native_preview(
            plugin,
            &target,
            &capabilities,
            &session,
            &state,
            text,
            native_phase,
            native_frame_state,
        )
        .await
        {
            NativeFlushResult::Fallback => {
                // Publish the handoff before changing transport or attempting
                // any legacy mutation. A later task failure must never revive
                // the stale native handle captured at spawn time.
                native_preview_relinquished.store(true, Ordering::Release);
                let Some(fallback) = select_legacy_preview_transport(&target, &legacy_capabilities)
                else {
                    *preview_transport = StreamPreviewTransport::Disabled;
                    return None;
                };
                *preview_transport = fallback;
            }
            NativeFlushResult::Acknowledged | NativeFlushResult::Skipped => return None,
        }
    }

    // Lazy native-format payload for Draft / Message paths. The Card path
    // sends the raw markdown directly (cardkit markdown elements don't
    // want HTML conversion), so it skips this builder unless it has to
    // degrade to Message mid-flight.
    let build_payload = || {
        build_stream_preview_payload(
            plugin,
            reply_to_message_id,
            thread_id,
            text,
            draft_id,
            max_msg_len,
        )
    };

    match preview_transport {
        StreamPreviewTransport::Native { .. } => unreachable!("native branch returned above"),
        StreamPreviewTransport::Draft => {
            let Some(payload) = build_payload() else {
                return None;
            };
            if let Err(e) = plugin.send_draft(account_id, chat_id, &payload).await {
                if should_fallback_from_draft_error(&e.to_string()) {
                    app_warn!(
                        "channel",
                        "worker",
                        "send_draft unavailable, falling back to send/edit preview: {}",
                        e
                    );
                    *preview_transport = StreamPreviewTransport::Message;
                    return send_message_preview(
                        plugin,
                        account_id,
                        chat_id,
                        &payload,
                        preview_message_id,
                        message_preview_ack,
                    )
                    .await;
                } else {
                    app_warn!("channel", "worker", "send_draft failed: {}", e);
                }
            }
            None
        }
        StreamPreviewTransport::Card => {
            match send_card_preview(
                plugin,
                account_id,
                chat_id,
                reply_to_message_id,
                thread_id,
                text,
                card_session,
            )
            .await
            {
                Ok(()) => None,
                Err(CardPreviewError::SafeBeforeAttach(error)) => {
                    // The card resource is not visible until attach succeeds,
                    // so a create-stage failure can safely choose Message.
                    let error = ha_core::logging::redact_sensitive(&error);
                    app_warn!(
                        "channel",
                        "worker",
                        "card stream create failed, falling back to message edit: {}",
                        error
                    );
                    *preview_transport = StreamPreviewTransport::Message;
                    match build_payload() {
                        Some(payload) => {
                            send_message_preview(
                                plugin,
                                account_id,
                                chat_id,
                                &payload,
                                preview_message_id,
                                message_preview_ack,
                            )
                            .await
                        }
                        None => None,
                    }
                }
                Err(CardPreviewError::AmbiguousAttach(error)) => {
                    // send_card_message is the first visible mutation. A
                    // missing acknowledgement must never be followed by a
                    // fresh Message that could duplicate an accepted card.
                    let error = ha_core::logging::redact_sensitive(&error);
                    app_warn!(
                        "channel",
                        "worker",
                        "card stream attach outcome is ambiguous: {}",
                        error
                    );
                    Some(unsafe_preview_delivery(error))
                }
                Err(CardPreviewError::VisibleMutation(error)) => {
                    // The card is already visible. Re-sending the accumulated
                    // snapshot as a new message could duplicate an update that
                    // the provider accepted but failed to acknowledge.
                    let error = ha_core::logging::redact_sensitive(&error);
                    app_warn!(
                        "channel",
                        "worker",
                        "visible card stream can no longer be updated safely: {}",
                        error
                    );
                    Some(unsafe_preview_delivery(error))
                }
            }
        }
        StreamPreviewTransport::Message => {
            let Some(payload) = build_payload() else {
                return None;
            };
            send_message_preview(
                plugin,
                account_id,
                chat_id,
                &payload,
                preview_message_id,
                message_preview_ack,
            )
            .await
        }
        StreamPreviewTransport::Disabled => None,
    }
}
