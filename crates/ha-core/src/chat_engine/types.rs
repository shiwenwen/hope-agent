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

/// Per-round assistant text accumulated by `ChannelStreamSink` so the IM
/// dispatcher can decide between "send only final-round text" and "split per
/// round" delivery (`ImReplyMode`). Without this split the dispatcher receives
/// the merged `collected_text` (round 0 narration + round 1 final answer
/// concatenated) and shoves it into one IM bubble — looking like two glued
/// sentences, see commit message for the fix context.
#[derive(Debug, Default)]
pub struct RoundTextAccumulator {
    /// Pre-final rounds' text. Each entry is the assistant text accumulated
    /// before the round's tool_call(s); empty rounds (consecutive tool_calls
    /// with no narration) are dropped by the dispatcher.
    pub completed: Vec<String>,
    /// In-flight round's text. After `run_chat_engine` returns this holds the
    /// final-round text (post last tool_result, before stream end).
    pub current: String,
}

/// EventSink for IM channel worker — pushes streaming events via the global EventBus
/// AND forwards them to a background task for progressive Telegram message editing.
///
/// Also accumulates any `media_items[]` emitted in `tool_result` events into
/// `pending_media`, and per-round assistant text into `round_texts`, both of
/// which the dispatcher drains after the chat engine finishes.
pub struct ChannelStreamSink {
    pub session_id: String,
    /// Forwards raw events to the channel streaming background task.
    pub event_tx: tokio::sync::mpsc::Sender<String>,
    /// Accumulates media items (from `send_attachment`, `image_generate`, ...)
    /// so the dispatcher can deliver them through the channel after the turn
    /// completes. The dispatcher owns the same `Arc` and drains this vec once
    /// `run_chat_engine` returns.
    pub pending_media: Arc<Mutex<Vec<MediaItem>>>,
    /// Round-by-round assistant text, see [`RoundTextAccumulator`].
    pub round_texts: Arc<Mutex<RoundTextAccumulator>>,
}

impl ChannelStreamSink {
    pub fn new(
        session_id: String,
        event_tx: tokio::sync::mpsc::Sender<String>,
        pending_media: Arc<Mutex<Vec<MediaItem>>>,
        round_texts: Arc<Mutex<RoundTextAccumulator>>,
    ) -> Self {
        Self {
            session_id,
            event_tx,
            pending_media,
            round_texts,
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
        // Cheap short-circuits: avoid a full JSON parse on every frame.
        // serde_json's default Map is BTreeMap so keys serialize alphabetically;
        // anchoring on `{"type":...` would never fire.
        if event.contains("\"media_items\"") && event.contains("\"type\":\"tool_result\"") {
            // `media_items` first — only send_attachment / image_generate populate it.
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
        } else if event.contains("\"type\":\"text_delta\"") {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                if let Some(text) = val
                    .get("content")
                    .or_else(|| val.get("text"))
                    .and_then(|v| v.as_str())
                {
                    if let Ok(mut acc) = self.round_texts.lock() {
                        acc.current.push_str(text);
                    }
                }
            }
        } else if event.contains("\"type\":\"tool_call\"") {
            // Round boundary: flush in-flight text. Multiple tool_calls in the
            // same round will push empty strings on subsequent calls, which the
            // dispatcher filters before split-mode delivery.
            if let Ok(mut acc) = self.round_texts.lock() {
                let text = std::mem::take(&mut acc.current);
                acc.completed.push(text);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{MediaItem, MediaKind};
    use serde_json::json;

    fn mk_media_item() -> MediaItem {
        MediaItem {
            url: "/attachments/x/avatar.png".into(),
            local_path: Some("/tmp/avatar.png".into()),
            name: "avatar.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 100,
            kind: MediaKind::Image,
            caption: None,
        }
    }

    fn mk_sink() -> (
        ChannelStreamSink,
        Arc<Mutex<Vec<MediaItem>>>,
        Arc<Mutex<RoundTextAccumulator>>,
    ) {
        let pending = Arc::new(Mutex::new(Vec::<MediaItem>::new()));
        let rounds = Arc::new(Mutex::new(RoundTextAccumulator::default()));
        let (tx, _rx) = tokio::sync::mpsc::channel::<String>(64);
        let sink = ChannelStreamSink::new("sess-1".into(), tx, pending.clone(), rounds.clone());
        (sink, pending, rounds)
    }

    fn emit(sink: &ChannelStreamSink, value: serde_json::Value) {
        sink.send(&serde_json::to_string(&value).unwrap());
    }

    #[test]
    fn channel_sink_collects_media_items_from_tool_result() {
        let (sink, pending, _) = mk_sink();
        let item = mk_media_item();
        let event = serde_json::to_string(&json!({
            "type": "tool_result",
            "call_id": "call_1",
            "name": "send_attachment",
            "result": "Sent attachment ...",
            "duration_ms": 2u64,
            "is_error": false,
            "media_items": [item],
        }))
        .unwrap();
        // The bug this guards against: BTreeMap key sort puts `type` mid-string,
        // so an anchored `starts_with("{\"type\"...")` guard never fires.
        assert!(
            !event.starts_with("{\"type\""),
            "if this fires the BTreeMap assumption changed; review sink guard: {event}"
        );

        sink.send(&event);
        let collected = pending.lock().unwrap();
        assert_eq!(collected.len(), 1, "media_items not collected: {event}");
        assert_eq!(collected[0].name, "avatar.png");
    }

    #[test]
    fn channel_sink_ignores_non_tool_result_events() {
        let (sink, pending, _) = mk_sink();
        emit(&sink, json!({"type": "text_delta", "content": "hello"}));
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn channel_sink_splits_round_texts_at_tool_call() {
        let (sink, _, rounds) = mk_sink();
        // Round 0: narration → tool_call (flush) → tool_result.
        emit(&sink, json!({"type": "text_delta", "content": "我把头像"}));
        emit(&sink, json!({"type": "text_delta", "content": "发给你。"}));
        emit(
            &sink,
            json!({
                "type": "tool_call",
                "call_id": "c1",
                "name": "send_attachment",
                "arguments": "{}",
            }),
        );
        emit(
            &sink,
            json!({
                "type": "tool_result",
                "call_id": "c1",
                "name": "send_attachment",
                "result": "ok",
                "duration_ms": 1u64,
                "is_error": false,
            }),
        );
        // Round 1: final answer (no further tool_call → stays in `current`).
        emit(&sink, json!({"type": "text_delta", "content": "已发。"}));

        let acc = rounds.lock().unwrap();
        assert_eq!(acc.completed, vec!["我把头像发给你。".to_string()]);
        assert_eq!(acc.current, "已发。");
    }

    #[test]
    fn channel_sink_drops_empty_rounds_for_back_to_back_tool_calls() {
        // Multiple tool_calls in the same round (or zero-narration rounds) push
        // empty strings; the dispatcher filters these before split-mode send.
        let (sink, _, rounds) = mk_sink();
        emit(&sink, json!({"type": "text_delta", "content": "thinking"}));
        emit(
            &sink,
            json!({"type": "tool_call", "call_id": "c1", "name": "x", "arguments": "{}"}),
        );
        emit(
            &sink,
            json!({"type": "tool_call", "call_id": "c2", "name": "y", "arguments": "{}"}),
        );
        let acc = rounds.lock().unwrap();
        assert_eq!(acc.completed, vec!["thinking".to_string(), String::new()]);
    }
}
