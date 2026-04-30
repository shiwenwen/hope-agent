//! Decision engine — single entry point that consumes all rule layers and
//! returns a final [`super::Decision`].
//!
//! Priority (high → low):
//! 1. Plan Mode — overrides everything (even YOLO).
//! 2. YOLO (global / session) — bypasses approvals, but emits `app_warn!`
//!    audit logs for protected-path / dangerous-command hits.
//! 3. Protected paths / dangerous commands — strict ask, no AllowAlways.
//! 4. AllowAlways accumulators (project / session / agent_home / global).
//! 5. Session mode preset:
//!    - Default → hardcoded edit-class + edit-command match + agent
//!      `custom_approval_tools` extras
//!    - Smart  → `_confidence` self-tag or `judge_model`
//! 6. Default fallback — Allow.

use serde_json::Value;

use super::mode::SessionMode;
use super::rules::extract_path_arg;
use super::{AskReason, Decision};

/// Context passed to [`resolve`] for a tool call. Decoupled from
/// `ToolExecContext` so the engine has a stable, narrow contract.
#[derive(Debug)]
pub struct ResolveContext<'a> {
    /// The tool name being invoked.
    pub tool_name: &'a str,
    /// The tool_call args JSON.
    pub args: &'a Value,
    /// Per-session permission mode.
    pub session_mode: SessionMode,
    /// `true` if global YOLO is enabled in `AppConfig.permission.global_yolo`.
    pub global_yolo: bool,
    /// `true` if the session is currently in Plan Mode.
    pub plan_mode: bool,
    /// Plan mode's whitelist of allowed tools (only consumed when `plan_mode`).
    pub plan_mode_allowed_tools: &'a [String],
    /// Agent-level "custom tool approval" toggle.
    pub agent_custom_approval_enabled: bool,
    /// Agent-level list of tool names to require approval for (Default mode only).
    pub agent_custom_approval_tools: &'a [String],
    /// Optional session ID used for in-memory session-scoped allowlist lookup.
    pub session_id: Option<&'a str>,
    /// Optional project ID used for project-scoped allowlist lookup.
    pub project_id: Option<&'a str>,
    /// Optional agent ID used for agent_home-scoped allowlist lookup.
    pub agent_id: Option<&'a str>,
    /// `true` if the tool is internal (per `ToolDefinition.internal`); these
    /// always bypass approval regardless of mode.
    pub is_internal_tool: bool,
}

/// Hardcoded edit-class tool names — these always require approval in
/// Default mode. Memoized as a slice rather than a HashSet for cheap matches.
const EDIT_TOOLS: &[&str] = &["write", "edit", "apply_patch"];

fn is_edit_tool(name: &str) -> bool {
    EDIT_TOOLS.contains(&name)
}

/// The single entry point. Returns a final [`Decision`] for one tool call.
pub fn resolve(ctx: &ResolveContext<'_>) -> Decision {
    if ctx.plan_mode {
        let allowed = ctx
            .plan_mode_allowed_tools
            .iter()
            .any(|t| t == ctx.tool_name);
        if !allowed {
            return Decision::Deny {
                reason: format!(
                    "Plan Mode active — tool '{}' is not in the allowed list",
                    ctx.tool_name
                ),
            };
        }
        return Decision::Allow;
    }

    // Internal tools are framework helpers that the LLM uses to introspect
    // or coordinate; they never touch external IO and are exempt from
    // approval gates regardless of mode.
    if ctx.is_internal_tool {
        return Decision::Allow;
    }

    let yolo = ctx.global_yolo || ctx.session_mode == SessionMode::Yolo;
    if yolo {
        if let Some(reason) = check_protected_path(ctx) {
            log_yolo_warn(ctx, &reason);
        }
        if let Some(reason) = check_dangerous_command(ctx) {
            log_yolo_warn(ctx, &reason);
        }
        return Decision::Allow;
    }

    if let Some(reason) = check_protected_path(ctx) {
        return Decision::Ask { reason };
    }
    if let Some(reason) = check_dangerous_command(ctx) {
        return Decision::Ask { reason };
    }

    // AllowAlways file-backed scopes (project / session / agent_home / global)
    // will be queried here once the GUI editor lands.

    match ctx.session_mode {
        SessionMode::Default => resolve_default_mode(ctx),
        SessionMode::Smart => resolve_smart_mode(ctx),
        // Defensive: YOLO is short-circuited above, but if a future refactor
        // skips that branch we must not panic in production — fall through
        // to Allow with a warn so the regression is visible in logs.
        SessionMode::Yolo => {
            app_warn!(
                "permission",
                "engine",
                "Reached fallthrough match arm for SessionMode::Yolo (tool '{}'); \
                 expected the YOLO short-circuit above to handle this — please report.",
                ctx.tool_name
            );
            Decision::Allow
        }
    }
}

fn resolve_default_mode(ctx: &ResolveContext<'_>) -> Decision {
    // Hardcoded edit-class tools.
    if is_edit_tool(ctx.tool_name) {
        return Decision::Ask {
            reason: AskReason::EditTool,
        };
    }

    // exec edit-command pattern match.
    if ctx.tool_name == "exec" {
        if let Some(reason) = check_edit_command(ctx) {
            return Decision::Ask { reason };
        }
    }

    // Agent custom approval list (additive).
    if ctx.agent_custom_approval_enabled
        && ctx
            .agent_custom_approval_tools
            .iter()
            .any(|t| t == ctx.tool_name)
    {
        return Decision::Ask {
            reason: AskReason::AgentCustomList,
        };
    }

    Decision::Allow
}

fn resolve_smart_mode(_ctx: &ResolveContext<'_>) -> Decision {
    // self-confidence and judge-model branches will land here; until then
    // Smart mode is a permissive placeholder.
    Decision::Allow
}

fn check_protected_path(ctx: &ResolveContext<'_>) -> Option<AskReason> {
    let path = extract_path_arg(ctx.tool_name, ctx.args)?;
    let matched = super::protected_paths::matches(&path, super::protected_paths::current_patterns())?;
    Some(AskReason::ProtectedPath {
        matched_path: matched.to_string(),
    })
}

fn check_dangerous_command(ctx: &ResolveContext<'_>) -> Option<AskReason> {
    if ctx.tool_name != "exec" {
        return None;
    }
    let cmd = ctx.args.get("command").and_then(|v| v.as_str())?;
    let matched = super::dangerous_commands::matches(cmd, super::dangerous_commands::current_patterns())?;
    Some(AskReason::DangerousCommand {
        matched_pattern: matched.to_string(),
    })
}

fn check_edit_command(ctx: &ResolveContext<'_>) -> Option<AskReason> {
    let cmd = ctx.args.get("command").and_then(|v| v.as_str())?;
    let matched = super::edit_commands::matches(cmd, super::edit_commands::current_patterns())?;
    Some(AskReason::EditCommand {
        matched_pattern: matched.to_string(),
    })
}

fn log_yolo_warn(ctx: &ResolveContext<'_>, reason: &AskReason) {
    use AskReason::*;
    let detail = match reason {
        ProtectedPath { matched_path } => format!("protected path '{matched_path}'"),
        DangerousCommand { matched_pattern } => format!("dangerous command '{matched_pattern}'"),
        EditCommand { matched_pattern } => format!("edit command '{matched_pattern}'"),
        EditTool => "edit-class tool".to_string(),
        AgentCustomList => "agent custom approval".to_string(),
        SmartJudge { rationale } => format!("smart judge: {rationale}"),
    };
    app_warn!(
        "permission",
        "yolo_bypass",
        "YOLO mode bypassed approval for tool '{}' ({})",
        ctx.tool_name,
        detail
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(
        tool: &'a str,
        args: &'a Value,
        mode: SessionMode,
        plan_tools: &'a Vec<String>,
        custom_tools: &'a Vec<String>,
    ) -> ResolveContext<'a> {
        ResolveContext {
            tool_name: tool,
            args,
            session_mode: mode,
            global_yolo: false,
            plan_mode: false,
            plan_mode_allowed_tools: plan_tools,
            agent_custom_approval_enabled: false,
            agent_custom_approval_tools: custom_tools,
            session_id: None,
            project_id: None,
            agent_id: None,
            is_internal_tool: false,
        }
    }

    #[test]
    fn write_tool_default_asks() {
        let args = json!({"path": "/tmp/foo"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let c = ctx("write", &args, SessionMode::Default, &plan, &custom);
        assert!(matches!(
            resolve(&c),
            Decision::Ask {
                reason: AskReason::EditTool
            }
        ));
    }

    #[test]
    fn read_tool_default_allows() {
        let args = json!({"path": "/tmp/foo"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let c = ctx("read", &args, SessionMode::Default, &plan, &custom);
        assert_eq!(resolve(&c), Decision::Allow);
    }

    #[test]
    fn yolo_overrides_edit_tool() {
        let args = json!({"path": "/tmp/foo"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let mut c = ctx("write", &args, SessionMode::Yolo, &plan, &custom);
        c.global_yolo = false;
        assert_eq!(resolve(&c), Decision::Allow);
    }

    #[test]
    fn plan_mode_denies_unlisted_tool() {
        let args = json!({});
        let plan: Vec<String> = vec!["read".into(), "submit_plan".into()];
        let custom: Vec<String> = vec![];
        let mut c = ctx("write", &args, SessionMode::Default, &plan, &custom);
        c.plan_mode = true;
        assert!(matches!(resolve(&c), Decision::Deny { .. }));
    }

    #[test]
    fn plan_mode_allows_listed_tool() {
        let args = json!({});
        let plan: Vec<String> = vec!["read".into()];
        let custom: Vec<String> = vec![];
        let mut c = ctx("read", &args, SessionMode::Default, &plan, &custom);
        c.plan_mode = true;
        assert_eq!(resolve(&c), Decision::Allow);
    }

    #[test]
    fn plan_overrides_yolo() {
        let args = json!({});
        let plan: Vec<String> = vec!["read".into()];
        let custom: Vec<String> = vec![];
        let mut c = ctx("write", &args, SessionMode::Yolo, &plan, &custom);
        c.plan_mode = true;
        c.global_yolo = true;
        assert!(matches!(resolve(&c), Decision::Deny { .. }));
    }

    #[test]
    fn dangerous_command_strict_ask() {
        let args = json!({"command": "rm -rf /"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let c = ctx("exec", &args, SessionMode::Default, &plan, &custom);
        assert!(matches!(
            resolve(&c),
            Decision::Ask {
                reason: AskReason::DangerousCommand { .. }
            }
        ));
    }

    #[test]
    fn edit_command_asks_in_default() {
        let args = json!({"command": "rm foo.txt"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let c = ctx("exec", &args, SessionMode::Default, &plan, &custom);
        assert!(matches!(
            resolve(&c),
            Decision::Ask {
                reason: AskReason::EditCommand { .. }
            }
        ));
    }

    #[test]
    fn safe_command_default_allows() {
        let args = json!({"command": "ls -la"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let c = ctx("exec", &args, SessionMode::Default, &plan, &custom);
        assert_eq!(resolve(&c), Decision::Allow);
    }

    #[test]
    fn agent_custom_approval_adds_tool() {
        let args = json!({});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec!["browser".into()];
        let mut c = ctx("browser", &args, SessionMode::Default, &plan, &custom);
        c.agent_custom_approval_enabled = true;
        assert!(matches!(
            resolve(&c),
            Decision::Ask {
                reason: AskReason::AgentCustomList
            }
        ));
    }

    #[test]
    fn agent_custom_approval_inactive_when_flag_off() {
        let args = json!({});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec!["browser".into()];
        let c = ctx("browser", &args, SessionMode::Default, &plan, &custom);
        // enable flag is false → list is ignored
        assert_eq!(resolve(&c), Decision::Allow);
    }

    #[test]
    fn smart_mode_inactive_for_custom_list() {
        // Smart mode ignores custom_approval_tools per design.
        let args = json!({});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec!["browser".into()];
        let mut c = ctx("browser", &args, SessionMode::Smart, &plan, &custom);
        c.agent_custom_approval_enabled = true;
        // Phase 4 will tighten this; for now Smart returns Allow as placeholder.
        assert_eq!(resolve(&c), Decision::Allow);
    }

    #[test]
    fn protected_path_strict_ask() {
        let args = json!({"path": "~/.ssh/id_rsa"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let c = ctx("read", &args, SessionMode::Default, &plan, &custom);
        match resolve(&c) {
            Decision::Ask {
                reason: AskReason::ProtectedPath { .. },
            } => {}
            other => panic!("expected ProtectedPath ask, got {:?}", other),
        }
    }

    #[test]
    fn internal_tools_skip_all_gates() {
        let args = json!({"path": "/tmp/foo"});
        let plan: Vec<String> = vec![];
        let custom: Vec<String> = vec![];
        let mut c = ctx("write", &args, SessionMode::Default, &plan, &custom);
        c.is_internal_tool = true;
        assert_eq!(resolve(&c), Decision::Allow);
    }
}
