//! `if` condition evaluation for hook handlers (design §6.4).
//!
//! A handler's `if` rule — e.g. `exec(rm *)`, `write(src/**)`,
//! `web_fetch(*.github.com)` — gates execution on the tool call's name and
//! arguments, reusing the permission engine's argument extractors + glob
//! matcher. Only tool-lifecycle events (`PreToolUse` / `PostToolUse` /
//! `PostToolUseFailure`) carry a tool name + args; for any other event a
//! `ToolName(...)` rule can't match, so the handler is skipped (fail-safe).

use crate::permission::rules::{
    extract_command_arg, extract_domain_arg, extract_path_arg, glob_match_simple,
};

use super::types::HookInput;

/// Normalize a rule's tool name to Hope Agent's internal tool names, so users
/// can paste Claude Code-style rules (`Bash(...)`, `Write(...)`) unchanged.
fn normalize_tool(tool: &str) -> &str {
    match tool {
        "Bash" | "bash" | "Shell" | "shell" => "exec",
        "Write" => "write",
        "Edit" => "edit",
        "Read" => "read",
        "WebFetch" => "web_fetch",
        other => other,
    }
}

/// Parse `"ToolName(pattern)"` into `(normalized_tool, pattern)`. `None` when the
/// string isn't in that shape — the caller treats that as "no usable rule" and
/// skips the handler (fail-safe).
fn parse_if_rule(rule: &str) -> Option<(&str, &str)> {
    let rule = rule.trim();
    let inner = rule.strip_suffix(')')?;
    let open = inner.find('(')?;
    let tool = inner[..open].trim();
    if tool.is_empty() {
        return None;
    }
    Some((normalize_tool(tool), &inner[open + 1..]))
}

/// Whether `input` satisfies the handler's `if` rule. Non-tool events,
/// unparseable rules, and tool-name mismatches all return `false` (the handler
/// is then skipped).
pub fn if_matches(rule: &str, input: &HookInput) -> bool {
    let Some((tool, pattern)) = parse_if_rule(rule) else {
        return false;
    };
    let Some(actual_tool) = input.tool_name() else {
        return false; // non-tool event can't satisfy a ToolName(...) rule
    };
    if actual_tool != tool {
        return false;
    }
    let Some(args) = input.tool_input() else {
        return false;
    };
    // Pick the argument to glob-match against, by tool family (mirrors the
    // permission engine's extractors). `apply_patch` / unknown tools fall back
    // to the whole args JSON since their target isn't a single path/command.
    let target = match tool {
        "exec" | "process" => extract_command_arg(args),
        "read" | "write" | "edit" | "ls" | "grep" | "find" => {
            extract_path_arg(tool, args).map(|p| p.to_string_lossy().into_owned())
        }
        // Feishu drive upload/download touch local paths — same family as
        // read/write, mirrored from `permission::rules::extract_path_arg`.
        n if n == crate::tools::feishu::TOOL_DRIVE_UPLOAD_MEDIA
            || n == crate::tools::feishu::TOOL_DRIVE_DOWNLOAD_MEDIA =>
        {
            extract_path_arg(tool, args).map(|p| p.to_string_lossy().into_owned())
        }
        "web_fetch" | "browser" => extract_domain_arg(args),
        _ => Some(args.to_string()),
    };
    target.is_some_and(|t| glob_match_simple(pattern, &t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{CommonHookInput, PermissionMode};
    use std::path::PathBuf;

    fn common() -> CommonHookInput {
        CommonHookInput {
            session_id: "s1".into(),
            transcript_path: PathBuf::from("/tmp/t.jsonl"),
            cwd: std::env::temp_dir(),
            permission_mode: PermissionMode::Default,
            hook_event_name: "PreToolUse".into(),
            agent_id: None,
            agent_type: None,
        }
    }

    fn pre_tool(tool: &str, args: serde_json::Value) -> HookInput {
        HookInput::PreToolUse {
            common: common(),
            tool_name: tool.into(),
            tool_input: args,
            tool_use_id: "c1".into(),
        }
    }

    #[test]
    fn parse_basic_and_alias() {
        assert_eq!(parse_if_rule("exec(rm *)"), Some(("exec", "rm *")));
        assert_eq!(parse_if_rule("Bash(rm *)"), Some(("exec", "rm *"))); // alias
        assert_eq!(parse_if_rule("Write(src/**)"), Some(("write", "src/**")));
        assert_eq!(parse_if_rule("exec()"), Some(("exec", "")));
        assert_eq!(parse_if_rule("not a rule"), None);
        assert_eq!(parse_if_rule("(x)"), None);
    }

    #[test]
    fn command_glob_matches() {
        let input = pre_tool("exec", serde_json::json!({ "command": "rm -rf /tmp/x" }));
        assert!(if_matches("exec(rm *)", &input));
        assert!(if_matches("Bash(rm *)", &input)); // alias normalizes to exec
        assert!(!if_matches("exec(ls *)", &input));
    }

    #[test]
    fn path_glob_matches() {
        let input = pre_tool("write", serde_json::json!({ "path": "src/main.rs" }));
        assert!(if_matches("write(src/**)", &input));
        assert!(!if_matches("write(docs/**)", &input));
    }

    #[test]
    fn tool_mismatch_skips() {
        // rule targets `write` but the call is `read` → skip.
        let input = pre_tool("read", serde_json::json!({ "path": "src/x" }));
        assert!(!if_matches("write(src/**)", &input));
    }

    #[test]
    fn non_tool_event_never_matches() {
        let input = HookInput::Notification {
            common: common(),
            notification_type: "idle_prompt".into(),
            message: "hi".into(),
            title: None,
        };
        assert!(!if_matches("exec(rm *)", &input));
    }

    #[test]
    fn unparseable_rule_skips() {
        let input = pre_tool("exec", serde_json::json!({ "command": "rm x" }));
        assert!(!if_matches("garbage", &input));
    }
}
