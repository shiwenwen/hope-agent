//! Hook output protocol parsing (design doc §8.3 / §8.5).
//!
//! Turns a [`RawHookResult`] (exit code + stdout/stderr) into a
//! [`HookContribution`] the aggregator consumes:
//! - exit 2 → block (stderr is the reason)
//! - other non-zero / no exit code / timeout → non-blocking error (inert)
//! - exit 0 → parse stdout as JSON per protocol, else plaintext-as-context for
//!   the two events that accept it (`SessionStart` / `UserPromptSubmit`).

use super::decision::HookContribution;
use super::runner::RawHookResult;
use super::types::{HookDecision, HookEvent, HookOutput};

/// Parse one handler's raw result into its decision contribution.
pub fn parse(raw: &RawHookResult, event: HookEvent) -> HookContribution {
    if raw.timed_out {
        return HookContribution::inert();
    }
    match raw.exit_code {
        Some(2) => HookContribution {
            decision: HookDecision::Block {
                reason: raw.stderr.trim().to_string(),
            },
            ..HookContribution::inert()
        },
        Some(0) => parse_stdout(&raw.stdout, event),
        // exit 1 / other non-zero / `None` (http, no exit) → non-blocking.
        _ => HookContribution::inert(),
    }
}

fn parse_stdout(stdout: &str, event: HookEvent) -> HookContribution {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return HookContribution::inert();
    }
    match serde_json::from_str::<HookOutput>(trimmed) {
        Ok(out) => contribution_from_output(out),
        Err(_) => {
            // Plaintext mode: only SessionStart / UserPromptSubmit treat raw
            // stdout as additionalContext (§8.5). Others ignore it.
            if matches!(event, HookEvent::SessionStart | HookEvent::UserPromptSubmit) {
                HookContribution {
                    additional_context: Some(trimmed.to_string()),
                    ..HookContribution::inert()
                }
            } else {
                HookContribution::inert()
            }
        }
    }
}

fn contribution_from_output(out: HookOutput) -> HookContribution {
    let decision = match out.decision.as_deref() {
        Some("block") => HookDecision::Block {
            reason: out.reason.clone().unwrap_or_default(),
        },
        Some("deny") => HookDecision::Deny {
            reason: out.reason.clone().unwrap_or_default(),
        },
        Some("ask") => HookDecision::Ask,
        _ => HookDecision::Allow,
    };
    let hso = out.hook_specific_output.unwrap_or_default();
    HookContribution {
        decision,
        continue_execution: out.continue_execution,
        stop_reason: out.stop_reason,
        system_message: out.system_message,
        additional_context: hso.additional_context,
        session_title: hso.session_title,
        updated_input: hso.updated_input,
        updated_mcp_output: None,
        retry: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn raw(exit: Option<i32>, stdout: &str, stderr: &str) -> RawHookResult {
        RawHookResult {
            exit_code: exit,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            duration: Duration::ZERO,
            timed_out: false,
        }
    }

    #[test]
    fn exit_two_blocks_with_stderr_reason() {
        let c = parse(
            &raw(Some(2), "", "  blocked: rm -rf  "),
            HookEvent::PreToolUse,
        );
        assert_eq!(
            c.decision,
            HookDecision::Block {
                reason: "blocked: rm -rf".into()
            }
        );
    }

    #[test]
    fn exit_one_is_non_blocking() {
        let c = parse(&raw(Some(1), "ignored", "err"), HookEvent::PostToolUse);
        assert_eq!(c.decision, HookDecision::Allow);
        assert!(c.additional_context.is_none());
    }

    #[test]
    fn timeout_is_inert() {
        let mut r = raw(None, "", "");
        r.timed_out = true;
        let c = parse(&r, HookEvent::PostToolUse);
        assert_eq!(c.decision, HookDecision::Allow);
    }

    #[test]
    fn exit_zero_json_additional_context() {
        let c = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput": {"additionalContext": "ctx"}}"#,
                "",
            ),
            HookEvent::PostToolUse,
        );
        assert_eq!(c.additional_context.as_deref(), Some("ctx"));
        assert_eq!(c.decision, HookDecision::Allow);
    }

    #[test]
    fn exit_zero_top_level_block_decision() {
        let c = parse(
            &raw(Some(0), r#"{"decision": "block", "reason": "no"}"#, ""),
            HookEvent::PostToolUse,
        );
        assert_eq!(
            c.decision,
            HookDecision::Block {
                reason: "no".into()
            }
        );
    }

    #[test]
    fn exit_zero_continue_false() {
        let c = parse(
            &raw(Some(0), r#"{"continue": false, "stopReason": "halt"}"#, ""),
            HookEvent::Stop,
        );
        assert!(!c.continue_execution);
        assert_eq!(c.stop_reason.as_deref(), Some("halt"));
    }

    #[test]
    fn plaintext_only_for_session_start_and_prompt() {
        // SessionStart accepts plaintext stdout as context.
        let c = parse(&raw(Some(0), "just text", ""), HookEvent::SessionStart);
        assert_eq!(c.additional_context.as_deref(), Some("just text"));
        // Other events ignore non-JSON stdout.
        let c2 = parse(&raw(Some(0), "just text", ""), HookEvent::PostToolUse);
        assert!(c2.additional_context.is_none());
    }

    #[test]
    fn empty_stdout_is_inert() {
        let c = parse(&raw(Some(0), "   ", ""), HookEvent::SessionStart);
        assert_eq!(c.decision, HookDecision::Allow);
        assert!(c.additional_context.is_none());
    }
}
