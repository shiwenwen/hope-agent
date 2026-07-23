use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use super::super::ToolProvider;
use super::core_tools::get_available_tools;
use super::types::{BackgroundPolicy, ToolDefinition};

/// Cached set of concurrent-safe tool names — derived from
/// `ToolDefinition.concurrent_safe`. Single source of truth lives at the
/// definition site (no separate hardcoded list).
static CONCURRENT_SAFE_TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    super::super::dispatch::all_dispatchable_tools()
        .iter()
        .filter(|t| t.concurrent_safe)
        .map(|t| t.name.clone())
        .collect()
});

/// Check if a tool is safe for concurrent execution within a tool round.
pub fn is_concurrent_safe(name: &str) -> bool {
    CONCURRENT_SAFE_TOOL_NAMES.contains(name)
}

/// Cached set of internal tool names — derived from
/// `ToolDefinition::is_internal()`.
static INTERNAL_TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    super::super::dispatch::all_dispatchable_tools()
        .iter()
        .filter(|t| t.is_internal())
        .map(|t| t.name.clone())
        .collect()
});

/// Check if a tool is an internal capability tool (never requires approval).
pub fn is_internal_tool(name: &str) -> bool {
    INTERNAL_TOOL_NAMES.contains(name)
}

/// Cached background policy registry. Only `GenericJob` tools may enter the
/// `async_jobs` wrapper; `SelfManaged` tools already own a durable lifecycle.
static BACKGROUND_POLICIES: LazyLock<HashMap<String, BackgroundPolicy>> = LazyLock::new(|| {
    let mut policies = super::super::dispatch::all_dispatchable_tools()
        .iter()
        .map(|tool| (tool.name.clone(), tool.background_policy))
        .collect::<HashMap<_, _>>();
    // Workflow is session-gated and intentionally absent from the static
    // dispatch catalog, but execution still needs its self-managed contract.
    let workflow = super::special_tools::get_workflow_tool();
    policies.insert(workflow.name, workflow.background_policy);
    policies
});

pub fn background_policy_for_tool(name: &str) -> Option<BackgroundPolicy> {
    BACKGROUND_POLICIES.get(name).copied().or_else(|| {
        // Dynamic MCP catalogs are refreshed after process startup and cannot
        // be frozen into the built-in LazyLock. Read their ArcSwap snapshot so
        // task-support changes take effect without restarting the app.
        crate::mcp::McpManager::global().and_then(|manager| {
            manager
                .mcp_tool_definitions()
                .iter()
                .find(|tool| tool.name == name)
                .map(|tool| tool.background_policy)
        })
    })
}

/// Check whether the generic tool-job executor may wrap this invocation.
pub fn is_generic_job_capable(name: &str) -> bool {
    background_policy_for_tool(name).is_some_and(BackgroundPolicy::supports_generic_job)
}

/// Returns all tool schemas formatted for the given provider.
pub fn get_tools_for_provider(provider: ToolProvider) -> Vec<serde_json::Value> {
    get_available_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}

/// Get tools that are structurally always-loaded (not defer-capable).
pub fn get_core_tools() -> Vec<ToolDefinition> {
    get_available_tools()
        .into_iter()
        .filter(|t| t.is_always_load())
        .collect()
}

/// Get tools that are capable of deferred discovery via tool_search.
pub fn get_deferred_tools() -> Vec<ToolDefinition> {
    get_available_tools()
        .into_iter()
        .filter(|t| t.is_deferred_default())
        .collect()
}

/// Get core tool schemas for a provider (when deferred loading is on).
pub fn get_core_tools_for_provider(provider: ToolProvider) -> Vec<serde_json::Value> {
    get_core_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}
