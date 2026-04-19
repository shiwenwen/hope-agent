use std::collections::HashSet;
use std::sync::LazyLock;

use super::super::{
    ToolProvider, TOOL_AGENTS_LIST, TOOL_ASK_USER_QUESTION, TOOL_FIND, TOOL_GET_WEATHER, TOOL_GREP,
    TOOL_IMAGE, TOOL_LS, TOOL_MEMORY_GET, TOOL_PDF, TOOL_PROJECT_READ_FILE, TOOL_READ,
    TOOL_RECALL_MEMORY, TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST, TOOL_SESSION_STATUS,
    TOOL_TASK_LIST, TOOL_WEB_FETCH, TOOL_WEB_SEARCH,
};
use super::core_tools::get_available_tools;
use super::extra_tools::{get_canvas_tool, get_notification_tool, get_web_search_tool};
use super::plan_tools::{get_amend_plan_tool, get_plan_step_tool, get_submit_plan_tool};
use super::special_tools::{
    get_acp_spawn_tool, get_image_generate_tool, get_subagent_tool, get_tool_search_tool,
};
use super::types::ToolDefinition;

/// Tools that are safe for concurrent execution (read-only, no side effects).
/// These tools can run in parallel within a single tool round.
static CONCURRENT_SAFE_TOOL_NAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        TOOL_READ,
        TOOL_PROJECT_READ_FILE,
        TOOL_LS,
        TOOL_GREP,
        TOOL_FIND,
        TOOL_RECALL_MEMORY,
        TOOL_MEMORY_GET,
        TOOL_WEB_SEARCH,
        TOOL_WEB_FETCH,
        TOOL_AGENTS_LIST,
        TOOL_SESSIONS_LIST,
        TOOL_SESSION_STATUS,
        TOOL_SESSIONS_HISTORY,
        TOOL_IMAGE,
        TOOL_PDF,
        TOOL_GET_WEATHER,
        TOOL_ASK_USER_QUESTION,
        TOOL_TASK_LIST,
        super::super::TOOL_PEEK_SESSIONS,
    ]
    .into_iter()
    .collect()
});

/// Check if a tool is safe for concurrent execution within a tool round.
pub fn is_concurrent_safe(name: &str) -> bool {
    CONCURRENT_SAFE_TOOL_NAMES.contains(name)
}

/// Cached set of internal tool names — derived from ToolDefinition.internal flag.
/// This is the single source of truth; no separate hardcoded list needed.
static INTERNAL_TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set: HashSet<String> = get_available_tools()
        .into_iter()
        .filter(|t| t.internal)
        .map(|t| t.name)
        .collect();
    // Tools not registered via get_available_tools() must be listed here —
    // their `internal: true` flag is otherwise invisible to the approval gate.
    for t in [
        get_notification_tool(),
        get_subagent_tool(),
        get_image_generate_tool(),
        get_canvas_tool(),
        get_acp_spawn_tool(),
        get_tool_search_tool(),
        // Plan Mode tools — injected on-demand by `apply_plan_tools`, never via
        // get_available_tools().
        get_submit_plan_tool(),
        get_amend_plan_tool(),
        get_plan_step_tool(),
    ] {
        if t.internal {
            set.insert(t.name);
        }
    }
    // job_status is auto-internal: querying job state never requires approval.
    set.insert("job_status".to_string());
    set
});

/// Check if a tool is an internal capability tool (never requires approval).
pub fn is_internal_tool(name: &str) -> bool {
    INTERNAL_TOOL_NAMES.contains(name)
}

/// Cached set of async-capable tool names — derived from ToolDefinition.async_capable
/// on both core and conditionally-injected tools.
static ASYNC_CAPABLE_TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set: HashSet<String> = get_available_tools()
        .into_iter()
        .filter(|t| t.async_capable)
        .map(|t| t.name)
        .collect();
    for t in [
        get_web_search_tool(),
        get_image_generate_tool(),
        get_notification_tool(),
        get_subagent_tool(),
        get_canvas_tool(),
        get_acp_spawn_tool(),
    ] {
        if t.async_capable {
            set.insert(t.name);
        }
    }
    set
});

/// Check if a tool is async-capable (supports `run_in_background` / auto-background).
pub fn is_async_capable(name: &str) -> bool {
    ASYNC_CAPABLE_TOOL_NAMES.contains(name)
}

/// Returns all tool schemas formatted for the given provider
pub fn get_tools_for_provider(provider: ToolProvider) -> Vec<serde_json::Value> {
    get_available_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}

/// Get only core (always-loaded) tools — used when deferred loading is enabled.
/// `always_load` overrides `deferred`: a tool can opt out of deferred gating
/// even when it isn't in `CORE_TOOL_NAMES` (e.g. `skill`).
pub fn get_core_tools() -> Vec<ToolDefinition> {
    get_available_tools()
        .into_iter()
        .filter(|t| !t.deferred || t.always_load)
        .collect()
}

/// Get only deferred tools — discoverable via tool_search. Mirrors
/// `get_core_tools`'s `always_load` override.
pub fn get_deferred_tools() -> Vec<ToolDefinition> {
    get_available_tools()
        .into_iter()
        .filter(|t| t.deferred && !t.always_load)
        .collect()
}

/// Get core tool schemas for a provider (when deferred loading is on).
pub fn get_core_tools_for_provider(provider: ToolProvider) -> Vec<serde_json::Value> {
    get_core_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}
