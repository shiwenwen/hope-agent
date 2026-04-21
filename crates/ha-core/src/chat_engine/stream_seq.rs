//! Per-session monotonic seq counters used to de-duplicate chat stream deltas
//! between the primary per-call sink path and the EventBus reattach path.
//!
//! The same registry also powers `active_counts()` — the single source of
//! truth for "how many chat engines are running right now" consumed by the
//! `/api/server/status` endpoint. Because `run_chat_engine` wraps its entire
//! lifetime in a `StreamLifecycle` Drop guard that calls [`begin`] / [`end`],
//! `active_counts` automatically covers desktop / HTTP / IM-channel paths
//! without a parallel tracker.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Which caller opened this chat stream. Surfaced in server runtime status
/// so the tooltip can split "N active sessions" into `X desktop · Y http
/// · Z channel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatSource {
    /// Tauri desktop shell (user talking in the GUI).
    Desktop,
    /// External HTTP client talking to the embedded server's `POST /api/chat`.
    Http,
    /// IM channel worker replying to an inbound message (Slack / 等).
    Channel,
}

struct Entry {
    counter: Arc<AtomicU64>,
    source: ChatSource,
}

static REGISTRY: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Mark the session as running. Resets the counter to 0 and records which
/// caller opened the stream.
pub fn begin(session_id: &str, source: ChatSource) {
    let mut map = registry().lock().expect("stream_seq registry poisoned");
    map.insert(
        session_id.to_string(),
        Entry {
            counter: Arc::new(AtomicU64::new(0)),
            source,
        },
    );
}

/// Drop the session entry, marking it as no longer streaming.
pub fn end(session_id: &str) {
    let mut map = registry().lock().expect("stream_seq registry poisoned");
    map.remove(session_id);
}

/// Return the next `seq` for this session, or `0` if the session isn't
/// registered (defensive — callers should [`begin`] first).
pub fn next_seq(session_id: &str) -> u64 {
    let map = registry().lock().expect("stream_seq registry poisoned");
    if let Some(entry) = map.get(session_id) {
        entry.counter.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        0
    }
}

/// Current value of the counter (highest issued seq).
pub fn last_seq(session_id: &str) -> u64 {
    let map = registry().lock().expect("stream_seq registry poisoned");
    map.get(session_id)
        .map(|e| e.counter.load(Ordering::SeqCst))
        .unwrap_or(0)
}

/// Whether the session is currently registered (run_chat is running).
pub fn is_active(session_id: &str) -> bool {
    let map = registry().lock().expect("stream_seq registry poisoned");
    map.contains_key(session_id)
}

/// Breakdown of how many chat engines are running right now, by caller.
/// `total` is just `desktop + http + channel`, exposed so the UI doesn't
/// have to sum client-side.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveChatCounts {
    pub desktop: u32,
    pub http: u32,
    pub channel: u32,
    pub total: u32,
}

/// Snapshot of in-flight chat sessions by source. Cheap: one lock + one
/// pass over an in-memory HashMap whose size is bounded by concurrent users.
pub fn active_counts() -> ActiveChatCounts {
    let map = registry().lock().expect("stream_seq registry poisoned");
    let mut out = ActiveChatCounts::default();
    for entry in map.values() {
        match entry.source {
            ChatSource::Desktop => out.desktop += 1,
            ChatSource::Http => out.http += 1,
            ChatSource::Channel => out.channel += 1,
        }
    }
    out.total = out.desktop + out.http + out.channel;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // All tests share one process-wide REGISTRY, so each test uses a unique
    // session_id prefix and cleans up after itself to stay independent.

    #[test]
    fn begin_end_roundtrip() {
        let sid = "test-stream_seq-begin_end";
        assert!(!is_active(sid));
        begin(sid, ChatSource::Desktop);
        assert!(is_active(sid));
        assert_eq!(last_seq(sid), 0);
        assert_eq!(next_seq(sid), 1);
        assert_eq!(next_seq(sid), 2);
        assert_eq!(last_seq(sid), 2);
        end(sid);
        assert!(!is_active(sid));
        // After end(), next_seq returns 0 (defensive fallback).
        assert_eq!(next_seq(sid), 0);
    }

    #[test]
    fn active_counts_splits_by_source() {
        let base = "test-stream_seq-counts";
        let d1 = format!("{base}-d1");
        let d2 = format!("{base}-d2");
        let h1 = format!("{base}-h1");
        let c1 = format!("{base}-c1");

        begin(&d1, ChatSource::Desktop);
        begin(&d2, ChatSource::Desktop);
        begin(&h1, ChatSource::Http);
        begin(&c1, ChatSource::Channel);

        let counts = active_counts();
        // Other tests may have sessions running concurrently; assert on the
        // delta we just created by pulling baseline afterwards via cleanup.
        assert!(counts.desktop >= 2);
        assert!(counts.http >= 1);
        assert!(counts.channel >= 1);
        assert_eq!(counts.total, counts.desktop + counts.http + counts.channel);

        end(&d1);
        end(&d2);
        end(&h1);
        end(&c1);
    }
}
