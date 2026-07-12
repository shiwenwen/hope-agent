use crate::session::SessionDB;

use super::injection::flush_pending_injections;
use super::types::{ParentAgentStreamEvent, SubagentEvent};
use super::INJECTING_SESSIONS;

// ── Startup Recovery ────────────────────────────────────────────

/// Clean up orphan sub-agent runs left in non-terminal state (spawning/running)
/// from a previous app session. Called once at startup.
pub fn cleanup_orphan_runs(session_db: &SessionDB) {
    match session_db.cleanup_orphan_subagent_runs() {
        Ok(affected) if affected > 0 => {
            app_warn!(
                "subagent",
                "startup",
                "Cleaned up {} orphan sub-agent run(s)",
                affected
            );
        }
        Err(e) => {
            app_error!(
                "subagent",
                "startup",
                "Failed to clean up orphan runs: {}",
                e
            );
        }
        _ => {}
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Truncate a string to max chars, appending "..." if truncated.
pub(crate) fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", cut)
    }
}

/// Emit a sub-agent event to the frontend via the global EventBus.
pub(crate) fn emit_subagent_event(event: &SubagentEvent) {
    if let Some(bus) = crate::get_event_bus() {
        if let Ok(payload) = serde_json::to_value(event) {
            bus.emit("subagent_event", payload);
        }
    }
}

/// Emit a parent agent stream event to the frontend.
pub(crate) fn emit_parent_stream_event(event: &ParentAgentStreamEvent) {
    if let Some(bus) = crate::get_event_bus() {
        if let Ok(payload) = serde_json::to_value(event) {
            bus.emit("parent_agent_stream", payload);
        }
    }
}

/// Mark a run_id as having its result already read by the parent agent.
pub fn mark_run_fetched(run_id: &str) {
    if let Ok(mut set) = super::FETCHED_RUN_IDS.lock() {
        set.insert(run_id.to_string());
    }
    if let Ok(active) = super::INJECTION_CANCELS.lock() {
        for injection in active.values() {
            if injection.run_id == run_id {
                injection
                    .cancel
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

/// R5: remove the given run ids from the fetched set, returning how many were
/// present. The Group join uses this to detect "the parent already collected
/// every child result" (via `wait_all` / `check` / `result`) and skip the
/// redundant merged injection — while also cleaning up marks that the
/// suppressed per-child injections would otherwise leave behind forever.
pub fn take_runs_fetched(run_ids: &[String]) -> usize {
    if run_ids.is_empty() {
        return 0;
    }
    let mut set = super::FETCHED_RUN_IDS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    run_ids.iter().filter(|id| set.remove(*id)).count()
}

/// RAII guard that removes a session from INJECTING_SESSIONS when dropped.
pub(crate) struct CleanupGuard {
    pub session_id: String,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = INJECTING_SESSIONS.lock() {
            guard.remove(&self.session_id);
        }
        // Re-trigger next pending injection for this session (serial execution)
        flush_pending_injections(&self.session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{mark_run_fetched, take_runs_fetched};
    use crate::subagent::{ActiveInjection, INJECTION_CANCELS};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn fetched_run_cancels_only_the_matching_active_injection() {
        let target_run = format!("workflow-checkpoint-{}", uuid::Uuid::new_v4());
        let other_run = format!("other-injection-{}", uuid::Uuid::new_v4());
        let target_session = format!("session-{}", uuid::Uuid::new_v4());
        let other_session = format!("session-{}", uuid::Uuid::new_v4());
        let target_cancel = Arc::new(AtomicBool::new(false));
        let other_cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = INJECTION_CANCELS.lock().expect("active injections");
            active.insert(
                target_session.clone(),
                ActiveInjection {
                    run_id: target_run.clone(),
                    cancel: target_cancel.clone(),
                },
            );
            active.insert(
                other_session.clone(),
                ActiveInjection {
                    run_id: other_run,
                    cancel: other_cancel.clone(),
                },
            );
        }

        mark_run_fetched(&target_run);

        assert!(target_cancel.load(Ordering::SeqCst));
        assert!(!other_cancel.load(Ordering::SeqCst));
        INJECTION_CANCELS
            .lock()
            .expect("active injections")
            .retain(|session_id, _| session_id != &target_session && session_id != &other_session);
        assert_eq!(take_runs_fetched(&[target_run]), 1);
    }
}
