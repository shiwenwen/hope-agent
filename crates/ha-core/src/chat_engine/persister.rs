//! Stream-event persistence shared by the chat engine and the subagent /
//! async-job injection path.
//!
//! Crash-resilient model: `text_delta` / `thinking_delta` insert a
//! placeholder row (`stream_status = 'streaming'`) on the first delta, then
//! a throttled UPDATE (every 500ms or 1KB) syncs the in-memory buffer into
//! the row's `content`. The placeholder finalizes to `'completed'` at the
//! next `tool_call` boundary or at turn end. SIGKILL mid-stream leaves a
//! `streaming` row that startup sweep promotes to `orphaned`, instead of
//! losing the whole segment.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::session::{MessageRole, NewMessage, SessionDB};

use super::stream_seq::ChatSource;
use super::types::CapturedUsage;

const FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const FLUSH_BYTES: usize = 1024;

/// Lock a `Mutex` for a poison-tolerant write. A poisoned lock means a
/// previous holder panicked while mutating; the buffer is still readable
/// and we'd rather keep the partial content than lose it.
fn lock_or_poisoned<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Owns the pending text / thinking buffers, the captured-usage cell, and
/// the in-flight streaming placeholder slot.
pub(crate) struct StreamPersister {
    db: Arc<SessionDB>,
    session_id: String,
    /// `ChatSource` of the run that owns this persister. Tagged onto every
    /// row this persister appends so `messages.source` reflects the caller
    /// even for streaming placeholder / tool / final assistant rows.
    source: ChatSource,
    pending_text: Mutex<String>,
    pending_thinking: Mutex<String>,
    thinking_start_time: Mutex<Option<Instant>>,
    had_thinking_blocks: AtomicBool,
    captured_usage: Mutex<CapturedUsage>,
    /// Single slot: `thinking_delta`/`text_delta` don't interleave within
    /// a round in practice, and a role switch finalizes the old placeholder
    /// before opening a new one.
    streaming_id: Mutex<Option<i64>>,
    streaming_role: Mutex<Option<MessageRole>>,
    last_flush: Mutex<Instant>,
    bytes_since_flush: AtomicUsize,
}

impl StreamPersister {
    /// Construct a registered persister. The returned `Arc` is also held
    /// (weakly) by [`super::active_persisters`] so a panic / signal hook
    /// can finalize any in-flight placeholder before the process exits.
    pub(crate) fn new(db: Arc<SessionDB>, session_id: String, source: ChatSource) -> Arc<Self> {
        let me = Arc::new(Self {
            db,
            session_id,
            source,
            pending_text: Mutex::new(String::new()),
            pending_thinking: Mutex::new(String::new()),
            thinking_start_time: Mutex::new(None),
            had_thinking_blocks: AtomicBool::new(false),
            captured_usage: Mutex::new(CapturedUsage::default()),
            streaming_id: Mutex::new(None),
            streaming_role: Mutex::new(None),
            last_flush: Mutex::new(Instant::now()),
            bytes_since_flush: AtomicUsize::new(0),
        });
        super::active_persisters::register(&me);
        me
    }

    pub(crate) fn had_thinking_blocks(&self) -> bool {
        self.had_thinking_blocks.load(Ordering::SeqCst)
    }

    pub(crate) fn usage(&self) -> CapturedUsage {
        lock_or_poisoned(&self.captured_usage).clone()
    }

    fn current_role(&self) -> Option<MessageRole> {
        lock_or_poisoned(&self.streaming_role).clone()
    }

    /// `Fn + Send + 'static` callback for `AssistantAgent::chat`. Does not
    /// forward events to any external sink — the caller composes it with
    /// their own sink-forwarding wrapper.
    pub(crate) fn build_callback(self: &Arc<Self>) -> impl Fn(&str) + Send + 'static {
        let me = Arc::clone(self);

        move |delta: &str| {
            let event = match serde_json::from_str::<serde_json::Value>(delta) {
                Ok(v) => v,
                Err(_) => return,
            };
            match event.get("type").and_then(|t| t.as_str()) {
                Some("usage") => {
                    lock_or_poisoned(&me.captured_usage).absorb_event(&event);
                }
                Some("thinking_delta") => {
                    if let Some(text) = event.get("content").and_then(|t| t.as_str()) {
                        let mut ts = lock_or_poisoned(&me.thinking_start_time);
                        if ts.is_none() {
                            *ts = Some(Instant::now());
                        }
                        drop(ts);
                        me.handle_text_chunk(MessageRole::ThinkingBlock, text);
                    }
                }
                Some("text_delta") => {
                    // `events::emit_text_delta` uses field "content", not "text".
                    if let Some(text) = event.get("content").and_then(|t| t.as_str()) {
                        me.handle_text_chunk(MessageRole::TextBlock, text);
                    }
                }
                Some("tool_call") => {
                    me.finalize_active_placeholder();
                    let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = event.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let arguments = event
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let tool_msg = NewMessage::tool(call_id, name, arguments, "", None, false)
                        .with_source(me.source);
                    let _ = me.db.append_message(&me.session_id, &tool_msg);
                }
                Some("tool_result") => {
                    let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let result = event.get("result").and_then(|v| v.as_str()).unwrap_or("");
                    let duration_ms = event.get("duration_ms").and_then(|v| v.as_i64());
                    let is_error = event
                        .get("is_error")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    // Persist structured tool side-output (file change diff,
                    // line deltas) on the result row so history reload feeds
                    // the diff panel without an extra roundtrip.
                    let metadata_json: Option<String> = event
                        .get("tool_metadata")
                        .filter(|v| !v.is_null())
                        .and_then(|v| serde_json::to_string(v).ok());
                    let _ = me.db.update_tool_result_with_metadata(
                        &me.session_id,
                        call_id,
                        result,
                        duration_ms,
                        is_error,
                        metadata_json.as_deref(),
                    );
                }
                _ => {}
            }
        }
    }

    fn buffer_for(&self, role: MessageRole) -> &Mutex<String> {
        match role {
            MessageRole::ThinkingBlock => &self.pending_thinking,
            _ => &self.pending_text,
        }
    }

    /// Append a streaming chunk and either open / flush / leave the
    /// placeholder alone based on role + throttle thresholds.
    fn handle_text_chunk(&self, role: MessageRole, text: &str) {
        // Role switch: finalize the prior placeholder before opening one
        // for the new role.
        if let Some(prior) = self.current_role() {
            if prior != role {
                self.finalize_active_placeholder();
            }
        }

        let buffer_arc = self.buffer_for(role);
        {
            let mut buf = lock_or_poisoned(buffer_arc);
            buf.push_str(text);
        }

        let need_begin = lock_or_poisoned(&self.streaming_id).is_none();
        if need_begin {
            self.begin_placeholder(role);
            return;
        }

        // Throttle: flush when EITHER 1KB accumulated OR 500ms elapsed.
        let bytes = self
            .bytes_since_flush
            .fetch_add(text.len(), Ordering::SeqCst)
            + text.len();
        let elapsed_ok = lock_or_poisoned(&self.last_flush).elapsed() >= FLUSH_INTERVAL;
        if bytes >= FLUSH_BYTES || elapsed_ok {
            // Snapshot the buffer only when actually flushing — otherwise
            // every per-delta clone in a hot stream would be wasted.
            let snapshot = lock_or_poisoned(buffer_arc).clone();
            self.flush_active_placeholder(&snapshot, "streaming", None);
        }
    }

    /// INSERT a placeholder row carrying the current buffer as initial
    /// content + `stream_status='streaming'`, and record its rowid.
    fn begin_placeholder(&self, role: MessageRole) {
        let buffer_arc = self.buffer_for(role);
        let initial = lock_or_poisoned(buffer_arc).clone();
        let placeholder = match role {
            MessageRole::ThinkingBlock => {
                let duration = lock_or_poisoned(&self.thinking_start_time)
                    .as_ref()
                    .map(|t| t.elapsed().as_millis() as i64);
                let mut msg = NewMessage::thinking_block_with_duration(&initial, duration);
                msg.stream_status = Some("streaming".to_string());
                msg.source = Some(self.source.as_str().to_string());
                msg
            }
            _ => {
                let mut msg = NewMessage::text_block(&initial);
                msg.stream_status = Some("streaming".to_string());
                msg.source = Some(self.source.as_str().to_string());
                msg
            }
        };
        match self.db.append_message(&self.session_id, &placeholder) {
            Ok(id) => {
                *lock_or_poisoned(&self.streaming_id) = Some(id);
                *lock_or_poisoned(&self.streaming_role) = Some(role);
                *lock_or_poisoned(&self.last_flush) = Instant::now();
                self.bytes_since_flush.store(0, Ordering::SeqCst);
                app_debug!(
                    "session",
                    "stream_persist",
                    "begin streaming row id={} session={}",
                    id,
                    self.session_id
                );
            }
            Err(e) => {
                app_warn!(
                    "session",
                    "stream_persist",
                    "begin placeholder failed for {}: {}",
                    self.session_id,
                    e
                );
            }
        }
    }

    fn flush_active_placeholder(&self, content: &str, status: &str, duration_ms: Option<i64>) {
        let id = match *lock_or_poisoned(&self.streaming_id) {
            Some(id) => id,
            None => return,
        };
        if let Err(e) = self
            .db
            .update_message_stream_content(id, content, status, duration_ms)
        {
            app_warn!(
                "session",
                "stream_persist",
                "flush placeholder id={} failed: {}",
                id,
                e
            );
        }
        *lock_or_poisoned(&self.last_flush) = Instant::now();
        self.bytes_since_flush.store(0, Ordering::SeqCst);
    }

    /// Promote the active placeholder to `status` with the final buffer
    /// content, then clear the streaming slot + buffer. `status` is
    /// `"completed"` for normal turn-end / tool boundary finalization,
    /// `"orphaned"` for crash / panic / error paths so startup sweep and
    /// `inject_orphaned_partial_summary` can recognize the row as
    /// interrupted.
    fn finalize_active_placeholder_with_status(&self, status: &str) {
        let role = match self.current_role() {
            Some(r) => r,
            None => return,
        };
        let buffer_arc = self.buffer_for(role);
        let final_content = std::mem::take(&mut *lock_or_poisoned(buffer_arc));
        // Recompute thinking duration at finalize: the placeholder was
        // inserted with a near-zero duration on the first delta, but the
        // real elapsed time only becomes accurate now.
        let duration_override = if matches!(role, MessageRole::ThinkingBlock) {
            lock_or_poisoned(&self.thinking_start_time)
                .as_ref()
                .map(|t| t.elapsed().as_millis() as i64)
        } else {
            None
        };
        self.flush_active_placeholder(&final_content, status, duration_override);
        *lock_or_poisoned(&self.streaming_id) = None;
        *lock_or_poisoned(&self.streaming_role) = None;
        if matches!(role, MessageRole::ThinkingBlock) {
            self.had_thinking_blocks.store(true, Ordering::SeqCst);
            // Reset so the next thinking block measures its own elapsed time.
            *lock_or_poisoned(&self.thinking_start_time) = None;
        }
    }

    fn finalize_active_placeholder(&self) {
        self.finalize_active_placeholder_with_status("completed");
    }

    /// Return any trailing text and clear it. The trailing text feeds the
    /// final `assistant` row's `content` so it stays canonical for FTS
    /// search, default history filtering, and clipboard copy — the
    /// `text_block` placeholder is a transient streaming artifact.
    ///
    /// On the success path we DELETE the placeholder row to avoid
    /// double-rendering (frontend `parseSessionMessages` concatenates
    /// pending `text_block` blocks with the assistant row's content).
    /// On crash / error the placeholder lives on as `streaming` →
    /// startup sweep promotes to `orphaned` and the resume turn surfaces
    /// it via `inject_orphaned_partial_summary`.
    pub(crate) fn take_trailing_text(&self) -> String {
        if matches!(self.current_role(), Some(MessageRole::TextBlock)) {
            let content = std::mem::take(&mut *lock_or_poisoned(&self.pending_text));
            if let Some(id) = lock_or_poisoned(&self.streaming_id).take() {
                if let Err(e) = self.db.delete_message_by_id(id) {
                    app_warn!(
                        "session",
                        "stream_persist",
                        "delete trailing placeholder id={} failed: {}",
                        id,
                        e
                    );
                }
            }
            *lock_or_poisoned(&self.streaming_role) = None;
            return content;
        }
        std::mem::take(&mut *lock_or_poisoned(&self.pending_text))
    }

    /// Flush any remaining thinking buffer at turn end. Run AFTER the
    /// agent.chat() future resolves and BEFORE writing the final assistant
    /// row, so `had_thinking_blocks()` is accurate when the caller decides
    /// whether to duplicate thinking into the assistant row's `thinking`
    /// column.
    pub(crate) fn flush_remaining_thinking(&self) {
        if matches!(self.current_role(), Some(MessageRole::ThinkingBlock)) {
            self.finalize_active_placeholder();
            return;
        }
        // Legacy fallback: text was buffered without an active placeholder
        // (e.g. SubAgent driving the persister differently).
        let mut pk = lock_or_poisoned(&self.pending_thinking);
        if pk.is_empty() {
            return;
        }
        let duration = lock_or_poisoned(&self.thinking_start_time)
            .take()
            .map(|t| t.elapsed().as_millis() as i64);
        let msg = NewMessage::thinking_block_with_duration(&pk, duration).with_source(self.source);
        let _ = self.db.append_message(&self.session_id, &msg);
        pk.clear();
        self.had_thinking_blocks.store(true, Ordering::SeqCst);
    }

    /// Synchronous crash-time flush: promote the active streaming
    /// placeholder (if any) to `orphaned` with whatever buffer content
    /// has accumulated, so startup sweep + `inject_orphaned_partial_summary`
    /// recognize it as interrupted (vs `completed`, which would silently
    /// hide the broken turn). Safe from a panic hook or signal handler —
    /// rusqlite is synchronous, no `await`. Idempotent.
    pub(crate) fn crash_flush(&self) {
        self.finalize_active_placeholder_with_status("orphaned");
    }

    /// Drop the active streaming placeholder without preserving its
    /// content — DELETE the row, clear the slot, drain the buffer.
    /// Used by failover error paths and user cancellation: the partial
    /// from a failed model attempt should not bleed into a successful
    /// retry's bubble or pollute the next turn's restore-summary
    /// injection.
    pub(crate) fn discard_active_placeholder(&self) {
        if let Some(id) = lock_or_poisoned(&self.streaming_id).take() {
            if let Err(e) = self.db.delete_message_by_id(id) {
                app_warn!(
                    "session",
                    "stream_persist",
                    "discard placeholder id={} failed: {}",
                    id,
                    e
                );
            }
        }
        *lock_or_poisoned(&self.streaming_role) = None;
        lock_or_poisoned(&self.pending_text).clear();
        lock_or_poisoned(&self.pending_thinking).clear();
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
        let u = lock_or_poisoned(&self.captured_usage);
        msg.tokens_in = u.input_tokens;
        msg.tokens_out = u.output_tokens;
        msg.tokens_in_last = u.last_input_tokens;
        msg.model = u.model.clone();
        msg.ttft_ms = u.ttft_ms;
        msg.tokens_cache_creation = u.cache_creation_input_tokens;
        msg.tokens_cache_read = u.cache_read_input_tokens;
        msg.source = Some(self.source.as_str().to_string());
        msg
    }
}

/// Last-resort cleanup for paths that didn't take the success route
/// (`take_trailing_text` / `flush_remaining_thinking`) or the explicit
/// crash route (`crash_flush`). Examples: `agent.chat()` returning `Err`,
/// failover swallowing the chat result, `abort_on_cancel` short-circuit.
/// If a streaming placeholder is still alive when the last `Arc` goes
/// away, mark it `orphaned` so it's eligible for the resume-turn summary
/// and doesn't linger as `streaming` until the next process restart.
impl Drop for StreamPersister {
    fn drop(&mut self) {
        if self.current_role().is_some() {
            self.finalize_active_placeholder_with_status("orphaned");
        }
    }
}
