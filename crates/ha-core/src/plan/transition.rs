//! Centralized plan-state transition helper.
//!
//! Plan-state changes have several side effects that **must** stay in sync
//! across every caller (UI, slash, tools, IM channel, HTTP):
//!
//! 1. Cancel active plan subagent when transitioning **to** Off.
//! 2. Cleanup git checkpoint when transitioning **to** Off or Completed.
//! 3. Create git checkpoint when transitioning **to** Executing (and none
//!    exists yet).
//! 4. Persist new state to the session DB.
//! 5. Emit `plan_mode_changed` so PlanPanel / detached window / IM channel
//!    can refresh.
//!
//! Before this helper, those steps were duplicated in 6 call sites
//! (`tools/enter_plan_mode.rs`, `tools/submit_plan.rs`,
//! `slash_commands/handlers/plan.rs`, `src-tauri/src/commands/plan.rs`,
//! `crates/ha-server/src/routes/plan.rs`, `tools/task.rs::maybe_complete_plan`).
//! The duplication caused real drift: the slash `/plan exit` path used to
//! forget cancel-subagent, and none of the slash / Tauri / HTTP paths emitted
//! `plan_mode_changed`. F-037 in `docs/plans/review-followups.md` tracked
//! the debt.
//!
//! All transition entry points now go through `transition_state(...)`. A
//! caller picks a stable `reason` string and (optionally) opts out of the
//! defaults; everything else is taken care of in one place.

use serde_json::json;

use super::{
    cleanup_checkpoint, create_checkpoint_for_session, get_active_plan_run_id, get_checkpoint_ref,
    get_plan_state, set_plan_state, should_create_execution_checkpoint, PlanModeState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// In-memory + DB updated, event emitted.
    Applied,
    /// State machine refused the edge (illegal transition). No side effects
    /// downstream of `set_plan_state` were executed.
    Rejected,
}

#[derive(Debug, Clone)]
pub struct TransitionOpts {
    /// Value for the `reason` field in `plan_mode_changed`. Each caller
    /// should pick a stable, descriptive reason (e.g. `"slash_exit"`,
    /// `"tool_enter_plan_mode"`, `"all_tasks_completed"`) so frontend and
    /// telemetry can attribute the change.
    pub reason: &'static str,
    /// When true, cancel any active plan subagent on Off transitions.
    /// Defaults to true; opt out only for tests or unusual paths.
    pub cancel_subagent_on_off: bool,
    /// When true, manage the git checkpoint:
    ///   * cleanup on Off / Completed
    ///   * create on Executing (when none exists)
    ///
    /// Defaults to true.
    pub manage_checkpoint: bool,
}

impl TransitionOpts {
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason,
            cancel_subagent_on_off: true,
            manage_checkpoint: true,
        }
    }
}

/// Apply a plan-state transition with all canonical side effects.
///
/// On `Ok(Applied)`: in-memory state updated, checkpoint managed (per opts),
/// DB persisted, `plan_mode_changed` emitted.
///
/// On `Ok(Rejected)`: the state machine refused (e.g. illegal edge). The
/// only thing that may have run before the rejection is `cancel_subagent`
/// (which fires before `set_plan_state` when target is Off, but Off is
/// always a valid edge so this is unreachable in practice).
///
/// `Err` only on DB persist failure. The in-memory state is already updated
/// at that point — the caller decides whether to surface the error to the
/// user or log-and-continue.
pub async fn transition_state(
    session_id: &str,
    target: PlanModeState,
    opts: TransitionOpts,
) -> anyhow::Result<TransitionOutcome> {
    // Snapshot pre-transition state for checkpoint decisions.
    let previous_state = get_plan_state(session_id).await;
    let persisted_plan_mode = crate::get_session_db()
        .and_then(|db| db.get_session(session_id).ok().flatten())
        .map(|meta| meta.plan_mode);
    let checkpoint_exists = get_checkpoint_ref(session_id).await.is_some();
    let should_create_checkpoint = opts.manage_checkpoint
        && should_create_execution_checkpoint(
            &target,
            &previous_state,
            persisted_plan_mode,
            checkpoint_exists,
        );
    let checkpoint_to_cleanup = if opts.manage_checkpoint
        && (target == PlanModeState::Off || target == PlanModeState::Completed)
    {
        get_checkpoint_ref(session_id).await
    } else {
        None
    };

    if opts.cancel_subagent_on_off && target == PlanModeState::Off {
        if let Some(run_id) = get_active_plan_run_id(session_id).await {
            if let Some(cancels) = crate::get_subagent_cancels() {
                cancels.cancel(&run_id);
                app_info!(
                    "plan",
                    "transition",
                    "Cancelled plan sub-agent {} (reason={})",
                    run_id,
                    opts.reason
                );
            }
        }
    }

    if !set_plan_state(session_id, target).await {
        return Ok(TransitionOutcome::Rejected);
    }

    if let Some(ref_name) = checkpoint_to_cleanup {
        cleanup_checkpoint(&ref_name);
    }

    // Create git checkpoint AFTER PlanMeta entry exists in the store.
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
                "reason": opts.reason,
            }),
        );
    }

    Ok(TransitionOutcome::Applied)
}
