use crate::commands::CmdError;
use crate::dashboard::{self, *};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn dashboard_overview(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<OverviewStats, CmdError> {
    query_overview(&state.session_db, &state.log_db, &state.cron_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_token_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTokenData, CmdError> {
    query_token_usage(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tool_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<ToolUsageStats>, CmdError> {
    query_tool_usage(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_sessions(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardSessionData, CmdError> {
    query_sessions(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_errors(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardErrorData, CmdError> {
    query_errors(&state.log_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tasks(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTaskData, CmdError> {
    query_tasks(&state.session_db, &state.cron_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_system_metrics() -> Result<dashboard::SystemMetrics, CmdError> {
    // Run on blocking thread since sysinfo does a brief sleep for CPU measurement
    tokio::task::spawn_blocking(|| dashboard::query_system_metrics())
        .await?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_session_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardSessionItem>, CmdError> {
    dashboard::query_session_list(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_message_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardMessageItem>, CmdError> {
    dashboard::query_message_list(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tool_call_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardToolCallItem>, CmdError> {
    dashboard::query_tool_call_list(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_error_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardErrorItem>, CmdError> {
    dashboard::query_error_list(&state.log_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_agent_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardAgentItem>, CmdError> {
    dashboard::query_agent_list(&state.session_db, &filter).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_overview_delta(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::OverviewStatsWithDelta, CmdError> {
    dashboard::query_overview_with_delta(&state.session_db, &state.log_db, &state.cron_db, &filter)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_insights(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::DashboardInsights, CmdError> {
    dashboard::query_insights(&state.session_db, &state.log_db, &state.cron_db, &filter)
        .map_err(Into::into)
}

// ── Phase B'4: Learning Dashboard ──────────────────────────────

#[tauri::command]
pub async fn dashboard_learning_overview(
    window_days: u32,
    state: State<'_, AppState>,
) -> Result<dashboard::LearningOverview, CmdError> {
    dashboard::query_learning_overview(&state.session_db, window_days).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_learning_timeline(
    window_days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::TimelinePoint>, CmdError> {
    dashboard::query_skill_timeline(&state.session_db, window_days).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_top_skills(
    window_days: u32,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::SkillUsage>, CmdError> {
    dashboard::query_top_skills(&state.session_db, window_days, limit).map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_recall_stats(
    window_days: u32,
    state: State<'_, AppState>,
) -> Result<dashboard::RecallStats, CmdError> {
    dashboard::query_recall_stats(&state.session_db, window_days).map_err(Into::into)
}
