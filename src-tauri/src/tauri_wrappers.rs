//! Thin `#[tauri::command]` wrappers for ha-core functions.
//!
//! ha-core's business logic functions don't have `#[tauri::command]` attributes
//! (to stay Tauri-independent). This module provides the thin Tauri command layer.

// ── Permissions ──────────────────────────────────────────────────

#[tauri::command]
pub async fn check_all_permissions() -> ha_core::permissions::AllPermissions {
    ha_core::permissions::check_all_permissions().await
}

#[tauri::command]
pub async fn check_permission(id: String) -> ha_core::permissions::PermissionStatus {
    ha_core::permissions::check_permission(id).await
}

#[tauri::command]
pub async fn request_permission(id: String) -> ha_core::permissions::PermissionStatus {
    ha_core::permissions::request_permission(id).await
}

// ── Sandbox ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_sandbox_config() -> Result<ha_core::sandbox::SandboxConfig, String> {
    ha_core::sandbox::get_sandbox_config().await
}

#[tauri::command]
pub async fn set_sandbox_config(config: ha_core::sandbox::SandboxConfig) -> Result<(), String> {
    ha_core::sandbox::set_sandbox_config(config).await
}

#[tauri::command]
pub async fn check_sandbox_available() -> ha_core::sandbox::DockerStatus {
    ha_core::sandbox::check_sandbox_available().await
}

// ── Slash Commands ───────────────────────────────────────────────
// ha-core's slash_commands read cross-runtime singletons (SessionDB,
// cached agent, etc.) via OnceLock accessors, so these wrappers are
// pure Tauri-surface adapters with no `State<AppState>` dependency.

#[tauri::command]
pub async fn list_slash_commands(
) -> Result<Vec<ha_core::slash_commands::types::SlashCommandDef>, String> {
    ha_core::slash_commands::list_slash_commands().await
}

#[tauri::command]
pub async fn execute_slash_command(
    session_id: Option<String>,
    agent_id: String,
    command_text: String,
) -> Result<ha_core::slash_commands::types::CommandResult, String> {
    ha_core::slash_commands::execute_slash_command(session_id, agent_id, command_text).await
}

#[tauri::command]
pub fn is_slash_command(text: String) -> bool {
    ha_core::slash_commands::is_slash_command(text)
}

// ── Canvas ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn canvas_submit_snapshot(
    request_id: String,
    data_url: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    ha_core::tools::canvas::canvas_submit_snapshot(request_id, data_url, error).await
}

#[tauri::command]
pub async fn canvas_submit_eval_result(
    request_id: String,
    result: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    ha_core::tools::canvas::canvas_submit_eval_result(request_id, result, error).await
}

#[tauri::command]
pub async fn get_canvas_config() -> Result<ha_core::tools::canvas::CanvasConfig, String> {
    ha_core::tools::canvas::get_canvas_config().await
}

#[tauri::command]
pub async fn save_canvas_config(
    config: ha_core::tools::canvas::CanvasConfig,
) -> Result<(), String> {
    ha_core::tools::canvas::save_canvas_config(config).await
}

#[tauri::command]
pub async fn list_canvas_projects() -> Result<String, String> {
    ha_core::tools::canvas::list_canvas_projects().await
}

#[tauri::command]
pub async fn list_canvas_projects_by_session(
    session_id: String,
) -> Result<Vec<ha_core::tools::canvas::CanvasProjectView>, String> {
    ha_core::tools::canvas::list_canvas_projects_by_session(session_id).await
}

#[tauri::command]
pub async fn get_canvas_project(project_id: String) -> Result<String, String> {
    ha_core::tools::canvas::get_canvas_project(project_id).await
}

#[tauri::command]
pub async fn delete_canvas_project(project_id: String) -> Result<(), String> {
    ha_core::tools::canvas::delete_canvas_project(project_id).await
}

#[tauri::command]
pub async fn show_canvas_panel(project_id: String) -> Result<(), String> {
    ha_core::tools::canvas::show_canvas_panel(project_id).await
}

// ── Developer Tools ──────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_sessions() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_sessions().await
}

#[tauri::command]
pub async fn dev_clear_cron() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_cron().await
}

#[tauri::command]
pub async fn dev_clear_memory() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_memory().await
}

#[tauri::command]
pub async fn dev_reset_config() -> Result<(), String> {
    ha_core::dev_tools::dev_reset_config().await
}

#[tauri::command]
pub async fn dev_clear_all() -> Result<(), String> {
    ha_core::dev_tools::dev_clear_all().await
}
