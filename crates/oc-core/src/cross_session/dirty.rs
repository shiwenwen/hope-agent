//! Dirty-bit broadcasting.
//!
//! When one session has activity, every *other* session's `SessionAwareness`
//! should be notified so that on its next turn it can decide whether to
//! rebuild its suffix. We avoid holding Arcs here to prevent leaks — we just
//! keep a small `HashSet<String>` of session IDs that are currently "dirty".

use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::RwLock;

/// Set of session IDs that should refresh their cross-session suffix on the
/// next turn. Sessions that are not registered here are still free to rebuild
/// on their normal time-window schedule.
static DIRTY: Lazy<RwLock<HashSet<String>>> = Lazy::new(|| RwLock::new(HashSet::new()));

/// Set of session IDs that currently have an active `SessionAwareness`.
/// Used so `mark_all_except` knows which observers to mark without visiting
/// every session ever seen. Trimmed on drop.
static OBSERVERS: Lazy<RwLock<HashSet<String>>> = Lazy::new(|| RwLock::new(HashSet::new()));

/// Register a session as an observer (called from `SessionAwareness::new`).
pub fn register_observer(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    if let Ok(mut g) = OBSERVERS.write() {
        g.insert(session_id.to_string());
    }
}

/// Unregister an observer (called from `SessionAwareness::drop`).
pub fn unregister_observer(session_id: &str) {
    if let Ok(mut g) = OBSERVERS.write() {
        g.remove(session_id);
    }
    if let Ok(mut g) = DIRTY.write() {
        g.remove(session_id);
    }
}

/// Called when `source_session_id` had activity. Marks every other observer
/// as dirty. Cheap: single write lock + HashSet iteration.
pub fn mark_all_except(source_session_id: &str) {
    let targets: Vec<String> = {
        let obs = match OBSERVERS.read() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        obs.iter()
            .filter(|id| id.as_str() != source_session_id)
            .cloned()
            .collect()
    };
    if targets.is_empty() {
        return;
    }
    let mut dirty = match DIRTY.write() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    for t in targets {
        dirty.insert(t);
    }
}

/// Take the dirty flag for this observer (clears it).
pub fn take_dirty(session_id: &str) -> bool {
    let mut guard = match DIRTY.write() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    guard.remove(session_id)
}

/// High-level helper called from `AssistantAgent::chat()`: touches the
/// registry *and* broadcasts dirty bits to peer sessions.
pub fn on_other_session_activity(source_session_id: &str) {
    super::registry::touch_active_session(source_session_id);
    mark_all_except(source_session_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_all_except_skips_source() {
        register_observer("sess-dirty-a");
        register_observer("sess-dirty-b");
        mark_all_except("sess-dirty-a");
        assert!(!take_dirty("sess-dirty-a"), "source should not be marked");
        assert!(take_dirty("sess-dirty-b"), "peer should be marked");
        unregister_observer("sess-dirty-a");
        unregister_observer("sess-dirty-b");
    }

    #[test]
    fn take_dirty_is_consuming() {
        register_observer("sess-dirty-c");
        register_observer("sess-dirty-d");
        mark_all_except("sess-dirty-c");
        assert!(take_dirty("sess-dirty-d"));
        assert!(!take_dirty("sess-dirty-d"));
        unregister_observer("sess-dirty-c");
        unregister_observer("sess-dirty-d");
    }
}
