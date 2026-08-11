//! 工具可见性收窄谓词与 [`ToolScope`]——schema 生成 / tool_search /
//! 执行层纵深防御共用的纯函数层。只依赖名字常量与
//! `agent_config::FilterConfig`，不依赖分发注册表。

use super::names::*;

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
                | TOOL_READ_CONTEXT_RESOURCE
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
            | TOOL_READ_CONTEXT_RESOURCE
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
    let turn_local_context_read = name == TOOL_READ_CONTEXT_RESOURCE;
    !denied_tools.iter().any(|t| t == name)
        && (turn_local_context_read
            || skill_allowed_tools.is_empty()
            || skill_allowed_tools.iter().any(|t| t == name))
        && (turn_local_context_read
            || plan_mode_allowed_tools.is_empty()
            || plan_mode_allowed_tools.iter().any(|t| t == name))
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
