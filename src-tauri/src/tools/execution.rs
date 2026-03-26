use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

use super::{
    approval,
    TOOL_EXEC, TOOL_PROCESS, TOOL_READ, TOOL_WRITE, TOOL_EDIT, TOOL_LS,
    TOOL_GREP, TOOL_FIND, TOOL_APPLY_PATCH, TOOL_WEB_SEARCH, TOOL_WEB_FETCH,
    TOOL_SAVE_MEMORY, TOOL_RECALL_MEMORY, TOOL_UPDATE_MEMORY, TOOL_DELETE_MEMORY,
    TOOL_MANAGE_CRON, TOOL_BROWSER, TOOL_SEND_NOTIFICATION, TOOL_SUBAGENT,
    TOOL_MEMORY_GET, TOOL_AGENTS_LIST, TOOL_SESSIONS_LIST, TOOL_SESSION_STATUS,
    TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_SEND, TOOL_IMAGE, TOOL_IMAGE_GENERATE, TOOL_PDF,
    TOOL_CANVAS, TOOL_ACP_SPAWN,
};
use super::{exec, process, read, write, edit, ls, grep, find, apply_patch};
use super::{web_search, web_fetch, memory, cron, browser, notification, subagent, acp_spawn};
use super::{agents, sessions, image, image_generate, pdf, canvas};

/// Default hard timeout (seconds) for a single tool execution.
/// Acts as a safety net when the inner tool timeout (e.g. reqwest) does not fire
/// in degraded network conditions (stuck TCP, unresponsive proxy, etc.).
/// Configurable via `config.json` → `toolTimeout` (seconds). 0 = disabled.
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Load the user-configured tool timeout from config.json, falling back to the
/// compile-time default. Returns `None` when the user explicitly set 0 (disabled).
fn tool_timeout() -> Option<Duration> {
    let secs = crate::provider::load_store()
        .map(|s| s.tool_timeout)
        .unwrap_or(DEFAULT_TOOL_TIMEOUT_SECS);
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
}

impl Default for ToolExecContext {
    fn default() -> Self {
        Self {
            context_window_tokens: None,
            home_dir: None,
            session_id: None,
            agent_id: None,
            subagent_depth: 0,
            require_approval: Vec::new(),
            force_sandbox: false,
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

/// Check if a tool requires approval based on the context's require_approval list.
fn tool_needs_approval(name: &str, ctx: &ToolExecContext) -> bool {
    // Internal capability tools never need approval (flag set on ToolDefinition)
    if super::is_internal_tool(name) {
        return false;
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
    let needs_approval = match perm_mode {
        approval::ToolPermissionMode::FullApprove => false,
        approval::ToolPermissionMode::AskEveryTime => {
            // In ask_every_time mode, all non-internal tools need approval
            !super::is_internal_tool(name) && name != TOOL_EXEC
        }
        approval::ToolPermissionMode::Auto => {
            tool_needs_approval(name, ctx) && name != TOOL_EXEC
        }
    };
    if needs_approval {
        let desc = format!("tool: {} {}", name, {
            let s = args.to_string();
            if s.len() > 200 { format!("{}...", crate::truncate_utf8(&s, 200)) } else { s }
        });
        let cwd = ctx.home_dir.as_deref().unwrap_or(".");
        match approval::check_and_request_approval(&desc, cwd).await {
            Ok(approval::ApprovalResponse::AllowOnce) => {
                app_info!("tool", "approval", "Tool '{}' approved (once)", name);
            }
            Ok(approval::ApprovalResponse::AllowAlways) => {
                if perm_mode == approval::ToolPermissionMode::Auto {
                    app_info!("tool", "approval", "Tool '{}' approved (always)", name);
                    approval::add_to_allowlist(&desc).await;
                } else {
                    app_info!("tool", "approval", "Tool '{}' approved (ask_every_time)", name);
                }
            }
            Ok(approval::ApprovalResponse::Deny) => {
                return Err(anyhow::anyhow!(
                    "Tool '{}' execution denied by user",
                    name
                ));
            }
            Err(e) => {
                app_warn!("tool", "approval",
                    "Tool approval check failed for '{}' ({}), proceeding",
                    name, e
                );
            }
        }
    }

    // Log tool execution start
    if let Some(logger) = crate::get_logger() {
        let args_preview = {
            let s = args.to_string();
            if s.len() > 500 { format!("{}...", crate::truncate_utf8(&s, 500)) } else { s }
        };
        logger.log("info", "tool", &format!("tools::{}", name),
            &format!("Tool '{}' started", name),
            Some(serde_json::json!({"args": args_preview}).to_string()),
            None, None);
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
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        }
    };

    let result = if let Some(hard_timeout) = tool_timeout() {
        match timeout(hard_timeout, dispatch).await {
            Ok(inner) => inner,
            Err(_elapsed) => {
                app_error!(
                    "tool", "execution",
                    "Tool '{}' timed out after {}s — forcefully cancelled",
                    name, hard_timeout.as_secs()
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
                let output_preview = if output.len() > 300 { format!("{}...", crate::truncate_utf8(output, 300)) } else { output.clone() };
                logger.log("info", "tool", &format!("tools::{}", name),
                    &format!("Tool '{}' completed in {}ms", name, duration_ms),
                    Some(serde_json::json!({"duration_ms": duration_ms, "output_preview": output_preview}).to_string()),
                    None, None);
            }
            Err(e) => {
                logger.log("error", "tool", &format!("tools::{}", name),
                    &format!("Tool '{}' failed in {}ms: {}", name, duration_ms, e),
                    Some(serde_json::json!({"duration_ms": duration_ms, "error": e.to_string()}).to_string()),
                    None, None);
            }
        }
    }

    result
}
