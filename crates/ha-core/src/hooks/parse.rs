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
        Ok(out) => contribution_from_output(out, event),
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

fn contribution_from_output(out: HookOutput, event: HookEvent) -> HookContribution {
    let hso = out.hook_specific_output.unwrap_or_default();
    // `permissionDecision` (allow / deny / ask) is a **PreToolUse-only** field —
    // it must NOT drive a verdict for other events (e.g. a PreCompact hook's
    // permissionDecision must not be read as a compaction block). For PreToolUse
    // it takes precedence over the top-level `decision`; everywhere else only
    // the top-level `decision` (block / deny / ask) applies. `permission_allow`
    // is set ONLY for an explicit PreToolUse `permissionDecision:"allow"` so the
    // tool gate can tell a deliberate auto-approve from the default `Allow` a
    // context-only hook produces — never silently skipping a prompt.
    let top_level_decision = || match out.decision.as_deref() {
        Some("block") => HookDecision::Block {
            reason: out.reason.clone().unwrap_or_default(),
        },
        Some("deny") => HookDecision::Deny {
            reason: out.reason.clone().unwrap_or_default(),
        },
        Some("ask") => HookDecision::Ask,
        _ => HookDecision::Allow,
    };
    let mut permission_allow = false;
    let decision = if matches!(event, HookEvent::PreToolUse) {
        match hso.permission_decision.as_deref() {
            Some("deny") => HookDecision::Deny {
                reason: hso
                    .permission_decision_reason
                    .clone()
                    .or_else(|| out.reason.clone())
                    .unwrap_or_default(),
            },
            Some("ask") => HookDecision::Ask,
            Some("allow") => {
                permission_allow = true;
                HookDecision::Allow
            }
            _ => top_level_decision(),
        }
    } else {
        top_level_decision()
    };
    HookContribution {
        decision,
        permission_allow,
        continue_execution: out.continue_execution,
        stop_reason: out.stop_reason,
        system_message: out.system_message,
        additional_context: hso.additional_context,
        session_title: hso.session_title,
        updated_input: hso.updated_input,
        updated_mcp_output: None,
        retry: false,
        worktree_path: hso.worktree_path,
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

    #[test]
    fn pretooluse_permission_decision_deny() {
        let c = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"blocked path"}}"#,
                "",
            ),
            HookEvent::PreToolUse,
        );
        assert_eq!(
            c.decision,
            HookDecision::Deny {
                reason: "blocked path".into()
            }
        );
        assert!(!c.permission_allow);
    }

    #[test]
    fn pretooluse_permission_decision_ask() {
        let c = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"permissionDecision":"ask"}}"#,
                "",
            ),
            HookEvent::PreToolUse,
        );
        assert_eq!(c.decision, HookDecision::Ask);
        assert!(!c.permission_allow);
    }

    #[test]
    fn permission_decision_is_pretooluse_only() {
        // permissionDecision is a PreToolUse-only field: for any other event it
        // must NOT drive a verdict (a PreCompact deny here would wrongly skip
        // compaction).
        let c = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"permissionDecision":"deny"}}"#,
                "",
            ),
            HookEvent::PreCompact,
        );
        assert_eq!(c.decision, HookDecision::Allow);
        assert!(!c.permission_allow);
        // The top-level `decision` still applies to non-PreToolUse events.
        let c2 = parse(
            &raw(Some(0), r#"{"decision":"block","reason":"stop"}"#, ""),
            HookEvent::PreCompact,
        );
        assert_eq!(
            c2.decision,
            HookDecision::Block {
                reason: "stop".into()
            }
        );
    }

    #[test]
    fn explicit_allow_sets_permission_allow_but_context_only_does_not() {
        // Explicit permissionDecision:"allow" → permission_allow = true.
        let explicit = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#,
                "",
            ),
            HookEvent::PreToolUse,
        );
        assert_eq!(explicit.decision, HookDecision::Allow);
        assert!(explicit.permission_allow);

        // A context-only hook (no decision field) is also Allow, but must NOT
        // set permission_allow — otherwise it would silently skip a prompt.
        let ctx_only = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"additionalContext":"fyi"}}"#,
                "",
            ),
            HookEvent::PreToolUse,
        );
        assert_eq!(ctx_only.decision, HookDecision::Allow);
        assert!(!ctx_only.permission_allow);
    }
}
