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

/// One LLM round's outbound payload as observed by the IM channel sink:
/// the `text_delta`s the model emitted before its first tool call, plus any
/// media items its `tool_result`s produced. The dispatcher fans these out
/// per `ImReplyMode` after `run_chat_engine` returns.
///
/// A "round" here corresponds to a single `process_round` cycle — the model
/// outputs narration + tool_calls, tools execute, and tool_results stream
/// back. The next `text_delta` after a tool_result starts a new round.
#[derive(Debug, Default, Clone)]
pub struct RoundOutput {
    /// Narration text the model emitted **before** the round's first tool
    /// call (or, for the final round, the entire post-last-tool reply).
    pub text: String,
    /// Media produced by this round's tool_results. Order matches
    /// tool_result arrival.
    pub medias: Vec<MediaItem>,
}

impl RoundOutput {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.medias.is_empty()
    }
}

/// Round-aware accumulator owned by `ChannelStreamSink`. Watches the event
/// stream and groups it into per-round buckets via a tiny state machine:
///
/// - `text_delta` → append to `current.text`. If we just left a tool phase,
///   roll `current` into `completed` first (new round starts).
/// - `tool_call` → mark "in tool phase" (the first one ends the round's
///   narration; subsequent calls in the same round are no-ops for grouping).
/// - `tool_result` w/ `media_items` → append to whichever round just
///   transitioned to tool phase (the freshest entry in `completed`, or
///   `current` as a defensive fallback for malformed streams).
///
/// After the engine returns, `current` either holds the final round's text
/// (model finished with narration) or is empty (model ended on a tool).
/// The dispatcher iterates `completed` then optionally `current`.
#[derive(Debug, Default)]
pub struct RoundTextAccumulator {
    /// Rounds the state machine has already closed (a tool_call arrived).
    pub completed: Vec<RoundOutput>,
    /// In-flight round's text/media. Promoted to `completed` when the next
    /// tool_call arrives; left as the trailing entry on stream end.
    pub current: RoundOutput,
    /// True between the round's first tool_call and the next text_delta.
    /// Used to detect round-boundary transitions cheaply on each event.
    in_tool_phase: bool,
    /// True between the first `thinking_delta` of a round and either the
    /// first `text_delta` (which closes the blockquote with `\n\n`) or the
    /// round's `tool_call` (which resets state for the next round). Only
    /// touched when `ChannelStreamSink::show_thinking` is enabled — when
    /// disabled, `on_thinking` is never called.
    thinking_active: bool,
}

/// Markdown blockquote opener prepended to the first thinking chunk of a
/// round when the channel account has `show_thinking = true`. Subsequent
/// chunks reuse the trailing `> ` from the previous append; embedded
/// newlines in chunks are rewritten to `\n> ` to keep the blockquote
/// uninterrupted across multi-line reasoning.
const THINKING_BLOCKQUOTE_OPENER: &str = "> 💭 **Thinking**\n> ";

impl RoundTextAccumulator {
    /// Append text to the current round. If we just exited a tool phase,
    /// flip the flag so the new text starts a fresh round — no need to push
    /// a placeholder, since the closing of the previous round already
    /// happened in `on_tool_call` (which pushed its narration to
    /// `completed`). `current` was reset there too.
    ///
    /// Returns `true` when this call closed a thinking blockquote (pushed
    /// the trailing `\n\n` separator). The IM channel sink mirrors that
    /// separator into `event_tx` so the streaming preview task's running
    /// `accumulated` buffer stays in sync with `current.text` —
    /// otherwise split-streaming finalize would render a preview where
    /// the answer text bleeds straight into the blockquote.
    fn on_text(&mut self, text: &str) -> bool {
        if self.in_tool_phase {
            self.in_tool_phase = false;
        }
        let closed_thinking = if self.thinking_active {
            self.current.text.push_str("\n\n");
            self.thinking_active = false;
            true
        } else {
            false
        };
        self.current.text.push_str(text);
        closed_thinking
    }

    /// Append a thinking-delta chunk to the current round, formatted as a
    /// markdown blockquote. Only invoked when `ChannelStreamSink::show_thinking`
    /// is enabled — disabled mode drops thinking entirely without touching the
    /// accumulator. Returns the exact slice appended to `current.text`, so
    /// `ChannelStreamSink` can synthesize a matching `text_delta` event for
    /// the streaming preview task to render.
    pub fn on_thinking(&mut self, text: &str) -> String {
        if self.in_tool_phase {
            self.in_tool_phase = false;
        }
        let mut appended = String::new();
        if !self.thinking_active {
            appended.push_str(THINKING_BLOCKQUOTE_OPENER);
            self.thinking_active = true;
        }
        // Keep multi-line reasoning inside the blockquote.
        if text.contains('\n') {
            appended.push_str(&text.replace('\n', "\n> "));
        } else {
            appended.push_str(text);
        }
        self.current.text.push_str(&appended);
        appended
    }

    /// Mark the round as having entered its tool phase. Idempotent within
    /// the same round — only the *first* tool_call rolls `current` into
    /// `completed`; subsequent tool_calls in the same round are no-ops for
    /// grouping (their results still attach correctly via `on_media`).
    ///
    /// Returns `true` when this call closed a thinking blockquote on the
    /// outgoing round (same contract as [`Self::on_text`] — sink must
    /// synthesize the matching `\n\n` text_delta into the preview event
    /// stream so per-round finalize doesn't ship a preview where the
    /// blockquote eats the round boundary).
    fn on_tool_call(&mut self) -> bool {
        if self.in_tool_phase {
            return false;
        }
        let closed_thinking = if self.thinking_active {
            self.current.text.push_str("\n\n");
            self.thinking_active = false;
            true
        } else {
            false
        };
        let prev = std::mem::take(&mut self.current);
        self.completed.push(prev);
        self.in_tool_phase = true;
        closed_thinking
    }

    /// Attach media to the round currently in the tool phase. Falls back to
    /// `current` defensively if the stream gave us a tool_result without a
    /// preceding tool_call (shouldn't happen, but won't lose data).
    fn on_media(&mut self, items: Vec<MediaItem>) {
        let target = if self.in_tool_phase {
            self.completed.last_mut()
        } else {
            None
        };
        if let Some(round) = target {
            round.medias.extend(items);
        } else {
            self.current.medias.extend(items);
        }
    }

    /// Snapshot of a closed round's media. Used by the IM stream task
    /// under split-streaming to deliver a round's media inline at the
    /// round boundary. The dispatcher still gets the full round (medias
    /// included) at end-of-turn drain — but only consults pre-finalized
    /// rounds for log metrics, so double-delivery isn't a concern.
    pub fn round_medias(&self, idx: usize) -> Vec<MediaItem> {
        self.completed
            .get(idx)
            .map(|round| round.medias.clone())
            .unwrap_or_default()
    }

    /// Drain the accumulator into a flat sequence of rounds in time order.
    /// The trailing entry is the "final round" when `current` is non-empty;
    /// otherwise the last `completed` entry is the final round.
    pub fn drain(&mut self) -> Vec<RoundOutput> {
        let mut out = std::mem::take(&mut self.completed);
        let last = std::mem::take(&mut self.current);
        if !last.is_empty() {
            out.push(last);
        }
        self.in_tool_phase = false;
        self.thinking_active = false;
        out
    }
}

/// EventSink for IM channel worker — pushes streaming events via the global EventBus
/// AND forwards them to a background task for progressive preview rendering.
///
/// Also feeds the round-aware accumulator (`round_texts`) so the dispatcher
/// can fan out narration and media in time order after the engine returns.
pub struct ChannelStreamSink {
    pub session_id: String,
    /// Forwards raw events to the channel streaming background task.
    pub event_tx: tokio::sync::mpsc::Sender<String>,
    /// Round-by-round text + media, see [`RoundTextAccumulator`].
    pub round_texts: Arc<Mutex<RoundTextAccumulator>>,
    /// Per-account `/reason` state. When `false` (default), `thinking_delta`
    /// events are dropped from the IM path entirely (the EventBus broadcast
    /// still goes out so the desktop UI mirroring the channel session keeps
    /// rendering the thinking block). When `true`, thinking is accumulated
    /// as a markdown blockquote and forwarded to the streaming preview task
    /// as a synthesized `text_delta`.
    pub show_thinking: bool,
    /// Whether this sink is the **primary** attach for the session. Only the
    /// primary forwards events into `event_tx` (and from there to the IM
    /// channel preview / final reply); secondary observers still emit on the
    /// EventBus so attached UIs can render the stream, but stay silent on
    /// the channel side.
    ///
    /// Wrapped in `Arc<AtomicBool>` so the dispatcher can clone-and-hold a
    /// reference to flip gating mid-turn without rebuilding the sink (e.g.
    /// when a primary swap fires from `/session exit`).
    pub is_primary: Arc<AtomicBool>,
}

impl ChannelStreamSink {
    /// Build a sink in primary mode — the common case (IM-driven turn,
    /// or any caller that wants events forwarded to the IM channel).
    pub fn new(
        session_id: String,
        event_tx: tokio::sync::mpsc::Sender<String>,
        round_texts: Arc<Mutex<RoundTextAccumulator>>,
        show_thinking: bool,
    ) -> Self {
        Self::with_primary(session_id, event_tx, round_texts, show_thinking, true)
    }

    /// Build a sink with an explicit `is_primary` flag — used by
    /// `chat_engine::im_mirror` when attaching observer sinks for
    /// non-primary attaches (the IM channel side stays silent).
    pub fn with_primary(
        session_id: String,
        event_tx: tokio::sync::mpsc::Sender<String>,
        round_texts: Arc<Mutex<RoundTextAccumulator>>,
        show_thinking: bool,
        is_primary: bool,
    ) -> Self {
        Self {
            session_id,
            event_tx,
            round_texts,
            show_thinking,
            is_primary: Arc::new(AtomicBool::new(is_primary)),
        }
    }

    /// Push a synthesized `text_delta` carrying the `\n\n` blockquote
    /// closer that `RoundTextAccumulator` just appended to `current.text`.
    /// Forwarded to `event_tx` ahead of the originating event (text_delta
    /// or tool_call) so the streaming preview task's `accumulated`
    /// stays byte-for-byte aligned with `current.text`.
    fn forward_thinking_close_separator(&self) {
        let synth = serde_json::json!({
            "type": "text_delta",
            "content": "\n\n",
        })
        .to_string();
        let _ = self.event_tx.try_send(synth);
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
        // anchoring on `{"type":...` (which lands mid-string) would never fire.
        // Discriminator order is rarer-needle-first.
        if event.contains("\"media_items\"") && event.contains("\"type\":\"tool_result\"") {
            // tool_result with media: parse to extract MediaItems and route
            // to the round currently in tool phase.
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                if let Some(arr) = val.get("media_items").and_then(|v| v.as_array()) {
                    let items: Vec<MediaItem> = arr
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    if !items.is_empty() {
                        if let Ok(mut acc) = self.round_texts.lock() {
                            acc.on_media(items);
                        }
                    }
                }
            }
        } else if event.contains("\"type\":\"text_delta\"") {
            let mut closed_thinking = false;
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                if let Some(text) = val
                    .get("content")
                    .or_else(|| val.get("text"))
                    .and_then(|v| v.as_str())
                {
                    if let Ok(mut acc) = self.round_texts.lock() {
                        closed_thinking = acc.on_text(text);
                    }
                }
            }
            if closed_thinking {
                self.forward_thinking_close_separator();
            }
        } else if event.contains("\"type\":\"tool_call\"") {
            let closed_thinking = if let Ok(mut acc) = self.round_texts.lock() {
                acc.on_tool_call()
            } else {
                false
            };
            if closed_thinking {
                self.forward_thinking_close_separator();
            }
        } else if event.contains("\"type\":\"thinking_delta\"") {
            // EventBus already broadcast the original event for desktop UI
            // mirroring. On the IM path we either fold the chunk into the
            // round accumulator (and forward a synthesized text_delta to
            // the streaming preview task so its existing `extract_text_delta`
            // logic renders the blockquote) or drop it entirely. Either
            // way we skip the raw event_tx forward at the bottom — the
            // preview task only knows how to render text_delta.
            if self.show_thinking {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                    if let Some(text) = val
                        .get("content")
                        .or_else(|| val.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        let appended = match self.round_texts.lock() {
                            Ok(mut acc) => acc.on_thinking(text),
                            Err(_) => String::new(),
                        };
                        if !appended.is_empty() {
                            let synthesized = serde_json::json!({
                                "type": "text_delta",
                                "content": appended,
                            })
                            .to_string();
                            let _ = self.event_tx.try_send(synthesized);
                        }
                    }
                }
            }
            return;
        }
        // Only the primary attach forwards events into the IM streaming
        // task. Secondary observers still get the EventBus emit above so
        // attached UIs render the stream — they just don't echo it back to
        // the IM channel.
        if self.is_primary.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = self.event_tx.try_send(event.to_string());
        }
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

    fn mk_sink() -> (ChannelStreamSink, Arc<Mutex<RoundTextAccumulator>>) {
        let rounds = Arc::new(Mutex::new(RoundTextAccumulator::default()));
        let (tx, _rx) = tokio::sync::mpsc::channel::<String>(64);
        let sink = ChannelStreamSink::new("sess-1".into(), tx, rounds.clone(), false);
        (sink, rounds)
    }

    /// Variant that surfaces the receiver so `/reason on` tests can verify
    /// the synthesized `text_delta` events forwarded to the streaming
    /// preview task.
    fn mk_sink_with_rx(
        show_thinking: bool,
    ) -> (
        ChannelStreamSink,
        Arc<Mutex<RoundTextAccumulator>>,
        tokio::sync::mpsc::Receiver<String>,
    ) {
        let rounds = Arc::new(Mutex::new(RoundTextAccumulator::default()));
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
        let sink = ChannelStreamSink::new("sess-1".into(), tx, rounds.clone(), show_thinking);
        (sink, rounds, rx)
    }

    fn emit(sink: &ChannelStreamSink, value: serde_json::Value) {
        sink.send(&serde_json::to_string(&value).unwrap());
    }

    fn tool_result_with_media(call_id: &str, items: Vec<MediaItem>) -> serde_json::Value {
        json!({
            "type": "tool_result",
            "call_id": call_id,
            "name": "send_attachment",
            "result": "ok",
            "duration_ms": 1u64,
            "is_error": false,
            "media_items": items,
        })
    }

    fn tool_call(call_id: &str) -> serde_json::Value {
        json!({
            "type": "tool_call",
            "call_id": call_id,
            "name": "send_attachment",
            "arguments": "{}",
        })
    }

    /// `serde_json` defaults to `BTreeMap` for object keys (no `preserve_order`
    /// in this workspace), so emitted JSON serializes keys alphabetically and
    /// `type` lands mid-string. Sink discriminators must use `contains`, not
    /// `starts_with` — otherwise every media item is silently dropped.
    #[test]
    fn emitted_event_keys_serialize_alphabetically() {
        let event =
            serde_json::to_string(&tool_result_with_media("c1", vec![mk_media_item()])).unwrap();
        assert!(
            !event.starts_with("{\"type\""),
            "if this fires the BTreeMap assumption changed; review sink guards: {event}"
        );
    }

    #[test]
    fn channel_sink_collects_media_into_active_round() {
        let (sink, rounds) = mk_sink();
        // Round 0: narration → tool_call → tool_result(media).
        emit(&sink, json!({"type": "text_delta", "content": "我把头像"}));
        emit(&sink, json!({"type": "text_delta", "content": "发给你。"}));
        emit(&sink, tool_call("c1"));
        emit(&sink, tool_result_with_media("c1", vec![mk_media_item()]));
        // Round 1: final narration.
        emit(&sink, json!({"type": "text_delta", "content": "已发。"}));

        let mut acc = rounds.lock().unwrap();
        let drained = acc.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].text, "我把头像发给你。");
        assert_eq!(drained[0].medias.len(), 1);
        assert_eq!(drained[0].medias[0].name, "avatar.png");
        assert_eq!(drained[1].text, "已发。");
        assert!(drained[1].medias.is_empty());
    }

    #[test]
    fn channel_sink_groups_multiple_tool_calls_into_same_round() {
        // A single LLM round can dispatch multiple tools; their text/media
        // all belong to *one* round. Idempotent on_tool_call ensures the
        // round only closes once.
        let (sink, rounds) = mk_sink();
        emit(
            &sink,
            json!({"type": "text_delta", "content": "doing both"}),
        );
        emit(&sink, tool_call("c1"));
        emit(&sink, tool_result_with_media("c1", vec![mk_media_item()]));
        emit(&sink, tool_call("c2")); // same round, no new boundary
        let mut item2 = mk_media_item();
        item2.name = "second.png".into();
        emit(&sink, tool_result_with_media("c2", vec![item2]));

        let mut acc = rounds.lock().unwrap();
        let drained = acc.drain();
        assert_eq!(drained.len(), 1, "two tool_calls should not split rounds");
        assert_eq!(drained[0].text, "doing both");
        assert_eq!(
            drained[0]
                .medias
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            vec!["avatar.png", "second.png"]
        );
    }

    #[test]
    fn channel_sink_ignores_non_relevant_events() {
        // text_delta is consumed (round 0 narration) but no tool boundary
        // exists yet, so drain returns one trailing round.
        let (sink, rounds) = mk_sink();
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "ignored"}),
        );
        emit(&sink, json!({"type": "text_delta", "content": "hi"}));
        let drained = rounds.lock().unwrap().drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].text, "hi");
        assert!(drained[0].medias.is_empty());
    }

    #[test]
    fn show_thinking_off_drops_thinking_and_skips_event_forward() {
        // Default behavior preserved: thinking is silently ignored, and
        // the sink does NOT forward the thinking event to the preview
        // task either (so the streaming preview never sees reasoning).
        let (sink, rounds, mut rx) = mk_sink_with_rx(false);
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "ponder"}),
        );
        emit(&sink, json!({"type": "text_delta", "content": "hi"}));

        // event_tx received only the text_delta; thinking_delta was dropped.
        let mut forwarded: Vec<String> = Vec::new();
        while let Ok(s) = rx.try_recv() {
            forwarded.push(s);
        }
        assert_eq!(forwarded.len(), 1);
        assert!(forwarded[0].contains("\"type\":\"text_delta\""));
        assert!(forwarded[0].contains("hi"));
        // No thinking content leaked into the round text.
        let drained = rounds.lock().unwrap().drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].text, "hi");
    }

    #[test]
    fn show_thinking_on_wraps_thinking_in_blockquote_and_forwards_synthetic_text() {
        let (sink, rounds, mut rx) = mk_sink_with_rx(true);
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "let me\nthink"}),
        );
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": " more."}),
        );
        emit(&sink, json!({"type": "text_delta", "content": "Answer."}));

        // Round text: blockquote opener, multi-line reasoning quoted, then
        // a separator and the reply.
        let drained = rounds.lock().unwrap().drain();
        assert_eq!(drained.len(), 1);
        let expected = "> 💭 **Thinking**\n> let me\n> think more.\n\nAnswer.";
        assert_eq!(drained[0].text, expected);

        // Preview task receives: 2 synthesized text_delta events (one per
        // thinking chunk) + 1 synthesized `\n\n` blockquote-close
        // separator (emitted when on_text closed the blockquote) + 1 real
        // text_delta for "Answer." = 4 total. The separator is what keeps
        // split-streaming finalize from gluing the answer onto the quote.
        // No raw thinking_delta forwarded.
        let mut forwarded: Vec<String> = Vec::new();
        while let Ok(s) = rx.try_recv() {
            forwarded.push(s);
        }
        assert_eq!(forwarded.len(), 4);
        for f in &forwarded {
            assert!(f.contains("\"type\":\"text_delta\""));
            assert!(!f.contains("thinking_delta"));
        }
        // The third forward must be the `\n\n` separator and arrive
        // BEFORE the real "Answer." text_delta (so the preview task's
        // accumulated buffer mirrors `current.text` byte-for-byte).
        assert!(forwarded[2].contains("\"content\":\"\\n\\n\""));
        assert!(forwarded[3].contains("Answer."));
    }

    #[test]
    fn closing_thinking_via_text_delta_forwards_separator_to_preview_task() {
        // Regression: the IM split-streaming preview accumulator builds
        // its buffer from forwarded text_delta events. If the accumulator
        // closes the thinking blockquote with `\n\n` but the sink doesn't
        // mirror that into event_tx, the preview message will look like
        // `> 💭 ThinkingAnswer.` (quote bleeds into reply text).
        let (sink, _rounds, mut rx) = mk_sink_with_rx(true);
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "step 1"}),
        );
        emit(&sink, json!({"type": "text_delta", "content": "Answer."}));

        let forwarded: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        // Order: thinking synth → \n\n separator → real text_delta.
        assert_eq!(forwarded.len(), 3);
        assert!(forwarded[0].contains("Thinking"));
        assert_eq!(forwarded[1], r#"{"content":"\n\n","type":"text_delta"}"#);
        assert!(forwarded[2].contains("Answer."));
    }

    #[test]
    fn closing_thinking_via_tool_call_forwards_separator_to_preview_task() {
        // Same regression as above but for the tool_call branch — split
        // streaming relies on the tool_call event arriving AFTER the
        // separator so finalize_split_round renders the round text
        // including the trailing `\n\n` (matching `current.text`).
        let (sink, _rounds, mut rx) = mk_sink_with_rx(true);
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "step 1"}),
        );
        emit(&sink, tool_call("c1"));

        let forwarded: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(forwarded.len(), 3);
        assert!(forwarded[0].contains("Thinking"));
        assert_eq!(forwarded[1], r#"{"content":"\n\n","type":"text_delta"}"#);
        assert!(forwarded[2].contains("\"type\":\"tool_call\""));
    }

    #[test]
    fn show_thinking_resets_across_tool_call_boundary() {
        // Round 0 has reasoning + tool_call. Round 1's reasoning should
        // get its own opener — `thinking_active` must reset on tool_call.
        let (sink, rounds, _rx) = mk_sink_with_rx(true);
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "step 1"}),
        );
        emit(&sink, tool_call("c1"));
        emit(&sink, tool_result_with_media("c1", vec![mk_media_item()]));
        emit(
            &sink,
            json!({"type": "thinking_delta", "content": "step 2"}),
        );
        emit(&sink, json!({"type": "text_delta", "content": "Done."}));

        let drained = rounds.lock().unwrap().drain();
        assert_eq!(drained.len(), 2);
        // Round 0 closed with the trailing `\n\n` (tool_call closes the
        // blockquote) before the round was rolled into `completed`.
        assert_eq!(drained[0].text, "> 💭 **Thinking**\n> step 1\n\n");
        assert_eq!(drained[0].medias.len(), 1);
        // Round 1: fresh opener, no leftover state.
        assert_eq!(drained[1].text, "> 💭 **Thinking**\n> step 2\n\nDone.");
    }

    #[test]
    fn channel_sink_handles_zero_narration_round() {
        // Model went straight to a tool_call without any narration. The
        // round should still appear in `completed` (with empty text + the
        // tool's media), so the dispatcher can deliver the media in time
        // order. Final round provides the closing narration.
        let (sink, rounds) = mk_sink();
        emit(&sink, tool_call("c1"));
        emit(&sink, tool_result_with_media("c1", vec![mk_media_item()]));
        emit(&sink, json!({"type": "text_delta", "content": "done"}));

        let drained = rounds.lock().unwrap().drain();
        assert_eq!(drained.len(), 2);
        assert!(drained[0].text.is_empty());
        assert_eq!(drained[0].medias.len(), 1);
        assert_eq!(drained[1].text, "done");
    }
}
