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

use crate::session::{MessageRole, NewMessage, SessionDB, SessionMessage};

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
    /// Partial rows created by this streaming attempt. Failed attempts are
    /// kept only when the whole failover chain fails; retry/fallback paths
    /// delete these rows so partial output from a failed model cannot be
    /// claimed by the next successful model. Event rows are intentionally
    /// not tracked here because they are real timeline events, not partial
    /// assistant content.
    owned_partial_message_ids: Mutex<Vec<i64>>,
    last_flush: Mutex<Instant>,
    bytes_since_flush: AtomicUsize,
    sealed: AtomicBool,
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
            owned_partial_message_ids: Mutex::new(Vec::new()),
            last_flush: Mutex::new(Instant::now()),
            bytes_since_flush: AtomicUsize::new(0),
            sealed: AtomicBool::new(false),
        });
        super::active_persisters::register(&me);
        me
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
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

    fn record_owned_partial_message_id(&self, id: i64) {
        lock_or_poisoned(&self.owned_partial_message_ids).push(id);
    }

    /// `Fn + Send + 'static` callback for `AssistantAgent::chat`. Does not
    /// forward events to any external sink — the caller composes it with
    /// their own sink-forwarding wrapper.
    pub(crate) fn build_callback(self: &Arc<Self>) -> impl Fn(&str) + Send + 'static {
        let me = Arc::clone(self);

        move |delta: &str| {
            if me.sealed.load(Ordering::SeqCst) {
                return;
            }
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
                    if let Ok(id) = me.db.append_message(&me.session_id, &tool_msg) {
                        me.record_owned_partial_message_id(id);
                    }
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
                    let attachments_meta = event
                        .get("media_items")
                        .and_then(crate::session::build_tool_media_items_attachments_meta);
                    let _ = me.db.update_tool_result_with_side_outputs(
                        &me.session_id,
                        call_id,
                        result,
                        duration_ms,
                        is_error,
                        metadata_json.as_deref(),
                        attachments_meta.as_deref(),
                    );
                }
                Some("round_limit_reached") => {
                    let _ = me.db.append_message(
                        &me.session_id,
                        &NewMessage::event(delta).with_source(me.source),
                    );
                }
                Some("context_compacted") => {
                    // Persist Tier ≥ 2 only — Tier 0/1 reactive micro-compact
                    // fires every turn and would flood the event timeline.
                    // Also skip live-only start markers (`summarizing` for
                    // Tier 3, `emergency_compacting` for Tier 4). The final
                    // event arrives a moment later with real
                    // `messages_affected`, otherwise reload renders two
                    // banners per compaction. Engine-level emergency_compact
                    // persists the final event directly at its callsite.
                    let data = event.get("data");
                    let tier = data
                        .and_then(|d| d.get("tier_applied"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    let description = data
                        .and_then(|d| d.get("description"))
                        .and_then(|d| d.as_str());
                    let is_start_marker =
                        matches!(description, Some("summarizing" | "emergency_compacting"));
                    if tier >= 2 && !is_start_marker {
                        let _ = me.db.append_message(
                            &me.session_id,
                            &NewMessage::event(delta).with_source(me.source),
                        );
                    }
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
                self.record_owned_partial_message_id(id);
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
        if let Ok(id) = self.db.append_message(&self.session_id, &msg) {
            self.record_owned_partial_message_id(id);
        }
        pk.clear();
        self.had_thinking_blocks.store(true, Ordering::SeqCst);
    }

    fn visible_partial_in_messages(messages: &[SessionMessage], latest_user_id: i64) -> bool {
        messages
            .iter()
            .filter(|msg| msg.id > latest_user_id)
            .any(|msg| match msg.role {
                MessageRole::TextBlock | MessageRole::ThinkingBlock => !msg.content.is_empty(),
                MessageRole::Tool => true,
                _ => false,
            })
    }

    fn latest_user_id(messages: &[SessionMessage]) -> i64 {
        messages
            .iter()
            .rev()
            .find(|msg| msg.role == MessageRole::User)
            .map(|msg| msg.id)
            .unwrap_or(0)
    }

    fn has_claimable_partial_rows(&self) -> bool {
        let messages = match self.db.load_session_messages(&self.session_id) {
            Ok(messages) => messages,
            Err(e) => {
                app_warn!(
                    "session",
                    "stream_persist",
                    "load partial rows for failed assistant failed for {}: {}",
                    self.session_id,
                    e
                );
                return false;
            }
        };
        Self::visible_partial_in_messages(&messages, Self::latest_user_id(&messages))
    }

    pub(crate) fn has_visible_partial_output(&self) -> bool {
        if !lock_or_poisoned(&self.pending_text).is_empty()
            || !lock_or_poisoned(&self.pending_thinking).is_empty()
        {
            return true;
        }

        let ids = lock_or_poisoned(&self.owned_partial_message_ids).clone();
        if ids.is_empty() {
            return false;
        }

        let messages = match self.db.load_session_messages(&self.session_id) {
            Ok(messages) => messages,
            Err(e) => {
                app_warn!(
                    "session",
                    "stream_persist",
                    "load visible partial rows for failed assistant failed for {}: {}",
                    self.session_id,
                    e
                );
                return false;
            }
        };
        messages.iter().any(|msg| {
            ids.contains(&msg.id)
                && match msg.role {
                    MessageRole::TextBlock | MessageRole::ThinkingBlock => !msg.content.is_empty(),
                    MessageRole::Tool => true,
                    _ => false,
                }
        })
    }

    /// Persist user-visible partial output when the whole failover chain
    /// eventually fails. Retry/fallback success paths delete tracked attempt
    /// rows so their partial text cannot bleed into a later successful model.
    pub(crate) fn persist_failed_partial_assistant(
        &self,
        thinking_from_api: Option<String>,
        duration_ms: u64,
    ) -> Option<i64> {
        if self.sealed.load(Ordering::SeqCst) {
            return None;
        }
        self.flush_remaining_thinking();
        let trailing_text = self.trailing_text_snapshot();
        let has_partial_rows = self.has_claimable_partial_rows();
        let has_thinking = thinking_from_api
            .as_deref()
            .map(|thinking| !thinking.is_empty())
            .unwrap_or(false);

        if trailing_text.is_empty() && !has_partial_rows && !has_thinking {
            return None;
        }

        let assistant_msg =
            self.build_assistant_message(&trailing_text, thinking_from_api, duration_ms);
        match self.db.append_message(&self.session_id, &assistant_msg) {
            Ok(id) => {
                self.clear_trailing_text_after_assistant_insert();
                Some(id)
            }
            Err(e) => {
                app_warn!(
                    "session",
                    "stream_persist",
                    "persist failed partial assistant for {} failed: {}",
                    self.session_id,
                    e
                );
                None
            }
        }
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

    /// User-stop watchdog flush. Unlike `crash_flush`, this keeps the row in
    /// the normal completed stream state so the cancellation finalize path can
    /// claim it as the current partial, then seals the persister so late model
    /// deltas cannot write after the turn has been marked terminal.
    pub(crate) fn cancel_flush_and_seal(&self) {
        self.finalize_active_placeholder_with_status("completed");
        self.sealed.store(true, Ordering::SeqCst);
    }

    fn trailing_text_snapshot(&self) -> String {
        lock_or_poisoned(&self.pending_text).clone()
    }

    fn clear_trailing_text_after_assistant_insert(&self) {
        if matches!(self.current_role(), Some(MessageRole::TextBlock)) {
            if let Some(id) = lock_or_poisoned(&self.streaming_id).take() {
                if let Err(e) = self.db.delete_message_by_id(id) {
                    app_warn!(
                        "session",
                        "stream_persist",
                        "delete failed partial placeholder id={} after assistant insert failed: {}",
                        id,
                        e
                    );
                }
            }
            *lock_or_poisoned(&self.streaming_role) = None;
        }
        lock_or_poisoned(&self.pending_text).clear();
    }

    /// Delete every row this attempt produced. Used only for failed
    /// attempts that are about to retry/fallback; final all-failed paths use
    /// `persist_failed_partial_assistant` to claim these rows instead.
    pub(crate) fn discard_attempt_rows(&self) {
        let ids = std::mem::take(&mut *lock_or_poisoned(&self.owned_partial_message_ids));
        for id in ids {
            if let Err(e) = self.db.delete_message_by_id(id) {
                app_warn!(
                    "session",
                    "stream_persist",
                    "discard attempt row id={} failed: {}",
                    id,
                    e
                );
            }
        }
        *lock_or_poisoned(&self.streaming_id) = None;
        *lock_or_poisoned(&self.streaming_role) = None;
        lock_or_poisoned(&self.pending_text).clear();
        lock_or_poisoned(&self.pending_thinking).clear();
        *lock_or_poisoned(&self.thinking_start_time) = None;
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
        msg.tokens_cache_creation = u
            .last_cache_creation_input_tokens
            .or(u.cache_creation_input_tokens);
        msg.tokens_cache_read = u.last_cache_read_input_tokens.or(u.cache_read_input_tokens);
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::agent_loader::DEFAULT_AGENT_ID;
    use crate::session::{MessageRole, NewMessage, SessionDB};

    use super::*;

    fn temp_db() -> Arc<SessionDB> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        // Leak tempdir for the test lifetime so SQLite can keep the file open.
        std::mem::forget(dir);
        Arc::new(SessionDB::open(&path).unwrap())
    }

    fn session_with_user(db: &SessionDB) -> String {
        let session = db.create_session(DEFAULT_AGENT_ID).unwrap();
        db.append_message(&session.id, &NewMessage::user("hello"))
            .unwrap();
        session.id
    }

    #[test]
    fn failed_partial_text_is_promoted_to_assistant_row() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);
        let cb = persister.build_callback();

        cb(r#"{"type":"text_delta","content":"partial answer"}"#);
        let assistant_id = persister
            .persist_failed_partial_assistant(None, 123)
            .expect("partial text should persist as assistant");

        let messages = db.load_session_messages(&session_id).unwrap();
        let assistant = messages
            .iter()
            .find(|msg| msg.id == assistant_id)
            .expect("assistant row exists");
        assert_eq!(assistant.role, MessageRole::Assistant);
        assert_eq!(assistant.content, "partial answer");
        assert_eq!(assistant.tool_duration_ms, Some(123));
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::TextBlock));
    }

    #[test]
    fn failed_partial_text_keeps_placeholder_when_assistant_insert_fails() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);
        let cb = persister.build_callback();

        cb(r#"{"type":"text_delta","content":"partial answer"}"#);
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_assistant_insert
                 BEFORE INSERT ON messages
                 WHEN NEW.role = 'assistant'
                 BEGIN
                   SELECT RAISE(ABORT, 'assistant insert failed');
                 END;",
            )
            .unwrap();

        assert!(
            persister
                .persist_failed_partial_assistant(None, 123)
                .is_none(),
            "trigger should force assistant insert failure"
        );
        let messages = db.load_session_messages(&session_id).unwrap();
        assert!(messages
            .iter()
            .any(|msg| { msg.role == MessageRole::TextBlock && msg.content == "partial answer" }));
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::Assistant));
    }

    #[test]
    fn failed_empty_attempt_does_not_create_empty_assistant_row() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);

        assert!(persister
            .persist_failed_partial_assistant(None, 50)
            .is_none());

        let messages = db.load_session_messages(&session_id).unwrap();
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::Assistant));
    }

    #[test]
    fn sealed_failed_partial_does_not_append_late_assistant() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);
        let cb = persister.build_callback();

        cb(r#"{"type":"text_delta","content":"already preserved"}"#);
        persister.cancel_flush_and_seal();
        assert!(persister
            .persist_failed_partial_assistant(None, 50)
            .is_none());

        let messages = db.load_session_messages(&session_id).unwrap();
        assert!(messages.iter().any(|msg| {
            msg.role == MessageRole::TextBlock && msg.content == "already preserved"
        }));
        assert!(!messages
            .iter()
            .any(|msg| msg.role == MessageRole::Assistant));
    }

    #[test]
    fn failed_tool_round_is_claimed_by_assistant_before_error_event() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);
        let cb = persister.build_callback();

        cb(r#"{"type":"text_delta","content":"I will check."}"#);
        cb(r#"{"type":"tool_call","call_id":"call-1","name":"read","arguments":"{}"}"#);
        cb(r#"{"type":"tool_result","call_id":"call-1","result":"ok","duration_ms":7}"#);
        let assistant_id = persister
            .persist_failed_partial_assistant(None, 456)
            .expect("completed text/tool fragments should be claimed by assistant");
        db.append_message(
            &session_id,
            &NewMessage::error_event("provider failed").with_source(ChatSource::Desktop),
        )
        .unwrap();

        let messages = db.load_session_messages(&session_id).unwrap();
        let assistant_idx = messages
            .iter()
            .position(|msg| msg.id == assistant_id)
            .expect("assistant row exists");
        let error_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::Event && msg.is_error == Some(true))
            .expect("error event exists");
        assert!(assistant_idx < error_idx);
        assert_eq!(messages[assistant_idx].role, MessageRole::Assistant);
        assert_eq!(messages[assistant_idx].content, "");
    }

    #[test]
    fn failed_thinking_only_creates_claimable_assistant_without_empty_text() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);
        let cb = persister.build_callback();

        cb(r#"{"type":"thinking_delta","content":"internal summary"}"#);
        let assistant_id = persister
            .persist_failed_partial_assistant(None, 321)
            .expect("thinking-only partial should be claimed for history grouping");

        let messages = db.load_session_messages(&session_id).unwrap();
        let thinking_idx = messages
            .iter()
            .position(|msg| msg.role == MessageRole::ThinkingBlock)
            .expect("thinking block should be preserved");
        let assistant_idx = messages
            .iter()
            .position(|msg| msg.id == assistant_id)
            .expect("assistant row should claim thinking block");
        assert!(thinking_idx < assistant_idx);
        assert_eq!(messages[assistant_idx].role, MessageRole::Assistant);
        assert_eq!(messages[assistant_idx].content, "");
    }

    #[test]
    fn discard_attempt_rows_preserves_event_rows() {
        let db = temp_db();
        let session_id = session_with_user(&db);
        let persister = StreamPersister::new(db.clone(), session_id.clone(), ChatSource::Desktop);
        let cb = persister.build_callback();

        cb(r#"{"type":"text_delta","content":"failed partial"}"#);
        cb(r#"{"type":"round_limit_reached","reason":"max_rounds"}"#);
        persister.discard_attempt_rows();

        let messages = db.load_session_messages(&session_id).unwrap();
        assert!(messages.iter().any(
            |msg| msg.role == MessageRole::Event && msg.content.contains("round_limit_reached")
        ));
        assert!(!messages
            .iter()
            .any(|msg| { msg.role == MessageRole::TextBlock && msg.content == "failed partial" }));
    }
}
