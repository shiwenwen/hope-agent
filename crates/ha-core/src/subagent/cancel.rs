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

    /// Register (or fetch) the cancel flag for a run, returning the
    /// `Arc<AtomicBool>`. **Get-or-create**: if a flag is already registered
    /// (R7.2 — it was registered at PARK time and this call is the promoted run
    /// reusing it via `launch_subagent_run`), the SAME flag is returned so a
    /// cancel signalled while the run was parked stays visible to the launched
    /// engine. A fresh, untripped flag is created only when none exists.
    pub fn register(&self, run_id: &str) -> Arc<AtomicBool> {
        if let Ok(mut map) = self.flags.lock() {
            return map
                .entry(run_id.to_string())
                .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                .clone();
        }
        Arc::new(AtomicBool::new(false))
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
        let run_ids: Vec<String> = db
            .list_active_subagent_runs(parent_session_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_is_get_or_create_and_preserves_trip() {
        // R7.2 promote-vs-cancel: a cancel signalled while a spawn is PARKED
        // must stay visible to the engine the promoter later launches. The
        // promoted run re-registers under the same run_id and must get the SAME,
        // already-tripped flag — a fresh flag would lose the cancel and let a
        // killed run execute to completion.
        let reg = SubagentCancelRegistry::new();
        let flag1 = reg.register("run-x");
        assert!(!flag1.load(Ordering::SeqCst));

        // Cancel while "parked".
        assert!(reg.cancel("run-x"));

        // Promotion re-registers — same Arc, trip preserved.
        let flag2 = reg.register("run-x");
        assert!(
            Arc::ptr_eq(&flag1, &flag2),
            "re-register must return the same flag, not a fresh one"
        );
        assert!(
            flag2.load(Ordering::SeqCst),
            "the cancel signalled while parked must survive re-registration"
        );

        // A distinct run still gets its own fresh, untripped flag.
        let other = reg.register("run-y");
        assert!(!other.load(Ordering::SeqCst));
        assert!(!Arc::ptr_eq(&flag1, &other));
    }
}
