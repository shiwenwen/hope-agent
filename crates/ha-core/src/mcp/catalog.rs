//! MCP 工具命名约定（单一来源）与 prompt 段 trampoline。
//!
//! `mcp__<server>__<tool>` 前缀约定被 kernel 分发（execution / dispatch）
//! 与 ha-mcp 运行时（invoke / registry / catalog 组名）两侧共用，故留
//! kernel；ha-mcp 侧经 `ha_core::mcp::catalog::*` 引用，**不得另写前缀
//! 判定**。

use ha_config_schema::mcp::McpServerConfig;

/// The `prefix_bytes` / `suffix_bytes` constants let callers decide
/// whether a name has our MCP prefix without ad-hoc string matching.
pub const MCP_TOOL_PREFIX: &str = "mcp__";

/// True iff the name is owned by the MCP subsystem. Exported for the
/// dispatch fallback branch in `tools::execution`.
pub fn is_mcp_tool_name(name: &str) -> bool {
    name.starts_with(MCP_TOOL_PREFIX)
}

/// Parse a namespaced MCP tool name of the form
/// `mcp__<server>__<tool>` into its two halves. Returns `None` when
/// the name isn't MCP-shaped or lacks the double-underscore separator
/// between the server and tool parts.
pub fn split_mcp_tool_name(name: &str) -> Option<(&str, &str)> {
    name.strip_prefix(MCP_TOOL_PREFIX)?.split_once("__")
}

/// True when the namespaced MCP tool belongs to a server whose tools should
/// be discoverable via `tool_search` instead of eagerly injected.
pub fn tool_belongs_to_deferred_server(name: &str, servers: &[McpServerConfig]) -> bool {
    let Some((server, _tool)) = split_mcp_tool_name(name) else {
        return false;
    };
    server_uses_deferred_tools(server, servers)
}

/// True when a configured, enabled MCP server explicitly opts its dynamic
/// tools into deferred discovery. Global Recommended-mode behavior is layered
/// on by `tools::dispatch`; this helper owns only the per-server switch.
pub fn server_uses_deferred_tools(server: &str, servers: &[McpServerConfig]) -> bool {
    servers
        .iter()
        .any(|cfg| cfg.enabled && cfg.name == server && cfg.deferred_tools)
}

/// Whether any configured MCP server has opted its tools into deferred loading.
pub fn has_deferred_tool_server(servers: &[McpServerConfig]) -> bool {
    servers.iter().any(|cfg| cfg.enabled && cfg.deferred_tools)
}

/// MCP Capabilities system prompt 段（实现在 ha-mcp，ArcSwap 支撑、同步
/// 可调）。未接线 `None`——prompt 无 MCP 段，与 manager 未初始化一致。
pub fn system_prompt_snippet() -> Option<String> {
    let hooks = crate::mcp_hooks::mcp_hooks()?;
    (hooks.system_prompt_snippet)()
}
