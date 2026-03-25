use tauri::State;
use crate::AppState;
use crate::dashboard::*;

#[tauri::command]
pub async fn dashboard_overview(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<OverviewStats, String> {
    query_overview(&state.session_db, &state.log_db, &state.cron_db, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_token_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTokenData, String> {
    query_token_usage(&state.session_db, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_tool_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<ToolUsageStats>, String> {
    query_tool_usage(&state.session_db, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_sessions(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardSessionData, String> {
    query_sessions(&state.session_db, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_errors(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardErrorData, String> {
    query_errors(&state.log_db, &filter)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_tasks(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTaskData, String> {
    query_tasks(&state.session_db, &state.cron_db, &filter)
        .map_err(|e| e.to_string())
}
