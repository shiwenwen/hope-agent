//! Centralized plan-state transition helper. All entry points (UI / slash /
//! tools / IM channel / HTTP) go through `transition_state` so the canonical
//! side-effect bundle stays in sync:
//!
//! 1. Cancel active plan subagent when transitioning **to** Off.
//! 2. Cleanup git checkpoint when transitioning **to** Off or Completed.
//! 3. Create git checkpoint when transitioning **to** Executing (and none
//!    exists yet).
//! 4. Persist new state to the session DB.
//! 5. Emit `plan_mode_changed` so PlanPanel / detached window / IM channel
//!    can refresh.
//!
//! Each caller picks a stable `reason` string (e.g. `"slash_exit"`,
//! `"all_tasks_completed"`) which lands in `plan_mode_changed.reason` so the
//! frontend / telemetry can attribute the change.

use serde_json::json;

use super::{
    cleanup_checkpoint, create_checkpoint_for_session, get_active_plan_run_id, get_checkpoint_ref,
    get_plan_state, set_plan_state, should_create_execution_checkpoint, store, PlanModeState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// In-memory + DB updated, event emitted.
    Applied,
    /// State machine refused the edge (illegal transition). No side effects
    /// downstream of `set_plan_state` were executed.
    Rejected,
}

/// Apply a plan-state transition with all canonical side effects.
///
/// On `Ok(Applied)`: in-memory state updated, checkpoint managed, DB
/// persisted, `plan_mode_changed` emitted.
///
/// On `Ok(Rejected)`: the state machine refused the edge. No downstream side
/// effects ran (cancel-subagent fires before `set_plan_state` only when
/// target is Off, which is always a valid edge — so unreachable in practice).
///
/// `Err` only on DB persist failure. The in-memory state is already updated
/// at that point; caller decides whether to surface or log-and-continue.
pub async fn transition_state(
    session_id: &str,
    target: PlanModeState,
    reason: &'static str,
) -> anyhow::Result<TransitionOutcome> {
    let checkpoint_ref = get_checkpoint_ref(session_id).await;
    let should_create_checkpoint = if target == PlanModeState::Executing {
        let previous_state = get_plan_state(session_id).await;
        let persisted_plan_mode = crate::get_session_db()
            .and_then(|db| db.get_session(session_id).ok().flatten())
            .map(|meta| meta.plan_mode);
        should_create_execution_checkpoint(
            &target,
            &previous_state,
            persisted_plan_mode,
            checkpoint_ref.is_some(),
        )
    } else {
        false
    };
    let checkpoint_to_cleanup = if matches!(target, PlanModeState::Off | PlanModeState::Completed) {
        checkpoint_ref
    } else {
        None
    };

    if target == PlanModeState::Off {
        if let Some(run_id) = get_active_plan_run_id(session_id).await {
            if let Some(cancels) = crate::get_subagent_cancels() {
                cancels.cancel(&run_id);
                app_info!(
                    "plan",
                    "transition",
                    "Cancelled plan sub-agent {} (reason={})",
                    run_id,
                    reason
                );
            }
        }
    }

    if !set_plan_state(session_id, target).await {
        return Ok(TransitionOutcome::Rejected);
    }

    // Stamp `executing_started_at` on transitions INTO Executing so
    // `maybe_complete_plan` can scope its "all tasks done" check to tasks
    // created since this point (avoids false trigger from pre-existing
    // session-scoped tasks, and false miss when a re-entry leaves stale ones).
    if target == PlanModeState::Executing {
        let mut map = store().write().await;
        if let Some(meta) = map.get_mut(session_id) {
            meta.executing_started_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    if let Some(ref_name) = checkpoint_to_cleanup {
        cleanup_checkpoint(&ref_name);
        // Off removes the PlanMeta entry outright; Completed keeps it, so the
        // stale ref must be cleared explicitly or `get_plan_checkpoint` will
        // keep returning a now-deleted branch and the rollback button breaks.
        if target == PlanModeState::Completed {
            let mut map = store().write().await;
            if let Some(meta) = map.get_mut(session_id) {
                meta.checkpoint_ref = None;
            }
        }
    }

    if should_create_checkpoint {
        create_checkpoint_for_session(session_id).await;
    }

    if let Some(db) = crate::get_session_db() {
        db.update_session_plan_mode(session_id, target)?;
    }

    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "plan_mode_changed",
            json!({
                "sessionId": session_id,
                "state": target.as_str(),
                "reason": reason,
            }),
        );
    }

    Ok(TransitionOutcome::Applied)
}
