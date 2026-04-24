//! Model Context Protocol (MCP) client integration.
//!
//! Adds "any external MCP server" as a tool source alongside the built-in
//! tool catalog and skills. See `docs/architecture/mcp.md` for the full
//! subsystem overview.
//!
//! Module layout follows the plan file — each file has a single narrow
//! responsibility; no file-level circular imports:
//!
//! * [`config`]     — `McpServerConfig` / `McpTransportSpec` etc. (persisted)
//! * [`errors`]     — `McpError` taxonomy used across the subsystem
//! * [`events`]     — EventBus event names + emit helpers
//!
//! Subsequent phases add: `registry`, `client`, `transport`, `watchdog`,
//! `catalog`, `invoke`, `oauth`, `credentials`, `prompts`, `resources`.
//!
//! Hard rule (enforced by code review, not the compiler): **no `use tauri::*`
//! anywhere under `mcp/`.** The Tauri and axum shells talk to this module
//! only through the public API re-exported below.

pub mod catalog;
pub mod client;
pub mod config;
pub mod errors;
pub mod events;
pub mod invoke;
pub mod registry;
pub mod transport;
pub mod watchdog;

pub use config::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};
pub use errors::{McpError, McpResult};
pub use registry::{McpManager, ServerHandle, ServerState, ServerStatusSnapshot, ToolIndexEntry};
