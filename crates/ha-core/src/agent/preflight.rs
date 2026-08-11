//! User-prompt preflight — the single chokepoint every user-message entry
//! point (Tauri / HTTP / IM / ACP) passes through *before* persisting the
//! message to the DB (design doc F3 fix).
//!
//! This is where the `UserPromptSubmit` hook runs. A blocking decision stops
//! the prompt before it is persisted or run; otherwise any injected
//! `additionalContext` is stashed for the turn's untrusted data envelope
//! (drained next to `SessionStart`), and the prompt proceeds unchanged. All
//! four entry points route through here, so the hook semantics live in one
//! place — the call sites only branch on `Block`.

use crate::hooks::HookDecision;
use std::sync::atomic::{AtomicBool, Ordering};

/// Stable error marker used by transport shells when an explicitly stopped
/// turn is cancelled before its user message has been persisted.
pub const CHAT_CANCELLED_DURING_PREFLIGHT_CODE: &str = "chat_cancelled_during_preflight";

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
    /// The id of the turn this prompt will run as — the same value the entry
    /// point hands to [`crate::chat_engine::active_turn::try_acquire`], so the
    /// `UserPromptSubmit` hook carries the same `prompt_id` as every in-turn
    /// hook that follows (PreToolUse → Stop).
    ///
    /// Required rather than `Option`: a new entry point must consciously supply
    /// one, which is what keeps the field non-null by construction instead of
    /// by convention. Pass `""` only where no turn will run.
    pub turn_id: &'a str,
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
    let outcome = crate::hooks::fire_user_prompt_submit(
        args.session_id,
        args.agent_id,
        args.raw_prompt,
        args.turn_id,
    )
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

    // Not blocked: stash any injected context (or clear the slot) for the
    // turn-local data envelope. The persisted user message stays exactly what
    // the user typed.
    crate::hooks::set_user_prompt_context(args.session_id, outcome.merged_additional_context());
    PreflightOutcome::Proceed {
        effective_prompt: args.raw_prompt.to_string(),
    }
}

/// Cancellation-aware wrapper for foreground chat entry points.
///
/// `UserPromptSubmit` handlers can legitimately wait on a command, network
/// request, model, or sub-agent. The active-turn cancel flag is acquired
/// before preflight, so Stop must be able to drop that work instead of waiting
/// for the handler's potentially minutes-long timeout. `None` means the caller
/// must not persist or execute the prompt.
pub async fn user_prompt_preflight_cancellable(
    args: PreflightArgs<'_>,
    cancel: &AtomicBool,
) -> Option<PreflightOutcome> {
    if cancel.load(Ordering::SeqCst) {
        crate::hooks::set_user_prompt_context(args.session_id, None);
        return None;
    }

    tokio::select! {
        biased;
        _ = async {
            while !cancel.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        } => {
            crate::hooks::set_user_prompt_context(args.session_id, None);
            None
        }
        outcome = user_prompt_preflight(args) => {
            finish_cancellable_preflight(args.session_id, cancel, outcome)
        },
    }
}

/// Close the final race between the cancellation poll and hook completion.
/// `user_prompt_preflight` may already have stashed additional context, so a
/// late Stop must also clear that slot before the shell can persist the prompt.
fn finish_cancellable_preflight(
    session_id: &str,
    cancel: &AtomicBool,
    outcome: PreflightOutcome,
) -> Option<PreflightOutcome> {
    if cancel.load(Ordering::SeqCst) {
        crate::hooks::set_user_prompt_context(session_id, None);
        None
    } else {
        Some(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn pass_through_returns_input_unchanged() {
        // No UserPromptSubmit hook configured → pass-through.
        let out = user_prompt_preflight(PreflightArgs {
            session_id: "preflight-test-s1",
            agent_id: None,
            raw_prompt: "hello world",
            turn_id: "preflight-test-turn-1",
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

    #[tokio::test]
    async fn cancelled_preflight_never_proceeds() {
        let cancel = AtomicBool::new(true);
        let out = user_prompt_preflight_cancellable(
            PreflightArgs {
                session_id: "preflight-cancelled-s1",
                agent_id: None,
                raw_prompt: "must not persist",
                turn_id: "preflight-cancelled-turn-1",
            },
            &cancel,
        )
        .await;

        assert!(out.is_none());
        assert!(crate::hooks::take_user_prompt_context("preflight-cancelled-s1").is_none());
    }

    #[test]
    fn cancellation_after_hook_completion_discards_outcome_and_context() {
        let session_id = "preflight-cancelled-after-hook-s1";
        crate::hooks::set_user_prompt_context(session_id, Some("hook context".into()));
        let cancel = AtomicBool::new(true);

        let out = finish_cancellable_preflight(
            session_id,
            &cancel,
            PreflightOutcome::Proceed {
                effective_prompt: "must not persist".into(),
            },
        );

        assert!(out.is_none());
        assert!(crate::hooks::take_user_prompt_context(session_id).is_none());
    }
}
