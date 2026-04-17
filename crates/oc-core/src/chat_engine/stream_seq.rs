//! Per-session monotonic seq counters used to de-duplicate chat stream deltas
//! between the primary per-call sink path and the EventBus reattach path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<AtomicU64>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Arc<AtomicU64>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Mark the session as running. Resets the counter to 0.
pub fn begin(session_id: &str) {
    let mut map = registry().lock().expect("stream_seq registry poisoned");
    map.insert(session_id.to_string(), Arc::new(AtomicU64::new(0)));
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
    if let Some(counter) = map.get(session_id) {
        counter.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        0
    }
}

/// Current value of the counter (highest issued seq).
pub fn last_seq(session_id: &str) -> u64 {
    let map = registry().lock().expect("stream_seq registry poisoned");
    map.get(session_id)
        .map(|c| c.load(Ordering::SeqCst))
        .unwrap_or(0)
}

/// Whether the session is currently registered (run_chat is running).
pub fn is_active(session_id: &str) -> bool {
    let map = registry().lock().expect("stream_seq registry poisoned");
    map.contains_key(session_id)
}
