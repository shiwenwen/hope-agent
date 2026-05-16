//! Ring buffer for browser observability events (console / network / errors).
//!
//! Single global instance per process. CDP backend feeds it directly from
//! `Console.messageAdded` / `Network.responseReceived` / `Runtime.exceptionThrown`
//! event subscribers.
//!
//! `observe` action reads from the buffer with an optional `since` cursor.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use super::backend::{ObserveEntry, ObserveKind};

const RING_CAPACITY: usize = 500;

struct Buffers {
    console: VecDeque<ObserveEntry>,
    network: VecDeque<ObserveEntry>,
    errors: VecDeque<ObserveEntry>,
}

impl Buffers {
    fn new() -> Self {
        Self {
            console: VecDeque::with_capacity(RING_CAPACITY),
            network: VecDeque::with_capacity(RING_CAPACITY),
            errors: VecDeque::with_capacity(RING_CAPACITY),
        }
    }
}

static BUFFERS: OnceLock<Mutex<Buffers>> = OnceLock::new();

fn buffers() -> &'static Mutex<Buffers> {
    BUFFERS.get_or_init(|| Mutex::new(Buffers::new()))
}

fn push_into(deque: &mut VecDeque<ObserveEntry>, entry: ObserveEntry) {
    if deque.len() == RING_CAPACITY {
        deque.pop_front();
    }
    deque.push_back(entry);
}

/// Append a captured event to the matching ring buffer.
pub fn push(kind: ObserveKind, entry: ObserveEntry) {
    if let Ok(mut buf) = buffers().lock() {
        match kind {
            ObserveKind::Console => push_into(&mut buf.console, entry),
            ObserveKind::Network => push_into(&mut buf.network, entry),
            ObserveKind::PageErrors => push_into(&mut buf.errors, entry),
        }
    }
}

/// Snapshot the buffer (cloning). `since` filters out entries with `at <= since`.
pub fn snapshot(kind: ObserveKind, since: Option<i64>) -> Vec<ObserveEntry> {
    let Ok(buf) = buffers().lock() else {
        return Vec::new();
    };
    let deque = match kind {
        ObserveKind::Console => &buf.console,
        ObserveKind::Network => &buf.network,
        ObserveKind::PageErrors => &buf.errors,
    };
    let cutoff = since.unwrap_or(i64::MIN);
    deque.iter().filter(|e| e.at > cutoff).cloned().collect()
}

/// Clear all three rings — called on `profile.disconnect` / `profile.launch`
/// to avoid leaking events from a previous session.
pub fn clear_all() {
    if let Ok(mut buf) = buffers().lock() {
        buf.console.clear();
        buf.network.clear();
        buf.errors.clear();
    }
}
