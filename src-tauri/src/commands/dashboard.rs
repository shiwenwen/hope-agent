use crate::dashboard::{self, *};
use crate::AppState;
use tauri::State;

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
    query_token_usage(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_tool_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<ToolUsageStats>, String> {
    query_tool_usage(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_sessions(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardSessionData, String> {
    query_sessions(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_errors(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardErrorData, String> {
    query_errors(&state.log_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_tasks(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTaskData, String> {
    query_tasks(&state.session_db, &state.cron_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_system_metrics() -> Result<dashboard::SystemMetrics, String> {
    // Run on blocking thread since sysinfo does a brief sleep for CPU measurement
    tokio::task::spawn_blocking(|| dashboard::query_system_metrics())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_session_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardSessionItem>, String> {
    dashboard::query_session_list(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_message_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardMessageItem>, String> {
    dashboard::query_message_list(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_tool_call_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardToolCallItem>, String> {
    dashboard::query_tool_call_list(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_error_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardErrorItem>, String> {
    dashboard::query_error_list(&state.log_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_agent_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardAgentItem>, String> {
    dashboard::query_agent_list(&state.session_db, &filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_overview_delta(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::OverviewStatsWithDelta, String> {
    dashboard::query_overview_with_delta(
        &state.session_db,
        &state.log_db,
        &state.cron_db,
        &filter,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard_insights(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::DashboardInsights, String> {
    dashboard::query_insights(
        &state.session_db,
        &state.log_db,
        &state.cron_db,
        &filter,
    )
    .map_err(|e| e.to_string())
}
