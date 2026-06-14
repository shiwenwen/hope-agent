//! Per-session "already edited" file tracker for Smart mode.
//!
//! Once a `write` / `edit` / `apply_patch` call is allowed to proceed in a
//! session (auto-allowed in-workspace, self-tagged, judge-approved, or
//! user-approved), the target path(s) are recorded here. Smart mode then
//! auto-allows later edits to the same file in the same session — the user
//! already consented to touching it, so re-prompting is just noise.
//!
//! Scope: process-global, in-memory, keyed by session id. Only [`engine`]'s
//! Smart resolver reads it, so recording in other modes is harmless. Paths are
//! stored in the canonical resolved form produced by
//! [`super::rules::resolved_edit_target_paths`] so lookups match exactly.
//!
//! [`engine`]: super::engine

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// Backstop so a pathological session can't grow the set without bound. Far
/// above any realistic edit count for one conversation.
const MAX_PATHS_PER_SESSION: usize = 4096;

fn store() -> &'static Mutex<HashMap<String, HashSet<PathBuf>>> {
    static STORE: OnceLock<Mutex<HashMap<String, HashSet<PathBuf>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember that `path` was edited in `session_id`. No-op for an empty session
/// id. Idempotent. Over the per-session cap, new paths are dropped (existing
/// ones still match) rather than evicting — the cap is only a runaway guard.
pub fn record(session_id: &str, path: &Path) {
    if session_id.is_empty() {
        return;
    }
    let mut map = store().lock().unwrap_or_else(PoisonError::into_inner);
    let set = map.entry(session_id.to_string()).or_default();
    if set.len() >= MAX_PATHS_PER_SESSION && !set.contains(path) {
        return;
    }
    set.insert(path.to_path_buf());
}

/// Whether `path` was already edited earlier in `session_id`.
pub fn contains(session_id: &str, path: &Path) -> bool {
    if session_id.is_empty() {
        return false;
    }
    store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session_id)
        .is_some_and(|set| set.contains(path))
}

/// Drop a session's recorded edits (e.g. on session deletion). Optional —
/// the map is small and in-memory, so leaking a closed session is harmless.
pub fn clear(session_id: &str) {
    store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(session_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The store is process-global and tests run concurrently, so each test
    // uses unique session ids and cleans up only its own — never a global wipe.

    #[test]
    fn record_then_contains() {
        let p = Path::new("/repo/se_rtc/main.rs");
        assert!(!contains("se-rtc-a", p));
        record("se-rtc-a", p);
        assert!(contains("se-rtc-a", p));
        // Scoped per session.
        assert!(!contains("se-rtc-b", p));
        // Distinct path not recorded.
        assert!(!contains("se-rtc-a", Path::new("/repo/se_rtc/other.rs")));
        clear("se-rtc-a");
    }

    #[test]
    fn empty_session_is_noop() {
        record("", Path::new("/repo/se_empty/x"));
        assert!(!contains("", Path::new("/repo/se_empty/x")));
    }

    #[test]
    fn clear_drops_session() {
        let p = Path::new("/repo/se_cds/y");
        record("se-cds", p);
        assert!(contains("se-cds", p));
        clear("se-cds");
        assert!(!contains("se-cds", p));
    }
}
