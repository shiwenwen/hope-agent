use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::session::SessionDB;

// ── Cancel Registry ─────────────────────────────────────────────

/// In-memory registry for active sub-agent cancel flags.
/// Uses AtomicBool (same pattern as chat_cancel in the codebase).
pub struct SubagentCancelRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl SubagentCancelRegistry {
    pub fn new() -> Self {
        Self {
            flags: Mutex::new(HashMap::new()),
        }
    }

    /// Register a cancel flag for a run, returning the Arc<AtomicBool>.
    pub fn register(&self, run_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.flags.lock() {
            map.insert(run_id.to_string(), flag.clone());
        }
        flag
    }

    /// Signal cancellation for a specific run.
    pub fn cancel(&self, run_id: &str) -> bool {
        if let Ok(map) = self.flags.lock() {
            if let Some(flag) = map.get(run_id) {
                flag.store(true, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    /// Cancel all active runs for a given parent session.
    pub fn cancel_all_for_session(&self, parent_session_id: &str, db: &SessionDB) -> u32 {
        let run_ids: Vec<String> = db.list_active_subagent_runs(parent_session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.run_id)
            .collect();

        let mut count = 0u32;
        if let Ok(map) = self.flags.lock() {
            for rid in &run_ids {
                if let Some(flag) = map.get(rid) {
                    flag.store(true, Ordering::SeqCst);
                    count += 1;
                }
            }
        }
        count
    }

    /// Remove a completed/terminated run from the registry.
    pub fn remove(&self, run_id: &str) {
        if let Ok(mut map) = self.flags.lock() {
            map.remove(run_id);
        }
    }
}
