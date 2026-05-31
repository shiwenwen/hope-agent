//! User-prompt preflight — the single chokepoint every user-message entry
//! point (Tauri / HTTP / IM / ACP) passes through *before* persisting the
//! message to the DB (design doc F3 fix).
//!
//! This is where the `UserPromptSubmit` hook runs. A blocking decision stops
//! the prompt before it is persisted or run; otherwise any injected
//! `additionalContext` is stashed for the turn to fold into its system prompt
//! (drained next to `SessionStart`), and the prompt proceeds unchanged. All
//! four entry points route through here, so the hook semantics live in one
//! place — the call sites only branch on `Block`.

use crate::hooks::HookDecision;

/// What an entry point should do after preflight.
#[derive(Debug, Clone)]
pub enum PreflightOutcome {
    /// Persist + run the turn with this prompt.
    Proceed { effective_prompt: String },
    /// A `UserPromptSubmit` hook blocked the prompt: do **not** persist it as a
    /// user message (a blocked prompt must never enter the conversation /
    /// LLM context) and do not run a turn. Surface `reason` to the user.
    Block { reason: String },
}

/// Inputs to [`user_prompt_preflight`].
#[derive(Debug, Clone, Copy)]
pub struct PreflightArgs<'a> {
    /// Target session id.
    pub session_id: &'a str,
    /// The agent that will run the turn (for the hook's `agent_id`; best-effort
    /// — `None` where the entry point hasn't resolved it yet).
    pub agent_id: Option<&'a str>,
    /// The content that is about to be persisted as the user message.
    pub raw_prompt: &'a str,
}

/// Run preflight for a user prompt: fire the `UserPromptSubmit` hook and decide
/// whether the turn proceeds.
///
/// - A `block`/`deny` decision (or `continue:false`) → [`PreflightOutcome::Block`].
/// - Otherwise → [`PreflightOutcome::Proceed`] with the prompt unchanged; any
///   hook `additionalContext` is stashed via
///   [`crate::hooks::set_user_prompt_context`] for the turn to inject.
///
/// No-hook fast path: the hook helper returns a no-op outcome, so this is a
/// pass-through (and `set_user_prompt_context(None)` just clears any stale slot).
pub async fn user_prompt_preflight(args: PreflightArgs<'_>) -> PreflightOutcome {
    let outcome =
        crate::hooks::fire_user_prompt_submit(args.session_id, args.agent_id, args.raw_prompt)
            .await;

    // A blocking decision (or an explicit `continue:false`) stops the prompt
    // before it is persisted or run.
    let blocked_reason = match &outcome.decision {
        HookDecision::Block { reason } | HookDecision::Deny { reason } => Some(reason.clone()),
        // Ask/Defer have no meaning for UserPromptSubmit (they are PreToolUse
        // semantics) → treated as proceed below.
        _ if !outcome.continue_execution => Some(outcome.stop_reason.clone().unwrap_or_default()),
        _ => None,
    };
    if let Some(reason) = blocked_reason {
        // Clear any stale slot so the next turn on this session starts clean.
        crate::hooks::set_user_prompt_context(args.session_id, None);
        let reason = if reason.trim().is_empty() {
            "Prompt blocked by a UserPromptSubmit hook.".to_string()
        } else {
            reason
        };
        return PreflightOutcome::Block { reason };
    }

    // Not blocked: stash any injected context (or clear the slot) for the turn
    // to fold into its system prompt. The context rides the system prompt, not
    // the persisted user message — so the message stays exactly what the user
    // typed.
    crate::hooks::set_user_prompt_context(args.session_id, outcome.merged_additional_context());
    PreflightOutcome::Proceed {
        effective_prompt: args.raw_prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pass_through_returns_input_unchanged() {
        // No UserPromptSubmit hook configured → pass-through.
        let out = user_prompt_preflight(PreflightArgs {
            session_id: "preflight-test-s1",
            agent_id: None,
            raw_prompt: "hello world",
        })
        .await;
        match out {
            PreflightOutcome::Proceed { effective_prompt } => {
                assert_eq!(effective_prompt, "hello world");
            }
            PreflightOutcome::Block { .. } => panic!("expected Proceed with no hook configured"),
        }
        // No context configured → slot cleared.
        assert!(crate::hooks::take_user_prompt_context("preflight-test-s1").is_none());
    }
}
