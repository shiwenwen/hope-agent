//! In-memory registry of recently-touched sessions.
//!
//! Every `AssistantAgent::chat()` call touches its session ID here; the
//! collector uses this to mark a session as `Active` in the snapshot.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

static REGISTRY: Lazy<RwLock<HashMap<String, Instant>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Mark `session_id` as active now.
pub fn touch_active_session(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let mut guard = match REGISTRY.write() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    guard.insert(session_id.to_string(), Instant::now());
    // Opportunistic GC: drop entries older than max(active_window * 2, 600s).
    // Uses the global config's active_window_secs as the baseline.
    let window = crate::config::cached_config().cross_session.active_window_secs;
    let gc_secs = (window.saturating_mul(2)).max(600);
    let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(gc_secs));
    if let Some(cutoff) = cutoff {
        guard.retain(|_, ts| *ts >= cutoff);
    }
}

/// Return session IDs that have been touched since `cutoff`.
pub fn active_since(cutoff: Instant) -> Vec<String> {
    let guard = match REGISTRY.read() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    guard
        .iter()
        .filter_map(|(k, ts)| if *ts >= cutoff { Some(k.clone()) } else { None })
        .collect()
}

/// Full snapshot — used by peek_tool and debugging.
pub fn active_snapshot() -> Vec<(String, Instant)> {
    let guard = match REGISTRY.read() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    guard.iter().map(|(k, v)| (k.clone(), *v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn touch_and_query_within_window() {
        touch_active_session("sess-regtest-1");
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(Instant::now);
        let active = active_since(cutoff);
        assert!(active.contains(&"sess-regtest-1".to_string()));
    }

    #[test]
    fn empty_id_is_ignored() {
        touch_active_session("");
        let snap = active_snapshot();
        assert!(!snap.iter().any(|(k, _)| k.is_empty()));
    }
}
