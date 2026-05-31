//! Decision aggregation across the hooks matched for one event (design doc
//! §9.1/§9.2).
//!
//! Each hook contributes a [`HookContribution`]; [`aggregate`] folds them into
//! a single [`HookOutcome`] using the official precedence
//! `deny > block > defer > ask > allow`, concatenating injected context in
//! order. Observation events only ever yield `Allow` + `additional_context`,
//! but the full precedence is implemented here so later phases plug in
//! unchanged.

use super::types::{HookDecision, HookOutcome};

/// One hook's parsed contribution to the aggregate. Produced by the runner's
/// output parser (`parse.rs`); consumed only here.
#[derive(Debug, Default, Clone)]
pub struct HookContribution {
    pub decision: HookDecision,
    /// Set only for an explicit `permissionDecision:"allow"` (PreToolUse), so a
    /// deliberate auto-approve is distinguishable from the default `Allow`.
    pub permission_allow: bool,
    pub continue_execution: bool,
    pub stop_reason: Option<String>,
    pub system_message: Option<String>,
    pub additional_context: Option<String>,
    pub session_title: Option<String>,
    pub updated_input: Option<serde_json::Value>,
    pub updated_mcp_output: Option<serde_json::Value>,
    pub retry: bool,
}

impl HookContribution {
    /// A contribution that changes nothing (used for non-blocking errors).
    pub fn inert() -> Self {
        Self {
            decision: HookDecision::Allow,
            continue_execution: true,
            ..Default::default()
        }
    }
}

/// Precedence rank — higher wins (design doc §9.1).
fn rank(d: &HookDecision) -> u8 {
    match d {
        HookDecision::Allow => 0,
        HookDecision::Ask => 1,
        HookDecision::Defer => 2,
        HookDecision::Block { .. } => 3,
        HookDecision::Deny { .. } => 4,
    }
}

/// Fold all contributions into a single outcome.
pub fn aggregate(contributions: Vec<HookContribution>) -> HookOutcome {
    if contributions.is_empty() {
        return HookOutcome::noop();
    }

    let mut outcome = HookOutcome::noop();
    let mut best_rank = 0u8;

    for c in contributions {
        // Highest-precedence decision wins; ties keep the first seen.
        let r = rank(&c.decision);
        if r > best_rank {
            best_rank = r;
            outcome.decision = c.decision;
        }

        // `continue: false` from any hook terminates the loop.
        if !c.continue_execution {
            outcome.continue_execution = false;
            if outcome.stop_reason.is_none() {
                outcome.stop_reason = c.stop_reason;
            }
        }

        if let Some(ctx) = c.additional_context {
            if !ctx.trim().is_empty() {
                outcome.additional_context.push(ctx);
            }
        }
        // First non-empty system message / session title wins; updatedInput
        // chains (last writer wins, matching successive rewrites).
        if outcome.system_message.is_none() {
            outcome.system_message = c.system_message;
        }
        if outcome.session_title.is_none() {
            outcome.session_title = c.session_title;
        }
        if c.updated_input.is_some() {
            outcome.updated_input = c.updated_input;
        }
        if c.updated_mcp_output.is_some() {
            outcome.updated_mcp_output = c.updated_mcp_output;
        }
        outcome.permission_allow = outcome.permission_allow || c.permission_allow;
        outcome.retry = outcome.retry || c.retry;
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow_with_ctx(ctx: &str) -> HookContribution {
        HookContribution {
            additional_context: Some(ctx.to_string()),
            ..HookContribution::inert()
        }
    }

    #[test]
    fn empty_is_noop() {
        let out = aggregate(vec![]);
        assert_eq!(out.decision, HookDecision::Allow);
        assert!(out.continue_execution);
        assert!(out.merged_additional_context().is_none());
    }

    #[test]
    fn additional_context_concatenated_in_order() {
        let out = aggregate(vec![allow_with_ctx("first"), allow_with_ctx("second")]);
        let merged = out.merged_additional_context().unwrap();
        assert!(merged.starts_with("first"));
        assert!(merged.ends_with("second"));
        assert!(merged.contains("---"));
        // Blank context is dropped.
        let out2 = aggregate(vec![allow_with_ctx("   "), allow_with_ctx("real")]);
        assert_eq!(out2.merged_additional_context().unwrap(), "real");
    }

    #[test]
    fn deny_beats_block_beats_ask_beats_allow() {
        let out = aggregate(vec![
            HookContribution {
                decision: HookDecision::Ask,
                ..HookContribution::inert()
            },
            HookContribution {
                decision: HookDecision::Deny {
                    reason: "no".into(),
                },
                ..HookContribution::inert()
            },
            HookContribution {
                decision: HookDecision::Block {
                    reason: "warn".into(),
                },
                ..HookContribution::inert()
            },
        ]);
        assert_eq!(
            out.decision,
            HookDecision::Deny {
                reason: "no".into()
            }
        );
    }

    #[test]
    fn continue_false_propagates_with_reason() {
        let out = aggregate(vec![
            HookContribution::inert(),
            HookContribution {
                continue_execution: false,
                stop_reason: Some("halt".into()),
                ..HookContribution::inert()
            },
        ]);
        assert!(!out.continue_execution);
        assert_eq!(out.stop_reason.as_deref(), Some("halt"));
    }

    #[test]
    fn updated_input_last_writer_wins() {
        let out = aggregate(vec![
            HookContribution {
                updated_input: Some(serde_json::json!({"v": 1})),
                ..HookContribution::inert()
            },
            HookContribution {
                updated_input: Some(serde_json::json!({"v": 2})),
                ..HookContribution::inert()
            },
        ]);
        assert_eq!(out.updated_input.unwrap()["v"], 2);
    }
}
