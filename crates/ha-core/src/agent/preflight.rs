//! User-prompt preflight — the single chokepoint every user-message entry
//! point (Tauri / HTTP / IM / ACP) passes through *before* persisting the
//! message to the DB (design doc F3 fix).
//!
//! Phase 0.1 ships this as a pure pass-through: it returns the raw prompt
//! unchanged so behavior is identical. The point of landing it now is that all
//! four entry points already route through one helper, so when PR 1.2 wires
//! the `UserPromptSubmit` hook (which can `block` the message or rewrite it via
//! `additionalContext`), only this function changes — not the four call sites.

/// What an entry point should do after preflight.
#[derive(Debug, Clone)]
pub enum PreflightOutcome {
    /// Persist + run the turn with this (possibly hook-modified) prompt.
    Proceed { effective_prompt: String },
    // PR 1.2 adds `Block { reason, system_message }` for `UserPromptSubmit`
    // hooks that deny the message.
}

/// Inputs to [`user_prompt_preflight`].
#[derive(Debug, Clone, Copy)]
pub struct PreflightArgs<'a> {
    /// Target session id.
    pub session_id: &'a str,
    /// The content that is about to be persisted as the user message.
    pub raw_prompt: &'a str,
}

/// Run preflight for a user prompt. **Phase 0.1: pass-through.**
///
/// Always returns [`PreflightOutcome::Proceed`] with the prompt unchanged. The
/// `async` signature is intentional — PR 1.2 awaits the `UserPromptSubmit`
/// hook dispatch here.
pub async fn user_prompt_preflight(args: PreflightArgs<'_>) -> PreflightOutcome {
    PreflightOutcome::Proceed {
        effective_prompt: args.raw_prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pass_through_returns_input_unchanged() {
        let out = user_prompt_preflight(PreflightArgs {
            session_id: "s1",
            raw_prompt: "hello world",
        })
        .await;
        match out {
            PreflightOutcome::Proceed { effective_prompt } => {
                assert_eq!(effective_prompt, "hello world");
            }
        }
    }
}
