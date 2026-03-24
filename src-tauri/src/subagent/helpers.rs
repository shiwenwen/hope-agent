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
            app_warn!("subagent", "startup", "Cleaned up {} orphan sub-agent run(s)", affected);
        }
        Err(e) => {
            app_error!("subagent", "startup", "Failed to clean up orphan runs: {}", e);
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

/// Emit a sub-agent event to the frontend via Tauri global event.
pub(crate) fn emit_subagent_event(event: &SubagentEvent) {
    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = handle.emit("subagent_event", event);
    }
}

/// Emit a parent agent stream event to the frontend.
pub(crate) fn emit_parent_stream_event(event: &ParentAgentStreamEvent) {
    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = handle.emit("parent_agent_stream", event);
    }
}

/// Mark a run_id as having its result already read by the parent agent.
pub fn mark_run_fetched(run_id: &str) {
    if let Ok(mut set) = super::FETCHED_RUN_IDS.lock() {
        set.insert(run_id.to_string());
    }
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
