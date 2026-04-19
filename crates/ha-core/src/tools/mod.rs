use serde_json::Value;

pub(crate) mod acp_spawn;
mod agents;
mod amend_plan;
mod apply_patch;
pub(crate) mod approval;
mod ask_user_question;
pub(crate) mod browser;
pub mod canvas;
mod cron;
mod definitions;
mod edit;
mod exec;
mod execution;
mod find;
mod grep;
pub(crate) mod image;
pub mod image_generate;
pub(crate) mod job_status;
mod ls;
mod memory;
mod notification;
pub(crate) mod pdf;
mod plan_step;
mod process;
mod project_read_file;
pub(crate) mod read;
mod send_attachment;
pub(crate) mod skill;
// NOTE: `skill` is `pub(crate)` only to expose `render_inline` for the
// slash-command handler; the `inline` / `fork` submodules stay private.
mod sessions;
mod settings;
pub(crate) mod subagent;
mod submit_plan;
mod task;
pub(crate) mod team;
pub(crate) mod tool_search;
mod weather;
pub mod web_fetch;
pub mod web_search;
mod write;

// ── Public Re-exports ─────────────────────────────────────────────

pub use approval::{
    set_tool_permission_mode, submit_approval_response, ApprovalResponse, ToolPermissionMode,
};
pub use definitions::{
    get_amend_plan_tool, get_ask_user_question_tool, get_available_tools, get_canvas_tool,
    get_core_tools, get_core_tools_for_provider, get_deferred_tools,
    get_image_generate_tool_dynamic, get_notification_tool, get_plan_step_tool, get_subagent_tool,
    get_submit_plan_tool, get_tool_search_tool, get_tools_for_provider, get_web_search_tool,
    is_async_capable, is_concurrent_safe, is_core_tool, is_internal_tool, ToolDefinition,
};
pub use execution::{execute_tool_with_context, ToolExecContext};

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
pub const TOOL_GET_WEATHER: &str = "get_weather";
pub const TOOL_UPDATE_PLAN_STEP: &str = "update_plan_step";
pub const TOOL_ASK_USER_QUESTION: &str = "ask_user_question";
pub const TOOL_SUBMIT_PLAN: &str = "submit_plan";
pub const TOOL_AMEND_PLAN: &str = "amend_plan";
pub const TOOL_TOOL_SEARCH: &str = "tool_search";
pub const TOOL_TASK_CREATE: &str = "task_create";
pub const TOOL_TASK_UPDATE: &str = "task_update";
pub const TOOL_TASK_LIST: &str = "task_list";
pub const TOOL_JOB_STATUS: &str = "job_status";
pub const TOOL_PROJECT_READ_FILE: &str = "project_read_file";
pub const TOOL_TEAM: &str = "team";
pub const TOOL_PEEK_SESSIONS: &str = "peek_sessions";
pub const TOOL_GET_SETTINGS: &str = "get_settings";
pub const TOOL_UPDATE_SETTINGS: &str = "update_settings";
pub const TOOL_LIST_SETTINGS_BACKUPS: &str = "list_settings_backups";
pub const TOOL_RESTORE_SETTINGS_BACKUP: &str = "restore_settings_backup";
pub const TOOL_SEND_ATTACHMENT: &str = "send_attachment";
pub const TOOL_SKILL: &str = "skill";

// ── Shared Helpers ────────────────────────────────────────────────

/// Agent-level tool filter semantics.
///
/// Internal system tools are always allowed here because the Agent settings UI
/// only exposes user-facing built-in tools. Stronger restrictions such as
/// denied_tools / skill_allowed_tools / Plan Mode still apply separately.
pub fn agent_tool_filter_allows(name: &str, filter: &crate::agent_config::FilterConfig) -> bool {
    if is_internal_tool(name) {
        return true;
    }
    filter.is_allowed(name)
}

/// Combined tool visibility check shared by schema generation, tool_search, and
/// execution-layer defense-in-depth.
pub fn tool_visible_with_filters(
    name: &str,
    agent_filter: &crate::agent_config::FilterConfig,
    denied_tools: &[String],
    skill_allowed_tools: &[String],
    plan_mode_allowed_tools: &[String],
) -> bool {
    agent_tool_filter_allows(name, agent_filter)
        && !denied_tools.iter().any(|t| t == name)
        && (skill_allowed_tools.is_empty() || skill_allowed_tools.iter().any(|t| t == name))
        && (plan_mode_allowed_tools.is_empty() || plan_mode_allowed_tools.iter().any(|t| t == name))
}

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

#[cfg(test)]
mod tests {
    use crate::agent_config::FilterConfig;

    use super::{agent_tool_filter_allows, tool_visible_with_filters, TOOL_TOOL_SEARCH};

    #[test]
    fn agent_filter_keeps_internal_tools_visible() {
        let filter = FilterConfig {
            allow: vec!["read".to_string()],
            deny: vec![],
        };
        assert!(agent_tool_filter_allows(TOOL_TOOL_SEARCH, &filter));
    }

    #[test]
    fn combined_visibility_applies_all_restrictions() {
        let filter = FilterConfig {
            allow: vec!["read".to_string(), "write".to_string()],
            deny: vec!["write".to_string()],
        };

        assert!(tool_visible_with_filters("read", &filter, &[], &[], &[]));
        assert!(!tool_visible_with_filters("write", &filter, &[], &[], &[]));
        assert!(!tool_visible_with_filters(
            "read",
            &filter,
            &[],
            &["write".to_string()],
            &[]
        ));
        assert!(!tool_visible_with_filters(
            "read",
            &filter,
            &["read".to_string()],
            &[],
            &[]
        ));
        assert!(!tool_visible_with_filters(
            "read",
            &filter,
            &[],
            &[],
            &["write".to_string()]
        ));
    }
}

// ── Provider Enum ─────────────────────────────────────────────────

/// Supported LLM provider types for tool schema adaptation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Anthropic,
    OpenAI,
}
