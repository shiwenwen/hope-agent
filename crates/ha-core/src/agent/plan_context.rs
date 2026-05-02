//! Plan-context resolution: backend `PlanModeState` → the full bundle the
//! chat engine needs (`PlanAgentMode`, file path allow-list, system-prompt
//! injection text). Centralized here so every chat entry point — Tauri
//! command, HTTP route, IM channel worker, cron executor, subagent spawn —
//! gets identical Plan-mode behavior. The pre-existing bug was that only
//! the Tauri path computed `extra_system_context` from the plan file, so
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
    /// `Completed → Off` transitions even though their `extra_system_context`
    /// is materially different (Review embeds the just-submitted plan
    /// content, Completed embeds the executed plan).
    pub state: crate::plan::PlanModeState,
    /// Plan agent mode. `Off` is a valid value (regular session) — the
    /// chat engine still calls the appropriate setter so the agent's
    /// internal-mutability slot stays current.
    pub mode: PlanAgentMode,
    /// Path allow-list for path-aware write/edit during Planning/Review.
    /// Empty for non-PlanAgent modes.
    pub allow_paths: Vec<String>,
    /// Plan-derived system-prompt segment. `None` for `Off`. Lives in a
    /// dedicated agent slot (`plan_extra_context`) so it's appended after
    /// the caller-supplied `extra_system_context` without overwriting it.
    pub extra_system_context: Option<String>,
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
            extra_system_context: None,
        }
    }

    /// Spawn-supplied PlanAgent context. Used by `spawn_plan_subagent` to
    /// tell the chat engine "this child session should run as PlanAgent
    /// regardless of what its own backend `plan_mode` says (which is
    /// `Off`, since nobody has called `enter_plan_mode` on it)".
    pub fn for_external_plan_agent(extra_system_context: Option<String>) -> Self {
        let (mode, allow_paths) = plan_agent_mode_for_state(PlanModeState::Planning);
        Self {
            state: PlanModeState::Planning,
            mode,
            allow_paths,
            extra_system_context,
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
    let extra_system_context = match state {
        PlanModeState::Off => None,
        PlanModeState::Planning => Some(PLAN_MODE_SYSTEM_PROMPT.to_string()),
        // Review fallback to the bare planning prompt when the file vanished
        // (rare — would require external deletion mid-flight). Keeps the
        // model in Planning shape rather than returning to a bare session.
        PlanModeState::Review => plan::load_plan_file(session_id)
            .ok()
            .flatten()
            .map(|content| {
                format!(
                    "# Plan Review\n\nThe following plan has been submitted and is awaiting user approval:\n\n{}",
                    content
                )
            })
            .or_else(|| Some(PLAN_MODE_SYSTEM_PROMPT.to_string())),
        PlanModeState::Executing => plan::load_plan_file(session_id)
            .ok()
            .flatten()
            .map(|content| format!("{}{}", PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX, content)),
        PlanModeState::Completed => plan::load_plan_file(session_id)
            .ok()
            .flatten()
            .map(|content| format!("{}{}", PLAN_COMPLETED_SYSTEM_PROMPT, content)),
    };
    PlanResolvedContext {
        state,
        mode,
        allow_paths,
        extra_system_context,
    }
}

/// Merge the caller's optional `extra_system_context` with the
/// plan-derived one. Caller's text comes first so cron/skill/etc. context
/// frames the model's task before the Plan Mode contract.
pub fn merge_extra_system_context(
    caller: Option<String>,
    plan: Option<String>,
) -> Option<String> {
    match (caller, plan) {
        (Some(c), Some(p)) => Some(format!("{}\n\n{}", c, p)),
        (Some(c), None) => Some(c),
        (None, Some(p)) => Some(p),
        (None, None) => None,
    }
}
