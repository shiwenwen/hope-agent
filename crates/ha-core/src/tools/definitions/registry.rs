use std::collections::HashSet;
use std::sync::LazyLock;

use super::super::ToolProvider;
use super::core_tools::get_available_tools;
use super::types::ToolDefinition;

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

/// Cached set of async-capable tool names — derived from
/// `ToolDefinition.async_capable`.
static ASYNC_CAPABLE_TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    super::super::dispatch::all_dispatchable_tools()
        .iter()
        .filter(|t| t.async_capable)
        .map(|t| t.name.clone())
        .collect()
});

/// Check if a tool is async-capable (supports `run_in_background` / auto-background).
pub fn is_async_capable(name: &str) -> bool {
    ASYNC_CAPABLE_TOOL_NAMES.contains(name)
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
