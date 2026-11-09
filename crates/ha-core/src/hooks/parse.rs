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

/// Compose a machine-readable MCP hook payload without discarding a legacy
/// verdict for supplemental metadata. If both channels contain protocol data,
/// preserve the stricter event-specific decision (continue:false wins).
pub fn mcp_hook_stdout(structured: &serde_json::Value, legacy: &str, event: HookEvent) -> String {
    let Some(map) = structured.as_object() else {
        return legacy.to_string();
    };
    // An empty protocol object means normal permission flow, not an explicit
    // approval. It must also veto a second channel's prompt-skipping allow.
    // Recognize the declared root namespace, including optional null fields
    // and empty/nested no-op outputs. Deserialization below checks types;
    // unknown metadata objects must not impersonate a protocol response.
    let known = map.is_empty()
        || map.keys().any(|key| {
            matches!(
                key.as_str(),
                "continue"
                    | "stopReason"
                    | "suppressOutput"
                    | "systemMessage"
                    | "terminalSequence"
                    | "decision"
                    | "reason"
                    | "hookSpecificOutput"
            )
        });
    let Ok(output) = serde_json::from_value::<HookOutput>(structured.clone()) else {
        return legacy.to_string();
    };
    if !known {
        return legacy.to_string();
    }
    let previous_output = serde_json::from_str::<HookOutput>(legacy).ok();
    // SDK dual results repeat the same protocol value; do not double contexts.
    if serde_json::from_str::<serde_json::Value>(legacy)
        .ok()
        .as_ref()
        == Some(structured)
    {
        return structured.to_string();
    }
    let candidate = contribution_from_output(output, event);
    let legacy_protocol = previous_output.is_some();
    let previous = previous_output
        .map(|output| contribution_from_output(output, event))
        .unwrap_or_else(|| parse_stdout(legacy, event));
    if !legacy_protocol && previous.additional_context.is_none() {
        return structured.to_string();
    }
    // An implicit Allow preserves normal approval, unlike explicit permission
    // allow. Two protocol channels must both opt in before a prompt is skipped.
    let permission_allow =
        candidate.permission_allow && (!legacy_protocol || previous.permission_allow);
    let updated_input = previous
        .updated_input
        .clone()
        .or_else(|| candidate.updated_input.clone());
    let updated_output = previous
        .updated_mcp_output
        .clone()
        .or_else(|| candidate.updated_mcp_output.clone());
    let mut merged = super::decision::aggregate(vec![previous, candidate]);
    merged.permission_allow = permission_allow;
    merged.updated_input = updated_input;
    merged.updated_mcp_output = updated_output;
    let mut wire = serde_json::json!({
        "continue": merged.continue_execution,
        "suppressOutput": merged.suppress_output,
        "stopReason": merged.stop_reason,
        "systemMessage": merged.system_message,
        "hookSpecificOutput": {
            "additionalContext": merged.merged_additional_context(),
            "sessionTitle": merged.session_title,
            "updatedInput": merged.updated_input,
            "updatedToolOutput": merged.updated_mcp_output,
            "retry": merged.retry,
            "worktreePath": merged.worktree_path
        }
    });
    match merged.decision {
        HookDecision::Deny { reason } => {
            wire["decision"] = serde_json::json!("deny");
            wire["reason"] = serde_json::json!(reason);
        }
        HookDecision::Block { reason } => {
            wire["decision"] = serde_json::json!("block");
            wire["reason"] = serde_json::json!(reason);
        }
        HookDecision::Ask => wire["decision"] = serde_json::json!("ask"),
        HookDecision::Defer => wire["decision"] = serde_json::json!("defer"),
        HookDecision::Allow if merged.permission_allow => match event {
            HookEvent::PreToolUse => {
                wire["hookSpecificOutput"]["permissionDecision"] = serde_json::json!("allow");
            }
            HookEvent::PermissionRequest => {
                wire["hookSpecificOutput"]["decision"] = serde_json::json!({"behavior": "allow"});
            }
            _ => {}
        },
        HookDecision::Allow => {}
    }
    wire.to_string()
}

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
            // Plaintext mode: SessionStart / UserPromptSubmit / UserPromptExpansion
            // treat raw stdout as additionalContext (official). Others ignore it.
            if matches!(
                event,
                HookEvent::SessionStart
                    | HookEvent::UserPromptSubmit
                    | HookEvent::UserPromptExpansion
            ) {
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
        // `defer` → defer to the normal permission flow. Downstream
        // (`execution.rs`) maps `Defer` to a manual prompt like `Ask`; producing
        // `Defer` here (rather than silently falling through to `Allow`) is what
        // makes the official `defer` value actually take effect.
        Some("defer") => HookDecision::Defer,
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
            Some("defer") => HookDecision::Defer,
            Some("allow") => {
                permission_allow = true;
                HookDecision::Allow
            }
            _ => top_level_decision(),
        }
    } else if matches!(event, HookEvent::PermissionRequest) {
        // PermissionRequest uses the official structured
        // `hookSpecificOutput.decision.behavior` (allow | deny), taking
        // precedence over any top-level `decision`.
        match hso.decision.as_ref().and_then(|d| d.behavior.as_deref()) {
            Some("deny") => HookDecision::Deny {
                reason: out.reason.clone().unwrap_or_default(),
            },
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
        suppress_output: out.suppress_output,
        stop_reason: out.stop_reason,
        system_message: out.system_message,
        additional_context: hso.additional_context,
        session_title: hso.session_title,
        updated_input: hso.updated_input,
        // Official `updatedToolOutput` (PostToolUse) — replaces the tool result.
        updated_mcp_output: hso.updated_tool_output,
        // Official `retry` (PermissionDenied) — model may retry the denied call.
        retry: hso.retry.unwrap_or(false),
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
    fn mcp_dual_results_use_event_specific_decisions() {
        let legacy = r#"{"decision":"block","reason":"legacy veto"}"#;
        let structured = serde_json::json!({
            "hookSpecificOutput": {"permissionDecision": "deny"}
        });
        let parse_mcp = |structured: &serde_json::Value, legacy: &str, event| {
            parse(
                &raw(Some(0), &mcp_hook_stdout(structured, legacy, event), ""),
                event,
            )
        };
        assert!(matches!(
            parse_mcp(&structured, legacy, HookEvent::PreToolUse).decision,
            HookDecision::Deny { .. }
        ));
        assert_eq!(
            parse_mcp(&structured, legacy, HookEvent::PermissionRequest).decision,
            HookDecision::Block {
                reason: "legacy veto".into()
            }
        );
        let stop = r#"{"continue":false,"stopReason":"stop"}"#;
        assert!(!parse_mcp(&structured, stop, HookEvent::PreToolUse).continue_execution);
        let block = serde_json::json!({"decision":"block"});
        assert!(matches!(
            parse_mcp(&block, "{}", HookEvent::PreToolUse).decision,
            HookDecision::Block { .. }
        ));
    }

    #[test]
    fn mcp_dual_results_preserve_normal_approval_context_and_rewrites() {
        for (event, allow) in [
            (
                HookEvent::PreToolUse,
                serde_json::json!({"hookSpecificOutput":{"permissionDecision":"allow"}}),
            ),
            (
                HookEvent::PermissionRequest,
                serde_json::json!({"hookSpecificOutput":{"decision":{"behavior":"allow"}}}),
            ),
        ] {
            for normal in [
                serde_json::json!({}),
                serde_json::json!({"hookSpecificOutput":{"additionalContext":"context"}}),
                serde_json::json!({"hookSpecificOutput":{"updatedInput":{"safe":true}}}),
                serde_json::json!({"hookSpecificOutput": {}}),
                serde_json::json!({"hookSpecificOutput": null}),
                serde_json::json!({"hookSpecificOutput": {"permissionDecision": null, "additionalContext": null, "updatedInput": null}}),
                serde_json::json!({"decision": null, "stopReason": null, "reason": null, "terminalSequence": null, "systemMessage": null}),
                serde_json::json!({"hookSpecificOutput": {"risk": "unknown"}}),
            ] {
                for (legacy, structured) in [(&normal, &allow), (&allow, &normal)] {
                    let stdout = mcp_hook_stdout(structured, &legacy.to_string(), event);
                    let result = parse(&raw(Some(0), &stdout, ""), event);
                    assert_eq!(result.decision, HookDecision::Allow);
                    assert!(!result.permission_allow);
                    assert_eq!(
                        result.updated_input,
                        normal["hookSpecificOutput"]
                            .get("updatedInput")
                            .filter(|value| !value.is_null())
                            .cloned()
                    );
                    assert_eq!(
                        result.additional_context.as_deref(),
                        normal["hookSpecificOutput"]["additionalContext"].as_str()
                    );
                }
            }
        }
        let context = serde_json::json!({"hookSpecificOutput":{"additionalContext":"once"}});
        assert_eq!(
            mcp_hook_stdout(&context, &context.to_string(), HookEvent::SessionStart),
            context.to_string()
        );
        let legacy = r#"{"hookSpecificOutput":{"updatedInput":{"command":"safe"}}}"#;
        let structured = serde_json::json!({"hookSpecificOutput":{"permissionDecision":"ask","updatedInput":{"command":"other"}}});
        let stdout = mcp_hook_stdout(&structured, legacy, HookEvent::PreToolUse);
        let result = parse(&raw(Some(0), &stdout, ""), HookEvent::PreToolUse);
        assert_eq!(result.decision, HookDecision::Ask);
        assert_eq!(
            result.updated_input,
            Some(serde_json::json!({"command":"safe"}))
        );
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

    #[test]
    fn pretooluse_permission_decision_defer() {
        // `defer` must produce a Defer decision (not silently fall through to
        // Allow) so the official value actually defers to the permission flow.
        let c = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"permissionDecision":"defer"}}"#,
                "",
            ),
            HookEvent::PreToolUse,
        );
        assert_eq!(c.decision, HookDecision::Defer);
        assert!(!c.permission_allow);
        // Top-level `decision:"defer"` on a non-PreToolUse event also defers.
        let c2 = parse(
            &raw(Some(0), r#"{"decision":"defer"}"#, ""),
            HookEvent::Stop,
        );
        assert_eq!(c2.decision, HookDecision::Defer);
    }

    #[test]
    fn updated_tool_output_parsed_into_mcp_output() {
        // Official `updatedToolOutput` (PostToolUse) is parsed (was a dead
        // hard-coded None before).
        let c = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"updatedToolOutput":"REDACTED"}}"#,
                "",
            ),
            HookEvent::PostToolUse,
        );
        assert_eq!(
            c.updated_mcp_output,
            Some(serde_json::Value::String("REDACTED".into()))
        );
    }

    #[test]
    fn retry_parsed_for_permission_denied() {
        // Official `retry` (PermissionDenied) is parsed (was a dead hard-coded
        // false before).
        let c = parse(
            &raw(Some(0), r#"{"hookSpecificOutput":{"retry":true}}"#, ""),
            HookEvent::PermissionDenied,
        );
        assert!(c.retry);
    }

    #[test]
    fn suppress_output_propagated() {
        let c = parse(
            &raw(Some(0), r#"{"suppressOutput":true}"#, ""),
            HookEvent::PostToolUse,
        );
        assert!(c.suppress_output);
    }

    #[test]
    fn permission_request_decision_behavior() {
        // Official structured `decision.behavior` drives a PermissionRequest
        // verdict.
        let deny = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"decision":{"behavior":"deny"}},"reason":"no"}"#,
                "",
            ),
            HookEvent::PermissionRequest,
        );
        assert_eq!(
            deny.decision,
            HookDecision::Deny {
                reason: "no".into()
            }
        );
        let allow = parse(
            &raw(
                Some(0),
                r#"{"hookSpecificOutput":{"decision":{"behavior":"allow"}}}"#,
                "",
            ),
            HookEvent::PermissionRequest,
        );
        assert!(allow.permission_allow);
    }

    #[test]
    fn plaintext_context_includes_user_prompt_expansion() {
        // Official: UserPromptExpansion (like SessionStart/UserPromptSubmit)
        // treats non-JSON stdout as additionalContext.
        let c = parse(
            &raw(Some(0), "expanded ctx", ""),
            HookEvent::UserPromptExpansion,
        );
        assert_eq!(c.additional_context.as_deref(), Some("expanded ctx"));
    }
}
