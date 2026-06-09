//! Per-session guard for user-facing chat turns.
//!
//! This sits one layer above `stream_seq`: callers acquire it before they
//! persist the user message, so reloads or duplicate "continue" clicks cannot
//! create a second main turn for the same session.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use super::stream_seq::{ChatSource, ACTIVE_STREAM_ERROR_CODE};

#[derive(Debug, Clone)]
pub struct ActiveTurnError {
    pub session_id: String,
    pub existing_source: ChatSource,
}

impl fmt::Display for ActiveTurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{ACTIVE_STREAM_ERROR_CODE}: session {} already has an active {} chat turn",
            self.session_id, self.existing_source
        )
    }
}

impl std::error::Error for ActiveTurnError {}

#[derive(Debug, Clone)]
struct Entry {
    token: String,
    turn_id: String,
    stream_id: Option<String>,
    source: ChatSource,
    cancel: Arc<AtomicBool>,
}

static ACTIVE_TURNS: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    ACTIVE_TURNS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug)]
pub struct ActiveTurnGuard {
    session_id: String,
    token: String,
    released: bool,
}

impl ActiveTurnGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        let mut map = registry()
            .lock()
            .expect("active chat turn registry poisoned");
        if map
            .get(&self.session_id)
            .map(|entry| entry.token.as_str() == self.token)
            .unwrap_or(false)
        {
            map.remove(&self.session_id);
        }
        self.released = true;
    }
}

impl Drop for ActiveTurnGuard {
    fn drop(&mut self) {
        self.release();
    }
}

pub fn try_acquire(
    session_id: &str,
    source: ChatSource,
    turn_id: String,
    cancel: Arc<AtomicBool>,
) -> Result<ActiveTurnGuard, ActiveTurnError> {
    let token = uuid::Uuid::new_v4().to_string();
    let mut map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    if let Some(existing) = map.get(session_id) {
        return Err(ActiveTurnError {
            session_id: session_id.to_string(),
            existing_source: existing.source,
        });
    }
    map.insert(
        session_id.to_string(),
        Entry {
            token: token.clone(),
            turn_id,
            stream_id: None,
            source,
            cancel,
        },
    );
    Ok(ActiveTurnGuard {
        session_id: session_id.to_string(),
        token,
        released: false,
    })
}

#[derive(Debug, Clone)]
pub struct ActiveTurnSnapshot {
    pub session_id: String,
    pub turn_id: String,
    pub stream_id: Option<String>,
    pub source: ChatSource,
    pub cancel: Arc<AtomicBool>,
}

pub fn current(session_id: &str) -> Option<ActiveTurnSnapshot> {
    let map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    map.get(session_id).map(|entry| ActiveTurnSnapshot {
        session_id: session_id.to_string(),
        turn_id: entry.turn_id.clone(),
        stream_id: entry.stream_id.clone(),
        source: entry.source,
        cancel: Arc::clone(&entry.cancel),
    })
}

/// Fast-path check for the per-token streaming hot loop: returns
/// `Some(accepting)` (`accepting = !cancel`) when `(session_id, turn_id)` is
/// the live active turn, **without** cloning the snapshot's Strings + Arc that
/// [`current`] allocates. Returns `None` when no entry matches that exact turn
/// (the caller decides the fallback — see `turn_accepts_stream_event`).
pub fn is_accepting(session_id: &str, turn_id: &str) -> Option<bool> {
    let map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    map.get(session_id).and_then(|entry| {
        if entry.turn_id == turn_id {
            Some(!entry.cancel.load(std::sync::atomic::Ordering::SeqCst))
        } else {
            None
        }
    })
}

/// True when the session has *any* live active-turn entry (turn_id agnostic).
/// Lets `turn_accepts_stream_event` preserve the original "a different turn is
/// live → reject without a DB probe" semantics without cloning a snapshot.
pub fn has_entry(session_id: &str) -> bool {
    registry()
        .lock()
        .expect("active chat turn registry poisoned")
        .contains_key(session_id)
}

pub fn all_current() -> Vec<ActiveTurnSnapshot> {
    let map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    map.iter()
        .map(|(session_id, entry)| ActiveTurnSnapshot {
            session_id: session_id.clone(),
            turn_id: entry.turn_id.clone(),
            stream_id: entry.stream_id.clone(),
            source: entry.source,
            cancel: Arc::clone(&entry.cancel),
        })
        .collect()
}

pub fn all_current_turn_ids() -> Vec<String> {
    let map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    map.values().map(|entry| entry.turn_id.clone()).collect()
}

/// Force-release one active turn by `(session_id, turn_id)`.
///
/// Used by the user-stop watchdog after it has already finalized the turn in
/// persistent state. The turn id guard prevents an old watchdog from clearing
/// a newer turn that started in the same session.
pub fn force_release(session_id: &str, turn_id: &str) -> bool {
    let mut map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    let matches = map
        .get(session_id)
        .map(|entry| entry.turn_id == turn_id)
        .unwrap_or(false);
    if matches {
        map.remove(session_id);
    }
    matches
}

/// Clear all in-memory active turn entries.
///
/// Used during runtime startup after persisted `running` / `cancelling` turns
/// have been marked interrupted. This is mostly relevant for hot-reload/dev
/// processes where Rust statics can outlive a logical app restart.
pub fn clear_all() -> usize {
    let mut map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    let n = map.len();
    map.clear();
    n
}

// ── Finalize re-entry guard ───────────────────────────────────────────
//
// `finalize_turn_context` can plausibly be invoked twice for the same
// turn — engine.rs failure convergence races with a SIGTERM signal
// handler walking `all_current()`; startup sweep races with a
// crash-flush left over from the previous run. The second call must
// be a no-op (already wrote `[系统事件]` marker, already wrote event
// row, already finished chat_turn). Re-entry guard is keyed by turn
// id so cross-session pairs don't interfere.

/// Bounded FIFO so a long-running process doesn't accumulate every
/// finalized turn id forever. 4096 × ~50 bytes ≈ 200 KiB worst case,
/// well above realistic re-entry windows (the same turn id is only
/// reused at process restart, and we want re-entry detection during
/// the **same** process lifetime).
const FINALIZED_RING_MAX: usize = 4096;

struct FinalizedRing {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl FinalizedRing {
    fn new() -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `true` if `id` was newly inserted.
    fn insert(&mut self, id: String) -> bool {
        if !self.set.insert(id.clone()) {
            return false;
        }
        self.order.push_back(id);
        while self.order.len() > FINALIZED_RING_MAX {
            if let Some(evicted) = self.order.pop_front() {
                self.set.remove(&evicted);
            }
        }
        true
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.set.clear();
        self.order.clear();
    }
}

static FINALIZED_TURNS: OnceLock<Mutex<FinalizedRing>> = OnceLock::new();

fn finalized_ring() -> &'static Mutex<FinalizedRing> {
    FINALIZED_TURNS.get_or_init(|| Mutex::new(FinalizedRing::new()))
}

/// Test-and-insert: returns `true` if this is the *first* finalize
/// call for `turn_id`; subsequent calls return `false` and the caller
/// must short-circuit. Passing `None` (sweep paths with no `turn_id`)
/// always returns `true` — those callers handle idempotency by other
/// means (DB UPDATE conditions, mostly).
pub fn mark_finalized(turn_id: Option<&str>) -> bool {
    let Some(id) = turn_id else { return true };
    let mut ring = finalized_ring()
        .lock()
        .expect("finalized turn registry poisoned");
    ring.insert(id.to_string())
}

/// Reset the re-entry guard. Test-only.
#[cfg(test)]
pub(crate) fn reset_finalized_for_test() {
    if let Ok(mut ring) = finalized_ring().lock() {
        ring.clear();
    }
}

pub fn set_stream_id(session_id: &str, turn_id: &str, stream_id: &str) -> bool {
    let mut map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    match map.get_mut(session_id) {
        Some(entry) if entry.turn_id == turn_id => {
            entry.stream_id = Some(stream_id.to_string());
            true
        }
        _ => false,
    }
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("active turn test lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_second_turn_until_guard_drops() {
        let _lock = test_lock();
        let sid = "test-active-turn-rejects-second";
        {
            let _guard = try_acquire(
                sid,
                ChatSource::Desktop,
                "turn-1".to_string(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            let err = try_acquire(
                sid,
                ChatSource::Http,
                "turn-2".to_string(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap_err();
            assert_eq!(err.session_id, sid);
            assert_eq!(err.existing_source, ChatSource::Desktop);
        }

        let _guard = try_acquire(
            sid,
            ChatSource::Http,
            "turn-3".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    }

    #[test]
    fn current_snapshot_tracks_stream_id() {
        let _lock = test_lock();
        let sid = "test-active-turn-current-snapshot";
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-current".to_string(),
            Arc::clone(&cancel),
        )
        .unwrap();

        assert_eq!(current(sid).unwrap().turn_id, "turn-current");
        assert!(set_stream_id(sid, "turn-current", "stream-current"));
        let snapshot = current(sid).unwrap();
        assert_eq!(snapshot.stream_id.as_deref(), Some("stream-current"));
        assert!(Arc::ptr_eq(&snapshot.cancel, &cancel));
        assert!(!set_stream_id(sid, "other-turn", "stream-other"));
    }

    #[test]
    fn is_accepting_and_has_entry_match_current_semantics() {
        let _lock = test_lock();
        let sid = "test-active-turn-is-accepting";
        // No entry yet.
        assert_eq!(is_accepting(sid, "turn-x"), None);
        assert!(!has_entry(sid));

        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-acc".to_string(),
            Arc::clone(&cancel),
        )
        .unwrap();

        // Matching live turn, not cancelled → Some(true).
        assert_eq!(is_accepting(sid, "turn-acc"), Some(true));
        assert!(has_entry(sid));
        // Session has an entry but under a *different* turn → None (caller
        // rejects without a DB probe, preserving old semantics).
        assert_eq!(is_accepting(sid, "turn-other"), None);
        // Cancelled → Some(false).
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(is_accepting(sid, "turn-acc"), Some(false));
    }

    #[test]
    fn all_current_returns_cancel_handles() {
        let _lock = test_lock();
        let sid = "test-active-turn-all-current";
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-all-current".to_string(),
            Arc::clone(&cancel),
        )
        .unwrap();

        let snapshot = all_current()
            .into_iter()
            .find(|snapshot| snapshot.session_id == sid)
            .unwrap();
        assert_eq!(snapshot.turn_id, "turn-all-current");
        assert!(Arc::ptr_eq(&snapshot.cancel, &cancel));
    }

    #[test]
    fn mark_finalized_is_one_shot_per_turn_id() {
        let _lock = test_lock();
        reset_finalized_for_test();
        assert!(mark_finalized(Some("t-first")));
        assert!(!mark_finalized(Some("t-first")));
        assert!(mark_finalized(Some("t-second")));
        // None means "no turn id" — always proceed (callers handle
        // idempotency another way).
        assert!(mark_finalized(None));
        assert!(mark_finalized(None));
    }

    #[test]
    fn clear_all_removes_active_turns() {
        let _lock = test_lock();
        let sid = "test-active-turn-clear-all";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-clear".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(current(sid).is_some());
        assert!(clear_all() >= 1);
        assert!(current(sid).is_none());
    }

    #[test]
    fn force_release_requires_matching_turn_id() {
        let _lock = test_lock();
        let sid = "test-active-turn-force-release";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-force".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(!force_release(sid, "other-turn"));
        assert!(current(sid).is_some());
        assert!(force_release(sid, "turn-force"));
        assert!(current(sid).is_none());
    }
}
