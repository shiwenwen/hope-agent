//! Thin `#[tauri::command]` wrappers for oc-core functions.
//!
//! oc-core's business logic functions don't have `#[tauri::command]` attributes
//! (to stay Tauri-independent). This module provides the thin Tauri command layer.

use crate::AppState;
use tauri::State;

// ── Permissions ──────────────────────────────────────────────────

#[tauri::command]
pub async fn check_all_permissions() -> oc_core::permissions::AllPermissions {
    oc_core::permissions::check_all_permissions().await
}

#[tauri::command]
pub async fn check_permission(id: String) -> oc_core::permissions::PermissionStatus {
    oc_core::permissions::check_permission(id).await
}

#[tauri::command]
pub async fn request_permission(id: String) -> oc_core::permissions::PermissionStatus {
    oc_core::permissions::request_permission(id).await
}

// ── Sandbox ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_sandbox_config() -> Result<oc_core::sandbox::SandboxConfig, String> {
    oc_core::sandbox::get_sandbox_config().await
}

#[tauri::command]
pub async fn set_sandbox_config(config: oc_core::sandbox::SandboxConfig) -> Result<(), String> {
    oc_core::sandbox::set_sandbox_config(config).await
}

#[tauri::command]
pub async fn check_sandbox_available() -> oc_core::sandbox::DockerStatus {
    oc_core::sandbox::check_sandbox_available().await
}

// ── Slash Commands ───────────────────────────────────────────────
// oc-core's slash_commands take `&AppState`, but Tauri commands receive `State<'_, AppState>`.

#[tauri::command]
pub async fn list_slash_commands(
    state: State<'_, AppState>,
) -> Result<Vec<oc_core::slash_commands::types::SlashCommandDef>, String> {
    oc_core::slash_commands::list_slash_commands(&state).await
}

#[tauri::command]
pub async fn execute_slash_command(
    state: State<'_, AppState>,
    session_id: Option<String>,
    agent_id: String,
    command_text: String,
) -> Result<oc_core::slash_commands::types::CommandResult, String> {
    oc_core::slash_commands::execute_slash_command(&state, session_id, agent_id, command_text).await
}

#[tauri::command]
pub fn is_slash_command(text: String) -> bool {
    oc_core::slash_commands::is_slash_command(text)
}

// ── Canvas ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn canvas_submit_snapshot(
    request_id: String,
    data_url: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    oc_core::tools::canvas::canvas_submit_snapshot(request_id, data_url, error).await
}

#[tauri::command]
pub async fn canvas_submit_eval_result(
    request_id: String,
    result: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    oc_core::tools::canvas::canvas_submit_eval_result(request_id, result, error).await
}

#[tauri::command]
pub async fn get_canvas_config() -> Result<oc_core::tools::canvas::CanvasConfig, String> {
    oc_core::tools::canvas::get_canvas_config().await
}

#[tauri::command]
pub async fn save_canvas_config(config: oc_core::tools::canvas::CanvasConfig) -> Result<(), String> {
    oc_core::tools::canvas::save_canvas_config(config).await
}

#[tauri::command]
pub async fn list_canvas_projects() -> Result<String, String> {
    oc_core::tools::canvas::list_canvas_projects().await
}

#[tauri::command]
pub async fn get_canvas_project(project_id: String) -> Result<String, String> {
    oc_core::tools::canvas::get_canvas_project(project_id).await
}

#[tauri::command]
pub async fn delete_canvas_project(project_id: String) -> Result<(), String> {
    oc_core::tools::canvas::delete_canvas_project(project_id).await
}

#[tauri::command]
pub async fn show_canvas_panel(project_id: String) -> Result<(), String> {
    oc_core::tools::canvas::show_canvas_panel(project_id).await
}

// ── Developer Tools ──────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_sessions() -> Result<(), String> {
    oc_core::dev_tools::dev_clear_sessions().await
}

#[tauri::command]
pub async fn dev_clear_cron() -> Result<(), String> {
    oc_core::dev_tools::dev_clear_cron().await
}

#[tauri::command]
pub async fn dev_clear_memory() -> Result<(), String> {
    oc_core::dev_tools::dev_clear_memory().await
}

#[tauri::command]
pub async fn dev_reset_config() -> Result<(), String> {
    oc_core::dev_tools::dev_reset_config().await
}

#[tauri::command]
pub async fn dev_clear_all() -> Result<(), String> {
    oc_core::dev_tools::dev_clear_all().await
}
