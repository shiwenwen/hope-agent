use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::agent::{AssistantAgent, PlanResolvedContext};
use crate::attachments::MediaItem;
use crate::chat_engine::stream_broadcast::EVENT_CHANNEL_STREAM_DELTA;
use crate::chat_engine::stream_seq::ChatSource;
use crate::context_compact::CompactConfig;
use crate::provider::{ActiveModel, ProviderConfig};
use crate::session::SessionDB;

// ── Shared Types ────────────────────────────────────────────────────

/// Token usage and metrics captured from streaming callbacks.
/// See `ChatUsage` for the `input_tokens` vs `last_input_tokens` split.
///
/// Public so `src-tauri` callsites that run chat outside of `run_chat_engine`
/// (e.g. the empty-model-chain fallback in `commands/chat.rs`) can reuse the
/// same capture shape instead of hand-rolling positional tuples.
#[derive(Default, Clone)]
pub struct CapturedUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub last_input_tokens: Option<i64>,
    pub model: Option<String>,
    pub ttft_ms: Option<i64>,
    /// Cache-creation input tokens (Anthropic prompt cache write).
    pub cache_creation_input_tokens: Option<i64>,
    /// Cache-read input tokens (Anthropic prompt cache hit or
    /// OpenAI-style `input_tokens_details.cached_tokens`).
    pub cache_read_input_tokens: Option<i64>,
}

impl CapturedUsage {
    /// Fold a `{"type":"usage", ...}` stream event into this struct. Only
    /// fields actually present in the event overwrite prior values.
    /// Mirror of the dispatch inside `StreamPersister::build_callback`.
    pub fn absorb_event(&mut self, event: &serde_json::Value) {
        if let Some(v) = event.get("input_tokens").and_then(|v| v.as_i64()) {
            self.input_tokens = Some(v);
        }
        if let Some(v) = event.get("output_tokens").and_then(|v| v.as_i64()) {
            self.output_tokens = Some(v);
        }
        if let Some(v) = event.get("last_input_tokens").and_then(|v| v.as_i64()) {
            self.last_input_tokens = Some(v);
        }
        if let Some(v) = event.get("model").and_then(|v| v.as_str()) {
            self.model = Some(v.to_string());
        }
        if let Some(v) = event.get("ttft_ms").and_then(|v| v.as_i64()) {
            self.ttft_ms = Some(v);
        }
        if let Some(v) = event
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_i64())
        {
            self.cache_creation_input_tokens = Some(v);
        }
        if let Some(v) = event
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_i64())
        {
            self.cache_read_input_tokens = Some(v);
        }
    }
}

// ── EventSink trait ─────────────────────────────────────────────────

/// Abstract output layer for chat events.
/// UI chat uses a Tauri-side `ChannelSink` (in src-tauri),
/// IM channel worker uses `ChannelStreamSink` (event bus emit).
pub trait EventSink: Send + Sync + 'static {
    fn send(&self, event: &str);
}

/// EventSink that drops every event. Used by callers that don't have a
/// real-time UI consumer (HTTP one-shot, cron, subagent fork-and-forget).
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn send(&self, _event: &str) {}
}

/// EventSink for IM channel worker — pushes streaming events via the global EventBus
/// AND forwards them to a background task for progressive Telegram message editing.
///
/// Also accumulates any `media_items[]` emitted in `tool_result` events into
/// `pending_media`, which the dispatcher drains after the chat engine finishes
/// to deliver attachments through the channel's native media API.
pub struct ChannelStreamSink {
    pub session_id: String,
    /// Forwards raw events to the channel streaming background task.
    pub event_tx: tokio::sync::mpsc::Sender<String>,
    /// Accumulates media items (from `send_attachment`, `image_generate`, ...)
    /// so the dispatcher can deliver them through the channel after the turn
    /// completes. The dispatcher owns the same `Arc` and drains this vec once
    /// `run_chat_engine` returns.
    pub pending_media: Arc<Mutex<Vec<MediaItem>>>,
}

impl ChannelStreamSink {
    pub fn new(
        session_id: String,
        event_tx: tokio::sync::mpsc::Sender<String>,
        pending_media: Arc<Mutex<Vec<MediaItem>>>,
    ) -> Self {
        Self {
            session_id,
            event_tx,
            pending_media,
        }
    }
}

impl EventSink for ChannelStreamSink {
    fn send(&self, event: &str) {
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                EVENT_CHANNEL_STREAM_DELTA,
                serde_json::json!({
                    "sessionId": &self.session_id,
                    "event": event,
                }),
            );
        }
        // Cheap short-circuit: only tool_result events carry media_items, and
        // only they start with {"type":"tool_result"...}. Avoids a full JSON
        // parse on every text_delta / tool_call frame.
        if event.starts_with("{\"type\":\"tool_result\"") && event.contains("\"media_items\"") {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                if let Some(arr) = val.get("media_items").and_then(|v| v.as_array()) {
                    let items: Vec<MediaItem> = arr
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    if !items.is_empty() {
                        if let Ok(mut guard) = self.pending_media.lock() {
                            guard.extend(items);
                        }
                    }
                }
            }
        }
        let _ = self.event_tx.try_send(event.to_string());
    }
}

// ── ChatEngineParams ────────────────────────────────────────────────

/// All parameters needed by the chat engine. Callers extract these from
/// `State<AppState>` (UI chat) or disk (channel worker).
pub struct ChatEngineParams {
    // Basic
    pub session_id: String,
    pub agent_id: String,
    pub message: String,
    pub attachments: Vec<crate::agent::Attachment>,
    pub session_db: Arc<SessionDB>,

    // Model chain (pre-resolved by caller)
    pub model_chain: Vec<ActiveModel>,
    /// Provider configs needed to build agents (snapshot, not reference to State)
    pub providers: Vec<ProviderConfig>,
    /// Codex OAuth token, if available
    pub codex_token: Option<(String, String)>,

    // Agent configuration
    pub resolved_temperature: Option<f64>,
    pub compact_config: CompactConfig,

    // Optional
    pub extra_system_context: Option<String>,
    pub reasoning_effort: Option<String>,
    pub cancel: Arc<AtomicBool>,
    /// Spawn-supplied Plan-mode override. `Some` means the caller is the
    /// source of truth and the chat engine must NOT consult this session's
    /// backend `plan_mode` (used by `spawn_plan_subagent`: the child
    /// session's `plan_mode` is `Off`, but the spawn caller wants
    /// `PlanAgent`). `None` (the common case for chat.rs / HTTP / channel /
    /// cron) lets the chat engine read backend `plan_mode` itself and the
    /// streaming loop's mid-turn probe stays free to re-sync after
    /// `enter_plan_mode` flips state.
    pub plan_context_override: Option<PlanResolvedContext>,
    /// Skill-level tool restriction (set when a skill with `allowed-tools` is activated)
    pub skill_allowed_tools: Vec<String>,
    /// Tools denied by the caller's execution policy.
    pub denied_tools: Vec<String>,
    /// Current sub-agent nesting depth for tool schema filtering and child spawns.
    pub subagent_depth: u32,
    /// Sub-agent run id whose steer mailbox should be drained each tool round.
    pub steer_run_id: Option<String>,

    /// When true, all tool calls are auto-approved (IM channel auto-approve mode).
    pub auto_approve_tools: bool,
    /// Whether provider loops should re-read global reasoning effort mid-turn.
    pub follow_global_reasoning_effort: bool,
    /// Whether to schedule title/memory/skill-review follow-ups after success.
    pub post_turn_effects: bool,
    /// Whether a caller-triggered cancel should discard the partial response and
    /// return an error to the caller instead of persisting a final assistant row.
    pub abort_on_cancel: bool,
    /// Whether run_chat_engine should persist its own final error event.
    pub persist_final_error_event: bool,

    /// Which caller opened this stream. Drives the `activeChatCounts`
    /// breakdown surfaced in `/api/server/status`.
    pub source: ChatSource,

    // Output
    pub event_sink: Arc<dyn EventSink>,
}

/// Result returned by the chat engine.
pub struct ChatEngineResult {
    pub response: String,
    /// The model that produced the successful response.
    pub model_used: Option<ActiveModel>,
    /// The agent instance after chat (for UI chat to update State).
    pub agent: Option<AssistantAgent>,
}
