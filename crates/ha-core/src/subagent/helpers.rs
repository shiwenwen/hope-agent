use std::sync::Arc;

use crate::session::SessionDB;

use super::injection::flush_pending_injections;
use super::types::{ParentAgentStreamEvent, SubagentEvent};
use super::{INJECTING_SESSIONS, PENDING_INJECTIONS};

// ── Startup Recovery ────────────────────────────────────────────

static SUBAGENT_STARTUP_RECOVERY_ONCE: std::sync::Once = std::sync::Once::new();

fn run_startup_recovery_once(gate: &std::sync::Once, recovery: impl FnOnce()) {
    gate.call_once(recovery);
}

/// Clean up orphan sub-agent runs left in non-terminal state
/// (queued/spawning/running) from a previous app session. Called once during
/// synchronous Primary init, before any recovered ParentInjection is allowed to
/// run.
///
/// Delivery-state convergence intentionally happens here while dispatch is
/// deferred to [`replay_pending_parent_deliveries`]: ordinary interrupted
/// `injecting` claims return to startup replay, while `injecting_no_replay`
/// remains fenced because this process cannot prove another process is dead.
pub fn cleanup_orphan_runs(session_db: &Arc<SessionDB>) {
    run_startup_recovery_once(&SUBAGENT_STARTUP_RECOVERY_ONCE, || {
        cleanup_orphan_runs_inner(session_db)
    });
}

fn cleanup_orphan_runs_inner(session_db: &Arc<SessionDB>) {
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
    match session_db.recover_subagent_result_deliveries_on_startup() {
        Ok(()) => {}
        Err(error) => app_error!(
            "subagent",
            "delivery",
            "Failed to prepare pending parent result deliveries for startup replay: {}",
            error
        ),
    }
}

/// Dispatch ordinary pending parent deliveries after the Primary channel
/// startup/account-readiness sweeps run. This is a pure list + per-row CAS:
/// unlike one-shot startup convergence, it never rewrites an active
/// `injecting`/`injecting_no_replay` owner and is therefore safe to call after
/// every account start.
pub(crate) fn replay_pending_parent_deliveries(session_db: &Arc<SessionDB>) {
    match session_db.list_pending_subagent_deliveries() {
        Ok(runs) => {
            for run in runs {
                super::spawn::dispatch_parent_result_delivery(&run.run_id, session_db.clone());
            }
        }
        Err(error) => app_error!(
            "subagent",
            "delivery",
            "Failed to replay pending parent result deliveries: {}",
            error
        ),
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
    if let Some(db) = crate::get_session_db() {
        if let Err(error) = db.suppress_subagent_result_delivery(run_id, "explicitly_consumed") {
            app_warn!(
                "subagent",
                "delivery",
                "failed to persist consumed result delivery for run {}: {}",
                run_id,
                error
            );
        }
    }
    mark_run_fetched_in_memory(run_id);
}

/// Process-local fast path after the durable delivery row was already
/// suppressed transactionally or through `SessionDB::run`. Async callers must
/// use this helper instead of [`mark_run_fetched`] so SQLite never blocks a
/// Tokio worker.
pub fn mark_run_fetched_in_memory(run_id: &str) {
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

pub(super) fn release_injection_owner(
    injecting: &mut std::collections::HashMap<String, String>,
    session_id: &str,
    run_id: &str,
) -> bool {
    if injecting.get(session_id).map(String::as_str) != Some(run_id) {
        return false;
    }
    injecting.remove(session_id);
    true
}

/// RAII guard that removes only its exact `(session, run)` owner from
/// INJECTING_SESSIONS when dropped. A stale guard must never clear a newer run.
pub(crate) struct CleanupGuard {
    pub session_id: String,
    pub run_id: String,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        // Keep the established INJECTING -> PENDING lock order. A concurrent
        // new B must observe either this active owner or the queued A at the
        // head; it must never slip through the release-to-flush gap.
        let released = {
            let mut injecting = INJECTING_SESSIONS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _pending = PENDING_INJECTIONS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            release_injection_owner(&mut injecting, &self.session_id, &self.run_id)
        };
        // Re-trigger next pending injection for this session (serial execution)
        if released {
            flush_pending_injections(&self.session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mark_run_fetched, run_startup_recovery_once, take_runs_fetched};
    use crate::subagent::{ActiveInjection, INJECTION_CANCELS};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn startup_recovery_gate_runs_exactly_once_under_concurrency() {
        let gate = std::sync::Once::new();
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    run_startup_recovery_once(&gate, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                    });
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

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
                    im_mirror: Arc::new(
                        crate::subagent::injection::ActiveInjectionMirrorCoordinator::new(None),
                    ),
                },
            );
            active.insert(
                other_session.clone(),
                ActiveInjection {
                    run_id: other_run,
                    cancel: other_cancel.clone(),
                    im_mirror: Arc::new(
                        crate::subagent::injection::ActiveInjectionMirrorCoordinator::new(None),
                    ),
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
