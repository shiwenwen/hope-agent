use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

use super::{
    acp_spawn, browser, cron, memory, notification, subagent, weather, web_fetch, web_search,
};
use super::{
    agents, amend_plan, canvas, image, image_generate, pdf, plan_question, plan_step, sessions,
    submit_plan,
};
use super::{apply_patch, edit, exec, find, grep, ls, process, read, write};
use super::{
    approval, TOOL_ACP_SPAWN, TOOL_AGENTS_LIST, TOOL_AMEND_PLAN, TOOL_APPLY_PATCH, TOOL_BROWSER,
    TOOL_CANVAS, TOOL_DELETE_MEMORY, TOOL_EDIT, TOOL_EXEC, TOOL_FIND, TOOL_GET_WEATHER, TOOL_GREP,
    TOOL_IMAGE, TOOL_IMAGE_GENERATE, TOOL_LS, TOOL_MANAGE_CRON, TOOL_MEMORY_GET, TOOL_PDF,
    TOOL_PLAN_QUESTION, TOOL_PROCESS, TOOL_READ, TOOL_RECALL_MEMORY, TOOL_SAVE_MEMORY,
    TOOL_SEND_NOTIFICATION, TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST, TOOL_SESSIONS_SEND,
    TOOL_SESSION_STATUS, TOOL_SUBAGENT, TOOL_SUBMIT_PLAN, TOOL_UPDATE_CORE_MEMORY,
    TOOL_UPDATE_MEMORY, TOOL_UPDATE_PLAN_STEP, TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE,
};

/// Load the user-configured tool timeout from config.json. Returns `None`
/// when the user explicitly set 0 (disabled). The serde default in
/// [`ProviderStore`] provides the 300s fallback when the field is missing.
fn tool_timeout() -> Option<Duration> {
    let secs = crate::provider::cached_store().tool_timeout;
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

// ── Tool Execution Context ────────────────────────────────────────

/// Context passed to tool execution for dynamic behavior
#[derive(Debug, Clone)]
pub struct ToolExecContext {
    /// Model context window in tokens (for dynamic output truncation)
    pub context_window_tokens: Option<u32>,
    /// Estimated tokens currently used by system prompt + messages + max_output.
    /// Used by the read tool to compute remaining context budget for adaptive sizing.
    pub used_tokens: Option<u32>,
    /// Agent home directory — used as default cwd/path for tools.
    /// Falls back to user ~ if None.
    pub home_dir: Option<String>,
    /// Current session ID (for sub-agent spawning context)
    pub session_id: Option<String>,
    /// Current agent ID
    pub agent_id: Option<String>,
    /// Sub-agent nesting depth (0 = top-level)
    pub subagent_depth: u32,
    /// Tool names that require user approval before execution.
    /// `["*"]` means all tools require approval.
    pub require_approval: Vec<String>,
    /// Whether the agent forces Docker sandbox mode for all exec commands.
    pub force_sandbox: bool,
    /// Plan mode file-pattern allow rules: when set, write/edit tools targeting these
    /// glob patterns are allowed even if the tool is in the denied list.
    /// Format: list of glob patterns (e.g. ["~/.opencomputer/plans/*.md"])
    pub plan_mode_allow_paths: Vec<String>,
    /// Plan mode tool whitelist: when non-empty, only these tools can execute.
    /// Enforced at execution layer as defense-in-depth (supplements schema-level filtering).
    pub plan_mode_allowed_tools: Vec<String>,
    /// When true, automatically approve all tool calls (IM channel auto-approve mode).
    pub auto_approve_tools: bool,
}

impl Default for ToolExecContext {
    fn default() -> Self {
        Self {
            context_window_tokens: None,
            used_tokens: None,
            home_dir: None,
            session_id: None,
            agent_id: None,
            subagent_depth: 0,
            require_approval: Vec::new(),
            force_sandbox: false,
            plan_mode_allow_paths: Vec::new(),
            plan_mode_allowed_tools: Vec::new(),
            auto_approve_tools: false,
        }
    }
}

impl ToolExecContext {
    /// Returns the default path for tools: agent home if set, otherwise ".".
    pub fn default_path(&self) -> &str {
        self.home_dir.as_deref().unwrap_or(".")
    }
}

// ── Tool Execution (provider-agnostic) ────────────────────────────

/// Execute a tool by name with the given JSON arguments.
#[allow(dead_code)]
pub async fn execute_tool(name: &str, args: &Value) -> anyhow::Result<String> {
    execute_tool_with_context(name, args, &ToolExecContext::default()).await
}

/// Check if a read tool call targets a SKILL.md file (pre-authorized by skill system).
fn is_skill_read(name: &str, args: &Value) -> bool {
    if name != TOOL_READ {
        return false;
    }
    args.get("path")
        .and_then(|v| v.as_str())
        .map(|p| p.ends_with("/SKILL.md") || p.ends_with("\\SKILL.md"))
        .unwrap_or(false)
}

/// Check if a tool requires approval based on the context's require_approval list.
fn tool_needs_approval(name: &str, args: &Value, ctx: &ToolExecContext) -> bool {
    // Internal capability tools never need approval (flag set on ToolDefinition)
    if super::is_internal_tool(name) {
        return false;
    }
    // Reading SKILL.md files never needs approval — skills are pre-authorized
    if name == TOOL_READ {
        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            if path.ends_with("/SKILL.md") || path.ends_with("\\SKILL.md") {
                return false;
            }
        }
    }
    if ctx.require_approval.is_empty() {
        return false;
    }
    // "*" means all (non-internal) tools require approval
    if ctx.require_approval.iter().any(|t| t == "*") {
        return true;
    }
    ctx.require_approval.iter().any(|t| t == name)
}

/// Execute a tool with additional context (model info, etc.)
pub async fn execute_tool_with_context(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
) -> anyhow::Result<String> {
    let start = std::time::Instant::now();

    // ── Tool-level approval gate ─────────────────────────────────
    // Check session-level permission mode and tool-level approval requirements.
    // Note: exec tool has its own command-level approval inside tool_exec;
    // this is the tool-level gate that applies to ALL approvable tools.
    let perm_mode = approval::get_tool_permission_mode().await;
    let needs_approval = if ctx.auto_approve_tools {
        false
    } else {
        match perm_mode {
            approval::ToolPermissionMode::FullApprove => false,
            approval::ToolPermissionMode::AskEveryTime => {
                // In ask_every_time mode, all non-internal tools need approval
                // (except reading SKILL.md — pre-authorized by skill system)
                !super::is_internal_tool(name) && name != TOOL_EXEC && !is_skill_read(name, args)
            }
            approval::ToolPermissionMode::Auto => {
                tool_needs_approval(name, args, ctx) && name != TOOL_EXEC
            }
        }
    };
    if needs_approval {
        let desc = format!("tool: {} {}", name, {
            let s = args.to_string();
            if s.len() > 200 {
                format!("{}...", crate::truncate_utf8(&s, 200))
            } else {
                s
            }
        });
        let cwd = ctx.home_dir.as_deref().unwrap_or(".");
        match approval::check_and_request_approval(&desc, cwd, ctx.session_id.as_deref()).await {
            Ok(approval::ApprovalResponse::AllowOnce) => {
                app_info!("tool", "approval", "Tool '{}' approved (once)", name);
            }
            Ok(approval::ApprovalResponse::AllowAlways) => {
                if perm_mode == approval::ToolPermissionMode::Auto {
                    app_info!("tool", "approval", "Tool '{}' approved (always)", name);
                    approval::add_to_allowlist(&desc).await;
                } else {
                    app_info!(
                        "tool",
                        "approval",
                        "Tool '{}' approved (ask_every_time)",
                        name
                    );
                }
            }
            Ok(approval::ApprovalResponse::Deny) => {
                return Err(anyhow::anyhow!("Tool '{}' execution denied by user", name));
            }
            Err(e) => {
                app_warn!(
                    "tool",
                    "approval",
                    "Tool approval check failed for '{}' ({}), proceeding",
                    name,
                    e
                );
            }
        }
    }

    // Log tool execution start
    if let Some(logger) = crate::get_logger() {
        let args_preview = {
            let s = args.to_string();
            if s.len() > 500 {
                format!("{}...", crate::truncate_utf8(&s, 500))
            } else {
                s
            }
        };
        logger.log(
            "info",
            "tool",
            &format!("tools::{}", name),
            &format!("Tool '{}' started", name),
            Some(serde_json::json!({"args": args_preview}).to_string()),
            None,
            None,
        );
    }

    // ── Plan Mode path-based permission check ─────────────────────
    // When plan_mode_allow_paths is set, write/edit/apply_patch tools check
    // the target file path and block non-plan-file operations.
    if !ctx.plan_mode_allow_paths.is_empty() {
        let is_path_aware = matches!(name, TOOL_WRITE | TOOL_EDIT | TOOL_APPLY_PATCH);
        if is_path_aware {
            let target_path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !target_path.is_empty() && !crate::plan::is_plan_mode_path_allowed(target_path) {
                return Err(anyhow::anyhow!(
                    "Plan Mode restriction: cannot modify '{}'. During planning, only plan files \
                     (under .opencomputer/plans/) can be edited. Use submit_plan to finalize the plan.",
                    target_path
                ));
            }
        }
    }

    // ── Plan Mode tool whitelist enforcement (defense-in-depth) ────
    // When plan_mode_allowed_tools is set, reject any tool not in the list.
    // This supplements the schema-level filtering done in providers.
    if !ctx.plan_mode_allowed_tools.is_empty()
        && !ctx.plan_mode_allowed_tools.iter().any(|t| t == name)
    {
        return Err(anyhow::anyhow!(
            "Plan Mode restriction: tool '{}' is not allowed during planning. Allowed: {}",
            name,
            ctx.plan_mode_allowed_tools.join(", ")
        ));
    }

    let dispatch = async {
        match name {
            TOOL_EXEC => exec::tool_exec(args, ctx).await,
            TOOL_PROCESS => process::tool_process(args).await,
            TOOL_READ | "read_file" => read::tool_read_file(args, ctx).await,
            TOOL_WRITE | "write_file" => write::tool_write_file(args).await,
            TOOL_EDIT | "patch_file" => edit::tool_edit(args).await,
            TOOL_LS | "list_dir" => ls::tool_ls(args, ctx).await,
            TOOL_GREP => grep::tool_grep(args, ctx).await,
            TOOL_FIND => find::tool_find(args, ctx).await,
            TOOL_APPLY_PATCH => apply_patch::tool_apply_patch(args).await,
            TOOL_WEB_SEARCH => web_search::tool_web_search(args).await,
            TOOL_WEB_FETCH => web_fetch::tool_web_fetch(args).await,
            TOOL_SAVE_MEMORY => memory::tool_save_memory(args).await,
            TOOL_RECALL_MEMORY => memory::tool_recall_memory(args).await,
            TOOL_UPDATE_MEMORY => memory::tool_update_memory(args).await,
            TOOL_DELETE_MEMORY => memory::tool_delete_memory(args).await,
            TOOL_UPDATE_CORE_MEMORY => {
                memory::tool_update_core_memory(args, ctx.agent_id.as_deref().unwrap_or("default"))
                    .await
            }
            TOOL_MANAGE_CRON => cron::tool_manage_cron(args).await,
            TOOL_BROWSER => browser::tool_browser(args).await,
            TOOL_SEND_NOTIFICATION => notification::tool_send_notification(args, ctx).await,
            TOOL_SUBAGENT => subagent::tool_subagent(args, ctx).await,
            TOOL_ACP_SPAWN => acp_spawn::tool_acp_spawn(args, ctx).await,
            TOOL_MEMORY_GET => memory::tool_memory_get(args).await,
            TOOL_AGENTS_LIST => agents::tool_agents_list(args).await,
            TOOL_SESSIONS_LIST => sessions::tool_sessions_list(args).await,
            TOOL_SESSION_STATUS => sessions::tool_session_status(args).await,
            TOOL_SESSIONS_HISTORY => sessions::tool_sessions_history(args).await,
            TOOL_SESSIONS_SEND => Box::pin(sessions::tool_sessions_send(args, ctx)).await,
            TOOL_IMAGE => image::tool_image(args).await,
            TOOL_IMAGE_GENERATE => image_generate::tool_image_generate(args).await,
            TOOL_PDF => pdf::tool_pdf(args).await,
            TOOL_CANVAS => canvas::tool_canvas(args, ctx).await,
            TOOL_GET_WEATHER => weather::tool_get_weather(args).await,
            TOOL_UPDATE_PLAN_STEP => Ok(plan_step::execute(args, ctx.session_id.as_deref()).await),
            TOOL_PLAN_QUESTION => Ok(plan_question::execute(args, ctx.session_id.as_deref()).await),
            TOOL_SUBMIT_PLAN => Ok(submit_plan::execute(args, ctx.session_id.as_deref()).await),
            TOOL_AMEND_PLAN => Ok(amend_plan::execute(args, ctx.session_id.as_deref()).await),
            super::TOOL_TOOL_SEARCH => super::tool_search::tool_search(args).await,
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        }
    };

    let result = if let Some(hard_timeout) = tool_timeout() {
        match timeout(hard_timeout, dispatch).await {
            Ok(inner) => inner,
            Err(_elapsed) => {
                app_error!(
                    "tool",
                    "execution",
                    "Tool '{}' timed out after {}s — forcefully cancelled",
                    name,
                    hard_timeout.as_secs()
                );
                Err(anyhow::anyhow!(
                    "Tool '{}' execution timed out after {}s. The operation was cancelled. \
                     This may be caused by network issues, an unresponsive API, or a slow provider. \
                     Please check your network connection and provider configuration, \
                     or increase toolTimeout in Settings > System.",
                    name, hard_timeout.as_secs()
                ))
            }
        }
    } else {
        // timeout disabled (toolTimeout = 0)
        dispatch.await
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    // Log tool execution result
    if let Some(logger) = crate::get_logger() {
        match &result {
            Ok(output) => {
                let output_preview = if output.len() > 300 {
                    format!("{}...", crate::truncate_utf8(output, 300))
                } else {
                    output.clone()
                };
                logger.log("info", "tool", &format!("tools::{}", name),
                    &format!("Tool '{}' completed in {}ms", name, duration_ms),
                    Some(serde_json::json!({"duration_ms": duration_ms, "output_preview": output_preview}).to_string()),
                    None, None);
            }
            Err(e) => {
                logger.log(
                    "error",
                    "tool",
                    &format!("tools::{}", name),
                    &format!("Tool '{}' failed in {}ms: {}", name, duration_ms, e),
                    Some(
                        serde_json::json!({"duration_ms": duration_ms, "error": e.to_string()})
                            .to_string(),
                    ),
                    None,
                    None,
                );
            }
        }
    }

    // ── Large result disk persistence ────────────────────────────────
    // If the result exceeds the threshold, write it to disk and return
    // a preview with a path reference so the model can `read` the full file.
    match result {
        Ok(output) if output.len() > disk_persist_threshold() => {
            match persist_large_result(&output, ctx.session_id.as_deref(), name) {
                Ok(path) => {
                    let head = crate::truncate_utf8(&output, 2000);
                    // Find a valid UTF-8 char boundary for tail extraction
                    let mut tail_start = output.len().saturating_sub(1000);
                    while tail_start > 0 && !output.is_char_boundary(tail_start) {
                        tail_start += 1;
                    }
                    let tail = &output[tail_start..];
                    let omitted = output.len().saturating_sub(head.len() + tail.len());
                    app_info!(
                        "tool",
                        "disk_persist",
                        "Tool '{}' result {}B persisted to {}",
                        name,
                        output.len(),
                        path
                    );
                    Ok(format!(
                        "{head}\n\n[...{omitted} bytes omitted...]\n\n{tail}\n\n\
                         [Full result ({total}B) saved to: {path}]\n\
                         [Use read tool with this path to access full content]",
                        total = output.len(),
                    ))
                }
                Err(e) => {
                    // Fall back to returning the full result if persistence fails
                    app_warn!(
                        "tool",
                        "disk_persist",
                        "Failed to persist large result for '{}': {}",
                        name,
                        e
                    );
                    Ok(output)
                }
            }
        }
        other => other,
    }
}

// ── Disk Persistence Helpers ─────────────────────────────────────

/// Load the disk persistence threshold from config.json, defaulting to 50KB.
/// Returns 0 to disable (never persist).
fn disk_persist_threshold() -> usize {
    crate::provider::cached_store()
        .tool_result_disk_threshold
        .unwrap_or(50_000)
}

/// Write a large tool result to disk and return the file path.
fn persist_large_result(
    content: &str,
    session_id: Option<&str>,
    tool_name: &str,
) -> anyhow::Result<String> {
    let base_dir = crate::paths::root_dir()?
        .join("tool_results")
        .join(session_id.unwrap_or("_global"));
    std::fs::create_dir_all(&base_dir)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("{tool_name}_{ts}.txt");
    let path = base_dir.join(&filename);
    std::fs::write(&path, content)?;

    Ok(path.to_string_lossy().to_string())
}
