//! HTTP routes for the MCP subsystem.
//!
//! Every handler delegates to [`ha_core::mcp::api`] so behavior parity
//! with the Tauri shell is guaranteed — the only difference here is the
//! wire-level framing (path + method + JSON body) and status-code
//! mapping via [`crate::error::AppError`].

use axum::extract::{Path, Query};
use axum::Json;
use ha_core::mcp::api::{
    self, ImportSummary, McpLogLine, McpServerDraft, McpServerSummary, McpToolSummary,
};
use ha_core::mcp::config::McpGlobalSettings;
use ha_core::mcp::registry::ServerStatusSnapshot;
use serde::Deserialize;

use crate::error::AppError;

#[derive(Deserialize)]
pub struct ReorderPayload {
    pub order: Vec<String>,
}

#[derive(Deserialize)]
pub struct ImportPayload {
    pub json: String,
}

#[derive(Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

// ── CRUD ─────────────────────────────────────────────────────────

pub async fn list_servers() -> Result<Json<Vec<McpServerSummary>>, AppError> {
    Ok(Json(api::list_servers().await))
}

pub async fn get_server_status(
    Path(id): Path<String>,
) -> Result<Json<ServerStatusSnapshot>, AppError> {
    api::get_server_status(&id)
        .await
        .map(Json)
        .ok_or_else(|| AppError::not_found(format!("MCP server '{id}' not found")))
}

pub async fn add_server(
    Json(draft): Json<McpServerDraft>,
) -> Result<Json<McpServerSummary>, AppError> {
    api::add_server(draft)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn update_server(
    Path(id): Path<String>,
    Json(draft): Json<McpServerDraft>,
) -> Result<Json<McpServerSummary>, AppError> {
    api::update_server(&id, draft)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn remove_server(Path(id): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    api::remove_server(&id)
        .await
        .map(|_| Json(serde_json::json!({ "ok": true })))
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn reorder_servers(
    Json(payload): Json<ReorderPayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    api::reorder_servers(payload.order)
        .await
        .map(|_| Json(serde_json::json!({ "ok": true })))
        .map_err(|e| AppError::bad_request(e.to_string()))
}

// ── Connection + diagnostics ─────────────────────────────────────

pub async fn test_connection(
    Path(id): Path<String>,
) -> Result<Json<ServerStatusSnapshot>, AppError> {
    api::test_connection(&id)
        .await
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

pub async fn reconnect_server(
    Path(id): Path<String>,
) -> Result<Json<ServerStatusSnapshot>, AppError> {
    api::reconnect_server(&id)
        .await
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

pub async fn list_tools(Path(id): Path<String>) -> Result<Json<Vec<McpToolSummary>>, AppError> {
    api::list_server_tools(&id)
        .await
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

pub async fn get_recent_logs(
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<McpLogLine>>, AppError> {
    api::get_recent_logs(&id, q.limit.unwrap_or(200))
        .await
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

pub async fn import_claude_desktop_config(
    Json(payload): Json<ImportPayload>,
) -> Result<Json<ImportSummary>, AppError> {
    api::import_claude_desktop_config(&payload.json)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

// ── Global settings ──────────────────────────────────────────────

pub async fn get_global_settings() -> Result<Json<McpGlobalSettings>, AppError> {
    Ok(Json(api::get_global_settings()))
}

pub async fn update_global_settings(
    Json(settings): Json<McpGlobalSettings>,
) -> Result<Json<serde_json::Value>, AppError> {
    api::update_global_settings(settings)
        .await
        .map(|_| Json(serde_json::json!({ "ok": true })))
        .map_err(|e| AppError::bad_request(e.to_string()))
}
