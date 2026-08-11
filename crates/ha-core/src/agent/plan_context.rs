//! Plan-context resolution: backend `PlanModeState` → the full bundle the
//! chat engine needs (`PlanAgentMode`, file path allow-list, trusted run
//! instruction and untrusted plan data). Centralized here so every chat entry point — Tauri
//! command, HTTP route, IM channel worker, cron executor, subagent spawn —
//! gets identical Plan-mode behavior. The pre-existing bug was that only
//! the Tauri path computed Plan context from the plan file, so
//! HTTP / channel / cron sessions in Plan Mode received PlanAgent tool
//! schemas without the `PLAN_MODE_SYSTEM_PROMPT` design contract or the
//! actual plan content under review/execution.
//!
//! Spawn-supplied overrides (currently `spawn_plan_subagent`) bypass the
//! backend probe — the spawn caller is the source of truth for child
//! sessions whose own backend `plan_mode` is `Off`.

use super::{plan_agent_mode_for_state, PlanAgentMode};
use crate::plan::{
    self, PlanModeState, PLAN_COMPLETED_SYSTEM_PROMPT, PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX,
    PLAN_MODE_SYSTEM_PROMPT,
};

/// Bundle of every Plan-derived input the chat engine threads into the
/// agent + system prompt. Constructed either from a backend snapshot
/// (`resolve_plan_context_for_session`) or supplied verbatim by the spawn
/// caller (`PlanResolvedContext::for_external_plan_agent`).
#[derive(Debug, Clone)]
pub struct PlanResolvedContext {
    /// Original `PlanModeState` this bundle was derived from. Cached on
    /// the agent so the streaming loop's mid-turn probe compares against
    /// the raw state — NOT the derived `mode`. Critical because
    /// `Planning` and `Review` both map to `PlanAgentMode::PlanAgent` (and
    /// `Completed` and `Off` both map to `PlanAgentMode::Off`), so a
    /// mode-only comparison would silently miss `Planning → Review` and
    /// `Completed → Off` transitions even though their run-context lanes
    /// differ materially.
    pub state: crate::plan::PlanModeState,
    /// Plan agent mode. `Off` is a valid value (regular session) — the
    /// chat engine still calls the appropriate setter so the agent's
    /// internal-mutability slot stays current.
    pub mode: PlanAgentMode,
    /// Path allow-list for path-aware write/edit during Planning/Review.
    /// Empty for non-PlanAgent modes.
    pub allow_paths: Vec<String>,
    /// Platform-maintained Plan contract. This retains developer authority,
    /// but is emitted after the stable cache boundary.
    pub run_instruction: Option<String>,
    /// User/model-authored Plan document. This always travels through the
    /// dynamic user-data lane and never inherits the Plan frame's authority.
    pub plan_data: Option<String>,
}

impl PlanResolvedContext {
    /// Idle / no-plan default. Used when a code path explicitly wants to
    /// run with no Plan-mode behavior (e.g. injection paths that send a
    /// plain notification message).
    pub fn off() -> Self {
        Self {
            state: PlanModeState::Off,
            mode: PlanAgentMode::Off,
            allow_paths: Vec::new(),
            run_instruction: None,
            plan_data: None,
        }
    }

    /// Spawn-supplied PlanAgent context. Used by `spawn_plan_subagent` to
    /// tell the chat engine "this child session should run as PlanAgent
    /// regardless of what its own backend `plan_mode` says (which is
    /// `Off`, since nobody has called `enter_plan_mode` on it)".
    pub fn for_external_plan_agent(run_instruction: Option<String>) -> Self {
        let (mode, allow_paths) = plan_agent_mode_for_state(PlanModeState::Planning);
        Self {
            state: PlanModeState::Planning,
            mode,
            allow_paths,
            run_instruction,
            plan_data: None,
        }
    }
}

/// Read this session's backend `plan_mode` and assemble the full
/// `PlanResolvedContext`. Called by the chat engine at turn start when no
/// `plan_context_override` was supplied. The streaming loop's mid-turn
/// probe uses the same building blocks (`plan_agent_mode_for_state` +
/// `PlanModeState`) so turn-start and mid-turn always see the same
/// resolution rules.
pub async fn resolve_plan_context_for_session(session_id: &str) -> PlanResolvedContext {
    let state = plan::get_plan_state(session_id).await;
    let (mode, allow_paths) = plan_agent_mode_for_state(state);
    let (run_instruction, plan_data) = match state {
        PlanModeState::Off => (None, None),
        PlanModeState::Planning => (Some(PLAN_MODE_SYSTEM_PROMPT.to_string()), None),
        PlanModeState::Review => (
            Some(
                "# Plan Review\n\nThe plan has been submitted and is awaiting user approval. Treat the plan document in run-context data as frozen evidence; do not execute it before approval."
                    .to_string(),
            ),
            plan::load_plan_file(session_id).ok().flatten(),
        ),
        PlanModeState::Executing => (
            Some(PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX.trim_end().to_string()),
            plan::load_plan_file(session_id).ok().flatten(),
        ),
        PlanModeState::Completed => (
            Some(PLAN_COMPLETED_SYSTEM_PROMPT.trim_end().to_string()),
            plan::load_plan_file(session_id).ok().flatten(),
        ),
    };
    PlanResolvedContext {
        state,
        mode,
        allow_paths,
        run_instruction,
        plan_data,
    }
}
