use serde_json::{json, Value};

pub(crate) mod acp_spawn;
mod agents;
mod app_update;
mod apply_patch;
pub(crate) mod approval;
mod artifact;
pub(crate) mod ask_user_question;
pub mod audio_generate;
pub(crate) mod browser;
pub mod canvas;
mod core_memory;
mod cron;
mod definitions;
pub mod design;
pub(crate) mod diff_util;
pub mod dispatch;
mod edit;
mod enter_plan_mode;
pub(crate) mod exec;
mod execution;
pub(crate) mod feishu;
mod find;
mod goal;
mod grep;
pub(crate) mod image;
pub mod image_generate;
pub(crate) mod image_markers;
mod issue_report;
pub(crate) mod job_status;
mod loop_tool;
mod ls;
mod lsp;
mod mac_control;
mod memory;
pub(crate) mod note;
mod notification;
pub(crate) mod pdf;
mod process;
mod project_memory;
pub(crate) mod read;
pub(crate) mod rejection;
mod runtime_cancel;
mod schedule_wakeup;
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
pub mod web_fetch_common;
pub mod web_search;
mod workflow_tool;
mod write;

// ── Public Re-exports ─────────────────────────────────────────────

pub(crate) use task::task_reminder_text;

pub use approval::{
    deny_all_pending, deny_pending_for_session, emit_approval_resolved,
    list_pending_approval_requests, submit_approval_response, ApprovalOrigin, ApprovalRequest,
    ApprovalResolutionSource, ApprovalResponse, ApprovalSubmitError, EVENT_APPROVAL_RESOLVED,
};
pub use definitions::{
    get_artifact_tool, get_ask_user_question_tool, get_audio_generate_tool_dynamic,
    get_available_tools, get_canvas_tool, get_core_tools, get_core_tools_for_provider,
    get_deferred_tools, get_enter_plan_mode_tool, get_image_generate_tool_dynamic,
    get_notification_tool, get_subagent_tool, get_submit_plan_tool, get_tool_search_tool,
    get_tools_for_provider, get_web_search_tool, get_workflow_tool, is_async_capable,
    is_concurrent_safe, is_internal_tool, CoreSubclass, ToolApprovalHint, ToolDefinition,
    ToolEffect, ToolInputMetadata, ToolInterruptBehavior, ToolMetadata, ToolPathExtractorMetadata,
    ToolPermissionMetadata, ToolPermissionSubject, ToolRenderMetadata, ToolResultKind, ToolRisk,
    ToolTier, ToolValidationMetadata,
};
#[doc(hidden)]
pub use execution::EffectiveArgsSink;
pub use execution::{
    execute_tool_with_context, purge_tool_results_for_session, PidSink, SessionDbHandle,
    ToolExecContext,
};
pub use rejection::{ToolRejection, TOOL_ERROR_PREFIX};

// ── Tool Name Constants ──────────────────────────────────────────

pub const TOOL_EXEC: &str = "exec";
pub const TOOL_PROCESS: &str = "process";
pub const TOOL_READ: &str = "read";
pub const TOOL_WRITE: &str = "write";
pub const TOOL_EDIT: &str = "edit";
pub const TOOL_LS: &str = "ls";
pub const TOOL_LSP: &str = "lsp";
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
pub const TOOL_CORE_MEMORY: &str = "core_memory";
pub const TOOL_PROJECT_MEMORY: &str = "project_memory";
pub const TOOL_MANAGE_CRON: &str = "manage_cron";
pub const TOOL_BROWSER: &str = "browser";
pub const TOOL_MAC_CONTROL: &str = "mac_control";
pub const TOOL_SEND_NOTIFICATION: &str = "send_notification";
pub const TOOL_SUBAGENT: &str = "subagent";
pub const TOOL_MEMORY_GET: &str = "memory_get";
pub const TOOL_AGENTS_LIST: &str = "agents_list";

/// Parse a model-facing compact call variant such as `browser__snapshot`.
/// Only explicitly registered composite tools are accepted, so arbitrary
/// tool names containing `__` (notably MCP names) are never rewritten.
pub(crate) fn split_call_variant_name(name: &str) -> Option<(&str, &str)> {
    let (canonical, action) = name.rsplit_once("__")?;
    let supported = matches!(
        canonical,
        TOOL_BROWSER | TOOL_MAC_CONTROL | TOOL_MANAGE_CRON | TOOL_APP_UPDATE
    ) && dispatch::all_dispatchable_tools()
        .iter()
        .find(|definition| definition.name == canonical)
        .is_some_and(|definition| definition.call_variant_actions().contains(&action));
    supported.then_some((canonical, action))
}

pub(crate) fn canonical_tool_schema_name(name: &str) -> &str {
    split_call_variant_name(name)
        .map(|(canonical, _)| canonical)
        .unwrap_or(name)
}

/// Convert a compact model-facing variant back into the canonical call before
/// permission, hooks, audit, persistence, and execution. The fixed action
/// always wins over a model-supplied conflicting value.
pub(crate) fn normalize_call_variant(name: &str, args: &Value) -> Option<(String, Value)> {
    let (canonical, action) = split_call_variant_name(name)?;
    let mut normalized = args.clone();
    let object = normalized.as_object_mut()?;
    object.insert("action".to_string(), Value::String(action.to_string()));
    Some((canonical.to_string(), normalized))
}

// Knowledge base (note_*) tools.
pub const TOOL_NOTE_CREATE: &str = "note_create";
pub const TOOL_NOTE_READ: &str = "note_read";
pub const TOOL_NOTE_UPDATE: &str = "note_update";
pub const TOOL_NOTE_PATCH: &str = "note_patch";
pub const TOOL_NOTE_APPEND: &str = "note_append";
pub const TOOL_NOTE_DELETE: &str = "note_delete";
pub const TOOL_NOTE_SEARCH: &str = "note_search";
pub const TOOL_NOTE_LINK: &str = "note_link";
pub const TOOL_NOTE_BACKLINKS: &str = "note_backlinks";
pub const TOOL_NOTE_BY_TAG: &str = "note_by_tag";
pub const TOOL_NOTE_TAGS: &str = "note_tags";
pub const TOOL_NOTE_RENAME: &str = "note_rename";
pub const TOOL_NOTE_MOVE: &str = "note_move";
pub const TOOL_NOTE_SET_FRONTMATTER: &str = "note_set_frontmatter";
pub const TOOL_NOTE_ASSIGN_BLOCK: &str = "note_assign_block";
pub const TOOL_NOTE_BROKEN_LINKS: &str = "note_broken_links";
pub const TOOL_NOTE_ORPHANS: &str = "note_orphans";
pub const TOOL_NOTE_GRAPH: &str = "note_graph";
pub const TOOL_NOTE_SIMILAR: &str = "note_similar";
pub const TOOL_NOTE_RELATED: &str = "note_related";
pub const TOOL_NOTE_SUGGEST_LINKS: &str = "note_suggest_links";
pub const TOOL_NOTE_DISTILL: &str = "note_distill";
pub const TOOL_NOTE_MOC: &str = "note_moc";
pub const TOOL_KNOWLEDGE_RECALL: &str = "knowledge_recall";
pub const TOOL_SESSION_TO_NOTE: &str = "session_to_note";
pub const TOOL_SESSIONS_LIST: &str = "sessions_list";
pub const TOOL_SESSION_STATUS: &str = "session_status";
pub const TOOL_SESSIONS_SEARCH: &str = "sessions_search";
pub const TOOL_SESSIONS_HISTORY: &str = "sessions_history";
pub const TOOL_SESSIONS_SEND: &str = "sessions_send";
pub const TOOL_IMAGE: &str = "image";
pub const TOOL_IMAGE_GENERATE: &str = "image_generate";
pub const TOOL_AUDIO_GENERATE: &str = "audio_generate";
pub const TOOL_ISSUE_REPORT: &str = "issue_report";
pub const TOOL_PDF: &str = "pdf";
pub const TOOL_CANVAS: &str = "canvas";
pub const TOOL_ARTIFACT: &str = "artifact";
pub const TOOL_DESIGN: &str = "design";
pub const TOOL_ACP_SPAWN: &str = "acp_spawn";
pub const TOOL_GET_WEATHER: &str = "get_weather";
pub const TOOL_ASK_USER_QUESTION: &str = "ask_user_question";
pub const TOOL_SUBMIT_PLAN: &str = "submit_plan";
pub const TOOL_ENTER_PLAN_MODE: &str = "enter_plan_mode";
pub const TOOL_TOOL_SEARCH: &str = "tool_search";
pub const TOOL_WORKFLOW: &str = "workflow";
pub const TOOL_TASK_CREATE: &str = "task_create";
pub const TOOL_TASK_UPDATE: &str = "task_update";
pub const TOOL_TASK_LIST: &str = "task_list";
pub const TOOL_GOAL_STATUS: &str = "goal_status";
pub const TOOL_GOAL_PREPARE_CONTRACT: &str = "goal_prepare_contract";
pub const TOOL_GOAL_CHECKPOINT: &str = "goal_checkpoint";
pub const TOOL_GOAL_RECORD_EVIDENCE: &str = "goal_record_evidence";
pub const TOOL_GOAL_EVALUATE: &str = "goal_evaluate";
pub const TOOL_GOAL_FINISH_REQUEST: &str = "goal_finish_request";
pub const TOOL_GOAL_BLOCK_REQUEST: &str = "goal_block_request";
pub const TOOL_LOOP_STATUS: &str = "loop_status";
pub const TOOL_LOOP_RESCHEDULE: &str = "loop_reschedule";
pub const TOOL_LOOP_STOP: &str = "loop_stop";
pub const TOOL_LOOP_RECORD_PROGRESS: &str = "loop_record_progress";
pub const TOOL_LOOP_WATCH: &str = "loop_watch";
pub const TOOL_LOOP_UNWATCH: &str = "loop_unwatch";
pub const TOOL_APP_UPDATE: &str = "app_update";
pub const TOOL_JOB_STATUS: &str = "job_status";
pub const TOOL_SCHEDULE_WAKEUP: &str = "schedule_wakeup";
pub const TOOL_RUNTIME_CANCEL: &str = "runtime_cancel";
pub const TOOL_TEAM: &str = "team";
pub const TOOL_PEEK_SESSIONS: &str = "peek_sessions";
pub const TOOL_GET_SETTINGS: &str = "get_settings";
pub const TOOL_UPDATE_SETTINGS: &str = "update_settings";
pub const TOOL_LIST_SETTINGS_BACKUPS: &str = "list_settings_backups";
pub const TOOL_RESTORE_SETTINGS_BACKUP: &str = "restore_settings_backup";
pub const TOOL_SEND_ATTACHMENT: &str = "send_attachment";
pub const TOOL_SKILL: &str = "skill";
pub const TOOL_MCP_RESOURCE: &str = "mcp_resource";
pub const TOOL_MCP_PROMPT: &str = "mcp_prompt";

/// Optional per-call async-job timeout injected into async-capable tool schemas.
///
/// This is intentionally separate from tool-specific timeouts such as
/// `exec.timeout`: it caps the outer async job, sets a per-call cap when the
/// user's `asyncTools.maxJobSecs` is unlimited, and can only tighten a positive
/// user-configured boundary.
pub const ASYNC_JOB_TIMEOUT_ARG: &str = "job_timeout_secs";

// ── Runtime Timeout Policy Helpers ───────────────────────────────

pub(crate) fn should_ignore_model_runtime_timeout_when_user_unlimited(
    user_limit_secs: u64,
) -> bool {
    matches!(
        crate::config::cached_config()
            .timeout_policy
            .model_runtime_overrides,
        crate::config::ModelRuntimeTimeoutOverrides::IgnoreWhenUserUnlimited
    ) && user_limit_secs == 0
}

pub(crate) fn audit_model_runtime_timeout_override(
    ctx: Option<&ToolExecContext>,
    tool: &str,
    parameter: &str,
    requested_secs: u64,
    effective_secs: u64,
    user_limit_secs: Option<u64>,
    ignored: bool,
    reason: &str,
) {
    let mode = crate::config::cached_config()
        .timeout_policy
        .model_runtime_overrides;
    if matches!(mode, crate::config::ModelRuntimeTimeoutOverrides::Allow) && !ignored {
        return;
    }

    let details = json!({
        "tool": tool,
        "parameter": parameter,
        "requestedSecs": requested_secs,
        "effectiveSecs": effective_secs,
        "userLimitSecs": user_limit_secs,
        "ignored": ignored,
        "reason": reason,
        "policy": mode,
    });
    let level = if ignored { "warn" } else { "info" };
    let message = if ignored {
        format!(
            "Ignored model runtime timeout override for {tool}.{parameter}: requested {requested_secs}s, effective {effective_secs}s ({reason})"
        )
    } else {
        format!(
            "Model runtime timeout override for {tool}.{parameter}: requested {requested_secs}s, effective {effective_secs}s ({reason})"
        )
    };

    if let Some(logger) = crate::get_logger() {
        logger.log(
            level,
            "tool",
            "timeout_policy::model_runtime_override",
            &message,
            Some(details.to_string()),
            ctx.and_then(|c| c.session_id.clone()),
            ctx.and_then(|c| c.agent_id.clone()),
        );
    }
}

pub(crate) async fn emit_model_runtime_timeout_metadata(
    ctx: &ToolExecContext,
    tool: &str,
    parameter: &str,
    requested_secs: u64,
    effective_secs: u64,
    user_limit_secs: Option<u64>,
    ignored: bool,
    reason: &str,
) {
    let mode = crate::config::cached_config()
        .timeout_policy
        .model_runtime_overrides;
    if matches!(mode, crate::config::ModelRuntimeTimeoutOverrides::Allow) && !ignored {
        return;
    }

    ctx.emit_metadata(json!({
        "kind": "runtime_timeout_override",
        "tool": tool,
        "parameter": parameter,
        "requestedSecs": requested_secs,
        "effectiveSecs": effective_secs,
        "userLimitSecs": user_limit_secs,
        "ignored": ignored,
        "reason": reason,
        "policy": mode,
    }))
    .await;
}

// ── Shared Helpers ────────────────────────────────────────────────

/// True for built-in long-term memory tools. These tools are governed by the
/// Memory tier gate (effective product master, agent memory switch, incognito)
/// and must stay aligned across schema generation, tool_search, prompt text,
/// and execution-layer defense in depth.
pub fn is_memory_tool(name: &str) -> bool {
    matches!(
        name,
        TOOL_RECALL_MEMORY
            | TOOL_SAVE_MEMORY
            | TOOL_UPDATE_MEMORY
            | TOOL_DELETE_MEMORY
            | TOOL_MEMORY_GET
            | TOOL_UPDATE_CORE_MEMORY
            | TOOL_CORE_MEMORY
            | TOOL_PROJECT_MEMORY
    )
}

/// True for built-in tools that are useless without an attached knowledge base:
/// all `note_*` tools plus `session_to_note` (they all resolve a `kb` through
/// `effective_kb_access` and hard-fail when no KB is reachable). Used to drop
/// them from the eager tool schema when the session has zero accessible KBs —
/// pure UX / token saving on top of the execution-layer access gate.
///
/// Deliberately EXCLUDES `knowledge_recall`: it is `Standard`/deferred and
/// cross-store (still searches Memory without any KB), so it must stay available.
pub fn is_kb_scoped_tool(name: &str) -> bool {
    name.starts_with("note_") || name == TOOL_SESSION_TO_NOTE
}

/// White-list predicate for [`ToolScope::Knowledge`] — the trimmed tool set the
/// knowledge-space sidebar chat injects. Keeps note read/write, cross-store
/// recall, memory, and the framework basics the dispatcher / deferred-tool flow
/// need (`skill` / `tool_search` / `ask_user_question` / `runtime_cancel` /
/// `job_status`); everything else (exec / browser / image / subagent / cron /
/// channel / web / raw fs …) is dropped so a document-writing chat can't wander
/// into unrelated capabilities.
///
/// Purely schema/visibility narrowing — it never WIDENS anything. KB access is
/// still decided solely by `effective_kb_access`.
pub fn is_knowledge_scope_tool(name: &str) -> bool {
    name.starts_with("note_")
        || matches!(
            name,
            TOOL_SESSION_TO_NOTE
                | TOOL_KNOWLEDGE_RECALL
                | TOOL_RECALL_MEMORY
                | TOOL_SAVE_MEMORY
                | TOOL_UPDATE_MEMORY
                | TOOL_MEMORY_GET
                | TOOL_SKILL
                | TOOL_TOOL_SEARCH
                | TOOL_ASK_USER_QUESTION
                | TOOL_RUNTIME_CANCEL
                | TOOL_JOB_STATUS
        )
}

/// White-list predicate for [`ToolScope::Design`] — the trimmed tool set the
/// design-space per-project chat injects. Keeps the `design` tool (the whole
/// create/iterate/restyle/critique surface), reference-gathering (`web_search` /
/// `web_fetch` / `image_generate` / `audio_generate`), cross-store recall, and the framework basics
/// the dispatcher / deferred-tool flow need; everything else (exec / browser /
/// subagent / cron / channel / raw fs …) is dropped so a design chat stays
/// focused on the artifact and can't wander into unrelated capabilities.
///
/// Purely schema/visibility narrowing — it never WIDENS anything. The `design`
/// tool is still gated by `app_config.design.enabled` at dispatch.
pub fn is_design_scope_tool(name: &str) -> bool {
    matches!(
        name,
        TOOL_DESIGN
            | TOOL_WEB_SEARCH
            | TOOL_WEB_FETCH
            | TOOL_IMAGE_GENERATE
            | TOOL_AUDIO_GENERATE
            | TOOL_RECALL_MEMORY
            | TOOL_MEMORY_GET
            | TOOL_KNOWLEDGE_RECALL
            | TOOL_SKILL
            | TOOL_TOOL_SEARCH
            | TOOL_ASK_USER_QUESTION
            | TOOL_RUNTIME_CANCEL
            | TOOL_JOB_STATUS
    )
}

/// Restricts which tools are visible for a turn, orthogonal to the agent's own
/// allow/deny config and to the chat source. `Knowledge` is the knowledge-space
/// sidebar chat's trimmed set; `Design` is the design-space per-project chat's.
/// `None` on [`crate::chat_engine::ChatEngineParams`] means no extra narrowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolScope {
    Knowledge,
    Design,
}

impl ToolScope {
    /// Parse the wire string (`"knowledge"` / `"design"`) into a scope; anything
    /// else → None.
    pub fn from_str_opt(s: Option<&str>) -> Option<Self> {
        match s {
            Some("knowledge") => Some(ToolScope::Knowledge),
            Some("design") => Some(ToolScope::Design),
            _ => None,
        }
    }

    /// True iff a tool `name` is visible under this scope.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            ToolScope::Knowledge => is_knowledge_scope_tool(name),
            ToolScope::Design => is_design_scope_tool(name),
        }
    }
}

/// Combined context-level visibility check shared by schema generation,
/// tool_search, and execution-layer defense-in-depth. Agent-level on/off
/// switches are handled by `dispatch::resolve_tool_fate`; this helper applies
/// only additional narrowing layers.
pub fn tool_visible_with_filters(
    name: &str,
    _agent_filter: &crate::agent_config::FilterConfig,
    denied_tools: &[String],
    skill_allowed_tools: &[String],
    plan_mode_allowed_tools: &[String],
) -> bool {
    !denied_tools.iter().any(|t| t == name)
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
pub fn expand_tilde(path: &str) -> String {
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

    use super::{is_kb_scoped_tool, is_knowledge_scope_tool, tool_visible_with_filters, ToolScope};

    #[test]
    fn knowledge_scope_whitelist() {
        // All note_* + the curated recall / memory / framework basics are kept.
        for t in [
            super::TOOL_NOTE_CREATE,
            super::TOOL_NOTE_PATCH,
            super::TOOL_NOTE_SEARCH,
            "note_brand_new",
            super::TOOL_SESSION_TO_NOTE,
            super::TOOL_KNOWLEDGE_RECALL,
            super::TOOL_RECALL_MEMORY,
            super::TOOL_SAVE_MEMORY,
            super::TOOL_MEMORY_GET,
            super::TOOL_SKILL,
            super::TOOL_TOOL_SEARCH,
            super::TOOL_ASK_USER_QUESTION,
            super::TOOL_RUNTIME_CANCEL,
            super::TOOL_JOB_STATUS,
        ] {
            assert!(
                is_knowledge_scope_tool(t),
                "{t} should be in knowledge scope"
            );
            assert!(ToolScope::Knowledge.allows(t), "{t} should be allowed");
        }
        // Unrelated capabilities are dropped from the knowledge chat.
        for t in [
            super::TOOL_EXEC,
            super::TOOL_BROWSER,
            super::TOOL_WEB_SEARCH,
            super::TOOL_SUBAGENT,
            super::TOOL_MANAGE_CRON,
            super::TOOL_IMAGE_GENERATE,
            "read",
            "write",
            "edit",
        ] {
            assert!(!is_knowledge_scope_tool(t), "{t} must be excluded");
            assert!(!ToolScope::Knowledge.allows(t), "{t} must be excluded");
        }
    }

    #[test]
    fn tool_scope_parses_wire_string() {
        assert_eq!(
            ToolScope::from_str_opt(Some("knowledge")),
            Some(ToolScope::Knowledge)
        );
        assert_eq!(ToolScope::from_str_opt(Some("bogus")), None);
        assert_eq!(ToolScope::from_str_opt(None), None);
    }

    #[test]
    fn kb_scoped_tool_predicate() {
        // All note_* tools are KB-scoped (gated off on a no-KB session).
        assert!(is_kb_scoped_tool(super::TOOL_NOTE_CREATE));
        assert!(is_kb_scoped_tool(super::TOOL_NOTE_SEARCH));
        assert!(is_kb_scoped_tool(super::TOOL_NOTE_MOC));
        assert!(is_kb_scoped_tool("note_anything_new"));
        // session_to_note also requires a KB to write into.
        assert!(is_kb_scoped_tool(super::TOOL_SESSION_TO_NOTE));
        // knowledge_recall is cross-store (Memory + notes) and must stay available
        // without a KB — it must NOT be caught by the gate.
        assert!(!is_kb_scoped_tool(super::TOOL_KNOWLEDGE_RECALL));
        // Unrelated tools are never gated.
        assert!(!is_kb_scoped_tool(super::TOOL_RECALL_MEMORY));
        assert!(!is_kb_scoped_tool("read"));
    }

    #[test]
    fn combined_visibility_applies_context_restrictions() {
        let filter = FilterConfig {
            allow: vec!["read".to_string(), "write".to_string()],
            deny: vec!["write".to_string()],
        };

        assert!(tool_visible_with_filters("read", &filter, &[], &[], &[]));
        assert!(tool_visible_with_filters("write", &filter, &[], &[], &[]));
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
