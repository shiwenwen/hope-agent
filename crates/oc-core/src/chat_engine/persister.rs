//! Stream-event persistence shared by the chat engine and the subagent /
//! async-job injection path. Turns the provider's `on_delta` event stream
//! into DB rows: `thinking_block` / `text_block` between tool calls,
//! `tool` (+ later `tool_result` update) per invocation, and captured
//! usage/model/ttft for the final `assistant` row.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::session::{NewMessage, SessionDB};

use super::types::CapturedUsage;

/// Owns the pending text / thinking buffers and the captured-usage cell,
/// produces an `on_delta` persistence closure, and exposes helpers for
/// end-of-turn flushing and assembling the final assistant row.
#[derive(Default)]
pub(crate) struct StreamPersister {
    pending_text: Arc<Mutex<String>>,
    pending_thinking: Arc<Mutex<String>>,
    thinking_start_time: Arc<Mutex<Option<Instant>>>,
    had_thinking_blocks: Arc<AtomicBool>,
    captured_usage: Arc<Mutex<CapturedUsage>>,
}

impl StreamPersister {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn had_thinking_blocks(&self) -> bool {
        self.had_thinking_blocks.load(Ordering::SeqCst)
    }

    pub(crate) fn usage(&self) -> CapturedUsage {
        self.captured_usage
            .lock()
            .map(|u| u.clone())
            .unwrap_or_default()
    }

    /// Build the persistence `on_delta` callback. The returned closure is
    /// `Fn + Send + 'static` so it fits `AssistantAgent::chat`. It does
    /// **not** forward events to any external sink — the caller should
    /// compose it with their own sink-forwarding wrapper.
    pub(crate) fn build_callback(
        &self,
        db: &Arc<SessionDB>,
        session_id: String,
    ) -> impl Fn(&str) + Send + 'static {
        let db = db.clone();
        let pending_text = self.pending_text.clone();
        let pending_thinking = self.pending_thinking.clone();
        let thinking_start_time = self.thinking_start_time.clone();
        let had_thinking_blocks = self.had_thinking_blocks.clone();
        let captured_usage = self.captured_usage.clone();

        // Called hundreds of times per turn; parse `delta` once and
        // dispatch from a single match.
        move |delta: &str| {
            let event = match serde_json::from_str::<serde_json::Value>(delta) {
                Ok(v) => v,
                Err(_) => return,
            };
            match event.get("type").and_then(|t| t.as_str()) {
                Some("usage") => {
                    if let Ok(mut u) = captured_usage.lock() {
                        u.absorb_event(&event);
                    }
                }
                Some("thinking_delta") => {
                    if let Some(text) = event.get("content").and_then(|t| t.as_str()) {
                        if let Ok(mut ts) = thinking_start_time.lock() {
                            if ts.is_none() {
                                *ts = Some(Instant::now());
                            }
                        }
                        if let Ok(mut pk) = pending_thinking.lock() {
                            pk.push_str(text);
                        }
                    }
                }
                Some("text_delta") => {
                    // Invariant: `events::emit_text_delta` uses field "content",
                    // not "text".
                    if let Some(text) = event.get("content").and_then(|t| t.as_str()) {
                        if let Ok(mut pt) = pending_text.lock() {
                            pt.push_str(text);
                        }
                    }
                }
                Some("tool_call") => {
                    if let Ok(mut pk) = pending_thinking.lock() {
                        if !pk.is_empty() {
                            let duration = thinking_start_time
                                .lock()
                                .ok()
                                .and_then(|mut ts| ts.take())
                                .map(|t| t.elapsed().as_millis() as i64);
                            let msg = NewMessage::thinking_block_with_duration(&pk, duration);
                            let _ = db.append_message(&session_id, &msg);
                            pk.clear();
                            had_thinking_blocks.store(true, Ordering::SeqCst);
                        }
                    }
                    if let Ok(mut pt) = pending_text.lock() {
                        if !pt.is_empty() {
                            let msg = NewMessage::text_block(&pt);
                            let _ = db.append_message(&session_id, &msg);
                            pt.clear();
                        }
                    }
                    let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = event.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let arguments = event
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let tool_msg = NewMessage::tool(call_id, name, arguments, "", None, false);
                    let _ = db.append_message(&session_id, &tool_msg);
                }
                Some("tool_result") => {
                    let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let result = event.get("result").and_then(|v| v.as_str()).unwrap_or("");
                    let duration_ms = event.get("duration_ms").and_then(|v| v.as_i64());
                    let is_error = event
                        .get("is_error")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let _ =
                        db.update_tool_result(&session_id, call_id, result, duration_ms, is_error);
                }
                _ => {}
            }
        }
    }

    /// Return and clear the trailing text accumulated after the last
    /// tool_call flush. Used as the final `assistant` row's content so we
    /// don't double-record text already saved as `text_block` rows — the
    /// frontend's `parseSessionMessages` concatenates pending `text_block`
    /// blocks with the assistant row's content, and a full accumulated
    /// text would appear twice (once per round, once as the final).
    pub(crate) fn take_trailing_text(&self) -> String {
        let mut pt = match self.pending_text.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        std::mem::take(&mut *pt)
    }

    /// Flush any remaining thinking buffer at turn end. Run AFTER the
    /// agent.chat() future resolves and BEFORE writing the final
    /// assistant row, so `had_thinking_blocks()` is accurate when the
    /// caller decides whether to duplicate thinking into the assistant
    /// row's `thinking` column.
    pub(crate) fn flush_remaining_thinking(&self, db: &Arc<SessionDB>, session_id: &str) {
        let mut pk = match self.pending_thinking.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if pk.is_empty() {
            return;
        }
        let duration = self
            .thinking_start_time
            .lock()
            .ok()
            .and_then(|mut ts| ts.take())
            .map(|t| t.elapsed().as_millis() as i64);
        let msg = NewMessage::thinking_block_with_duration(&pk, duration);
        let _ = db.append_message(session_id, &msg);
        pk.clear();
        self.had_thinking_blocks.store(true, Ordering::SeqCst);
    }

    /// Build the final assistant `NewMessage` carrying captured usage /
    /// model / ttft. When no `thinking_block` row was written during the
    /// turn, the legacy `thinking` column is populated so the bubble can
    /// still surface the chain-of-thought.
    pub(crate) fn build_assistant_message(
        &self,
        response: &str,
        thinking_from_api: Option<String>,
        duration_ms: u64,
    ) -> NewMessage {
        let mut msg = NewMessage::assistant(response);
        msg.tool_duration_ms = Some(duration_ms as i64);
        if !self.had_thinking_blocks() {
            msg.thinking = thinking_from_api;
        }
        if let Ok(u) = self.captured_usage.lock() {
            msg.tokens_in = u.input_tokens;
            msg.tokens_out = u.output_tokens;
            msg.tokens_in_last = u.last_input_tokens;
            msg.model = u.model.clone();
            msg.ttft_ms = u.ttft_ms;
            msg.tokens_cache_creation = u.cache_creation_input_tokens;
            msg.tokens_cache_read = u.cache_read_input_tokens;
        }
        msg
    }
}
