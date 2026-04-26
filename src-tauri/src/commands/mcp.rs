//! Tauri IPC commands for the MCP subsystem.
//!
//! Thin shells over the business logic in
//! [`ha_core::mcp::api`]. Every command returns `Result<T, CmdError>` —
//! Tauri serializes `T` to JS via serde and surfaces the `Err` (rendered as
//! a string) to `invoke()` callers as a rejected promise.
//!
//! Both the Tauri invoke handler in `src-tauri/src/lib.rs` and the
//! matching axum routes in `crates/ha-server/src/routes/mcp.rs` dispatch
//! to the SAME `ha_core::mcp::api::*` functions — behavior parity is
//! enforced by the single source of truth, not by copy-paste.

use crate::commands::CmdError;
use ha_core::mcp::api::{
    self, ImportSummary, McpLogLine, McpServerDraft, McpServerSummary, McpToolSummary,
};
use ha_core::mcp::config::McpGlobalSettings;
use ha_core::mcp::registry::ServerStatusSnapshot;

use crate::AppState;
use tauri::State;

// ── CRUD ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn mcp_list_servers(
    _state: State<'_, AppState>,
) -> Result<Vec<McpServerSummary>, CmdError> {
    Ok(api::list_servers().await)
}

#[tauri::command]
pub async fn mcp_get_server_status(
    id: String,
    _state: State<'_, AppState>,
) -> Result<ServerStatusSnapshot, CmdError> {
    api::get_server_status(&id)
        .await
        .ok_or_else(|| CmdError::msg(format!("MCP server '{id}' not found")))
}

#[tauri::command]
pub async fn mcp_add_server(
    draft: McpServerDraft,
    _state: State<'_, AppState>,
) -> Result<McpServerSummary, CmdError> {
    api::add_server(draft).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_update_server(
    id: String,
    draft: McpServerDraft,
    _state: State<'_, AppState>,
) -> Result<McpServerSummary, CmdError> {
    api::update_server(&id, draft).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_remove_server(id: String, _state: State<'_, AppState>) -> Result<(), CmdError> {
    api::remove_server(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_reorder_servers(
    order: Vec<String>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    api::reorder_servers(order).await.map_err(Into::into)
}

// ── Connection + diagnostics ─────────────────────────────────────

#[tauri::command]
pub async fn mcp_test_connection(
    id: String,
    _state: State<'_, AppState>,
) -> Result<ServerStatusSnapshot, CmdError> {
    api::test_connection(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_reconnect_server(
    id: String,
    _state: State<'_, AppState>,
) -> Result<ServerStatusSnapshot, CmdError> {
    api::reconnect_server(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_start_oauth(id: String, _state: State<'_, AppState>) -> Result<(), CmdError> {
    api::start_oauth(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_sign_out(id: String, _state: State<'_, AppState>) -> Result<(), CmdError> {
    api::sign_out(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_list_tools(
    id: String,
    _state: State<'_, AppState>,
) -> Result<Vec<McpToolSummary>, CmdError> {
    api::list_server_tools(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_get_recent_logs(
    id: String,
    limit: Option<usize>,
    _state: State<'_, AppState>,
) -> Result<Vec<McpLogLine>, CmdError> {
    api::get_recent_logs(&id, limit.unwrap_or(200))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mcp_import_claude_desktop_config(
    json: String,
    _state: State<'_, AppState>,
) -> Result<ImportSummary, CmdError> {
    api::import_claude_desktop_config(&json)
        .await
        .map_err(Into::into)
}

// ── Global settings ──────────────────────────────────────────────

#[tauri::command]
pub async fn mcp_get_global_settings(
    _state: State<'_, AppState>,
) -> Result<McpGlobalSettings, CmdError> {
    Ok(api::get_global_settings())
}

#[tauri::command]
pub async fn mcp_update_global_settings(
    settings: McpGlobalSettings,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    api::update_global_settings(settings)
        .await
        .map_err(Into::into)
}
