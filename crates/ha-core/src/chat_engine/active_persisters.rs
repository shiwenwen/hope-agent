//! Process-wide registry of in-flight `StreamPersister`s, used by the
//! crash-flush hook (panic / SIGINT / SIGTERM) to promote any active
//! placeholder rows from `streaming` to `completed` before the process
//! exits, so the buffered partial text isn't lost.
//!
//! Entries hold `Weak<StreamPersister>` — the persister itself owns its
//! `Arc<SessionDB>`, so the registry only needs one weak reference per
//! turn. Entries self-prune once the `Arc` count drops to zero.
//! `flush_all_blocking` is synchronous so it can run from a panic hook
//! or signal handler where awaiting is unsafe; rusqlite is synchronous
//! anyway.

use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::persister::StreamPersister;

fn active() -> &'static Mutex<Vec<Weak<StreamPersister>>> {
    static ACTIVE: OnceLock<Mutex<Vec<Weak<StreamPersister>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register an in-flight persister and opportunistically prune dead
/// entries so the registry doesn't grow unbounded across long-running
/// sessions.
pub(crate) fn register(persister: &Arc<StreamPersister>) {
    let Ok(mut guard) = active().lock() else {
        return;
    };
    guard.retain(|w| w.strong_count() > 0);
    guard.push(Arc::downgrade(persister));
}

/// Synchronous best-effort flush of all in-flight persisters. Called from
/// the panic hook (any thread that panics) and the SIGINT/SIGTERM signal
/// handler (graceful shutdown). Each persister finalizes its placeholder
/// to `completed` using the latest buffered content.
pub fn flush_all_blocking() {
    let entries: Vec<Weak<StreamPersister>> = match active().lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(p) => std::mem::take(&mut *p.into_inner()),
    };
    let mut flushed = 0usize;
    for weak in entries {
        if let Some(persister) = weak.upgrade() {
            persister.crash_flush();
            flushed += 1;
        }
    }
    if flushed > 0 {
        app_info!(
            "session",
            "stream_persist",
            "crash flush completed for {} active persister(s)",
            flushed
        );
    }
}
