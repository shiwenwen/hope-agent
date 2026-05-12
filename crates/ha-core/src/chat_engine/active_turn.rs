//! Per-session guard for user-facing chat turns.
//!
//! This sits one layer above `stream_seq`: callers acquire it before they
//! persist the user message, so reloads or duplicate "continue" clicks cannot
//! create a second main turn for the same session.

use std::collections::HashMap;
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
}
