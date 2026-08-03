use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ── Channel Cancel Registry ─────────────────────────────────────

/// In-memory registry for active channel stream cancel flags, keyed by session ID.
/// Follows the same pattern as `SubagentCancelRegistry`.
pub struct ChannelCancelRegistry {
    /// More than one inbound may be preparing for the same session while an
    /// earlier turn is active. Keep each request's flag so `/stop` cancels all
    /// preflight/persistence/engine phases without one registration replacing
    /// another.
    flags: Mutex<HashMap<String, HashMap<String, Arc<AtomicBool>>>>,
}

pub struct ChannelCancelRegistration {
    pub id: String,
    pub cancel: Arc<AtomicBool>,
}

impl ChannelCancelRegistry {
    pub fn new() -> Self {
        Self {
            flags: Mutex::new(HashMap::new()),
        }
    }

    fn lock_flags(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, HashMap<String, Arc<AtomicBool>>>> {
        self.flags
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Register one cancellable inbound request for a session.
    pub fn register(&self, session_id: &str) -> ChannelCancelRegistration {
        let id = uuid::Uuid::new_v4().to_string();
        let flag = Arc::new(AtomicBool::new(false));
        self.lock_flags()
            .entry(session_id.to_string())
            .or_default()
            .insert(id.clone(), flag.clone());
        ChannelCancelRegistration { id, cancel: flag }
    }

    /// Signal cancellation for a specific session's active stream.
    pub fn cancel(&self, session_id: &str) -> bool {
        let map = self.lock_flags();
        if let Some(flags) = map.get(session_id) {
            for flag in flags.values() {
                flag.store(true, Ordering::SeqCst);
            }
            return !flags.is_empty();
        }
        false
    }

    /// Signal every active Channel inbound and return the affected sessions.
    /// Used only by the legacy/emergency global Stop fallback.
    pub fn cancel_all(&self) -> Vec<String> {
        let map = self.lock_flags();
        map.iter()
            .filter_map(|(session_id, flags)| {
                if flags.is_empty() {
                    return None;
                }
                for flag in flags.values() {
                    flag.store(true, Ordering::SeqCst);
                }
                Some(session_id.clone())
            })
            .collect()
    }

    /// Remove exactly one completed inbound registration.
    pub fn remove(&self, session_id: &str, registration_id: &str) {
        let mut map = self.lock_flags();
        if let Some(flags) = map.get_mut(session_id) {
            flags.remove(registration_id);
            if flags.is_empty() {
                map.remove(session_id);
            }
        }
    }

    /// Whether this session currently has a live cancel handle — i.e. a
    /// channel-driven turn is in flight. Used by attach catch-up to decide
    /// whether to append the "reply is being generated" hint.
    pub fn is_active(&self, session_id: &str) -> bool {
        self.lock_flags()
            .get(session_id)
            .is_some_and(|flags| !flags.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_cancels_every_inbound_registered_for_the_session() {
        let registry = ChannelCancelRegistry::new();
        let first = registry.register("session-1");
        let second = registry.register("session-1");
        let other = registry.register("session-2");

        assert!(registry.cancel("session-1"));
        assert!(first.cancel.load(Ordering::SeqCst));
        assert!(second.cancel.load(Ordering::SeqCst));
        assert!(!other.cancel.load(Ordering::SeqCst));

        registry.remove("session-1", &first.id);
        assert!(registry.is_active("session-1"));
        registry.remove("session-1", &second.id);
        assert!(!registry.is_active("session-1"));

        assert_eq!(registry.cancel_all(), vec!["session-2".to_string()]);
        assert!(other.cancel.load(Ordering::SeqCst));
    }
}
