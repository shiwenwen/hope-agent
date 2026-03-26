use serde_json::Value;

mod agents;
mod approval;
mod apply_patch;
pub(crate) mod browser;
pub(crate) mod canvas;
mod cron;
mod definitions;
mod edit;
mod exec;
mod execution;
mod find;
mod grep;
mod image;
pub(crate) mod image_generate;
mod ls;
mod memory;
mod notification;
mod pdf;
mod process;
pub(crate) mod read;
mod sessions;
pub(crate) mod acp_spawn;
pub(crate) mod subagent;
pub(crate) mod web_fetch;
pub(crate) mod web_search;
mod write;
mod plan_step;

// ── Public Re-exports ─────────────────────────────────────────────

pub use approval::{ApprovalResponse, ToolPermissionMode, submit_approval_response, set_tool_permission_mode, get_tool_permission_mode};
pub use definitions::{get_available_tools, get_subagent_tool, get_notification_tool, get_image_generate_tool, get_image_generate_tool_dynamic, get_canvas_tool, get_acp_spawn_tool, get_tools_for_provider, is_internal_tool, get_plan_step_tool};
pub use execution::{ToolExecContext, execute_tool_with_context};

// ── Tool Name Constants ──────────────────────────────────────────

pub const TOOL_EXEC: &str = "exec";
pub const TOOL_PROCESS: &str = "process";
pub const TOOL_READ: &str = "read";
pub const TOOL_WRITE: &str = "write";
pub const TOOL_EDIT: &str = "edit";
pub const TOOL_LS: &str = "ls";
pub const TOOL_GREP: &str = "grep";
pub const TOOL_FIND: &str = "find";
pub const TOOL_APPLY_PATCH: &str = "apply_patch";
pub const TOOL_WEB_SEARCH: &str = "web_search";
pub const TOOL_WEB_FETCH: &str = "web_fetch";
pub const TOOL_SAVE_MEMORY: &str = "save_memory";
pub const TOOL_RECALL_MEMORY: &str = "recall_memory";
pub const TOOL_UPDATE_MEMORY: &str = "update_memory";
pub const TOOL_DELETE_MEMORY: &str = "delete_memory";
pub const TOOL_UPDATE_CORE_MEMORY: &str = "update_core_memory";
pub const TOOL_MANAGE_CRON: &str = "manage_cron";
pub const TOOL_BROWSER: &str = "browser";
pub const TOOL_SEND_NOTIFICATION: &str = "send_notification";
pub const TOOL_SUBAGENT: &str = "subagent";
pub const TOOL_MEMORY_GET: &str = "memory_get";
pub const TOOL_AGENTS_LIST: &str = "agents_list";
pub const TOOL_SESSIONS_LIST: &str = "sessions_list";
pub const TOOL_SESSION_STATUS: &str = "session_status";
pub const TOOL_SESSIONS_HISTORY: &str = "sessions_history";
pub const TOOL_SESSIONS_SEND: &str = "sessions_send";
pub const TOOL_IMAGE: &str = "image";
pub const TOOL_IMAGE_GENERATE: &str = "image_generate";
pub const TOOL_PDF: &str = "pdf";
pub const TOOL_CANVAS: &str = "canvas";
pub const TOOL_ACP_SPAWN: &str = "acp_spawn";
pub const TOOL_UPDATE_PLAN_STEP: &str = "update_plan_step";

// ── Shared Helpers ────────────────────────────────────────────────

/// Extract a string value from a Value that might be a plain string, `{type:"text", text:"..."}`,
/// or an array of such objects (e.g. `[{type:"text", text:"..."}]`).
pub(crate) fn extract_string_param(val: &Value) -> Option<&str> {
    // Plain string
    if let Some(s) = val.as_str() {
        return Some(s);
    }
    // Structured content: {type: "text", text: "..."}
    if let Some(obj) = val.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("text") {
            return obj.get("text").and_then(|v| v.as_str());
        }
    }
    // Array of structured content: [{type: "text", text: "..."}]
    if let Some(arr) = val.as_array() {
        if let Some(first) = arr.first() {
            return extract_string_param(first);
        }
    }
    None
}

/// Expand ~ and ~/ to home directory.
pub(crate) fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return if path == "~" {
                home.to_string_lossy().to_string()
            } else {
                home.join(&path[2..]).to_string_lossy().to_string()
            };
        }
    }
    path.to_string()
}

// ── Provider Enum ─────────────────────────────────────────────────

/// Supported LLM provider types for tool schema adaptation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Anthropic,
    OpenAI,
}
