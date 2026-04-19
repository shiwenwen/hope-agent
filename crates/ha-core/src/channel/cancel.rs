use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ── Channel Cancel Registry ─────────────────────────────────────

/// In-memory registry for active channel stream cancel flags, keyed by session ID.
/// Follows the same pattern as `SubagentCancelRegistry`.
pub struct ChannelCancelRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ChannelCancelRegistry {
    pub fn new() -> Self {
        Self {
            flags: Mutex::new(HashMap::new()),
        }
    }

    /// Register a cancel flag for a session, returning the Arc<AtomicBool>.
    pub fn register(&self, session_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.flags.lock() {
            map.insert(session_id.to_string(), flag.clone());
        }
        flag
    }

    /// Signal cancellation for a specific session's active stream.
    pub fn cancel(&self, session_id: &str) -> bool {
        if let Ok(map) = self.flags.lock() {
            if let Some(flag) = map.get(session_id) {
                flag.store(true, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    /// Remove a completed session from the registry.
    pub fn remove(&self, session_id: &str) {
        if let Ok(mut map) = self.flags.lock() {
            map.remove(session_id);
        }
    }
}
