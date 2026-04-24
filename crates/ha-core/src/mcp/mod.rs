//! Model Context Protocol (MCP) client integration.
//!
//! Adds "any external MCP server" as a tool source alongside the built-in
//! tool catalog and skills. See `docs/architecture/mcp.md` for the full
//! subsystem overview.
//!
//! Module layout follows the plan file — each file has a single narrow
//! responsibility; no file-level circular imports:
//!
//! * [`config`]       — `McpServerConfig` / `McpTransportSpec` etc. (persisted)
//! * [`errors`]       — `McpError` taxonomy used across the subsystem
//! * [`events`]       — EventBus event names + emit helpers
//! * [`credentials`]  — OAuth token persistence (0600 file under ~/.hope-agent)
//!
//! Subsequent phases add: `oauth`, `prompts`, `resources`.
//!
//! Hard rule (enforced by code review, not the compiler): **no `use tauri::*`
//! anywhere under `mcp/`.** The Tauri and axum shells talk to this module
//! only through the public API re-exported below.

pub mod api;
pub mod catalog;
pub mod client;
pub mod config;
pub mod credentials;
pub mod errors;
pub mod events;
pub mod invoke;
pub mod oauth;
pub mod prompts;
pub mod registry;
pub mod resources;
pub mod transport;
pub mod watchdog;

pub use config::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};
pub use credentials::McpCredentials;
pub use errors::{McpError, McpResult};
pub use registry::{McpManager, ServerHandle, ServerState, ServerStatusSnapshot, ToolIndexEntry};

/// Look up a server by id or name, returning an `anyhow`-flavored
/// error so tool handlers can propagate it directly. Wrapper over
/// [`McpManager::locate`] that also turns "manager not initialized"
/// and "server not found" into distinct messages.
pub(crate) async fn locate_server(
    name_or_id: &str,
) -> anyhow::Result<std::sync::Arc<ServerHandle>> {
    let mgr =
        McpManager::global().ok_or_else(|| anyhow::anyhow!("MCP subsystem not initialized"))?;
    mgr.locate(name_or_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("MCP server '{name_or_id}' not found"))
}
