use crate::commands::CmdError;
use crate::dashboard::{self, *};
use crate::AppState;
use ha_core::blocking::run_blocking;
use tauri::State;

#[tauri::command]
pub async fn dashboard_overview(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<OverviewStats, CmdError> {
    let log_db = state.log_db.clone();
    run_blocking(move || query_overview(&log_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_token_usage(
    filter: DashboardFilter,
) -> Result<DashboardTokenData, CmdError> {
    run_blocking(move || query_token_usage(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tool_usage(
    filter: DashboardFilter,
) -> Result<Vec<ToolUsageStats>, CmdError> {
    run_blocking(move || query_tool_usage(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_sessions(filter: DashboardFilter) -> Result<DashboardSessionData, CmdError> {
    run_blocking(move || query_sessions(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_errors(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardErrorData, CmdError> {
    let log_db = state.log_db.clone();
    run_blocking(move || query_errors(&log_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tasks(filter: DashboardFilter) -> Result<DashboardTaskData, CmdError> {
    run_blocking(move || query_tasks(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_control_plane(
    filter: ControlPlaneDashboardFilter,
) -> Result<ControlPlaneDashboard, CmdError> {
    run_blocking(move || query_control_plane_dashboard(&filter))
        .await
        .map_err(Into::into)
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
) -> Result<Vec<dashboard::DashboardSessionItem>, CmdError> {
    run_blocking(move || dashboard::query_session_list(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_message_list(
    filter: DashboardFilter,
) -> Result<Vec<dashboard::DashboardMessageItem>, CmdError> {
    run_blocking(move || dashboard::query_message_list(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tool_call_list(
    filter: DashboardFilter,
) -> Result<Vec<dashboard::DashboardToolCallItem>, CmdError> {
    run_blocking(move || dashboard::query_tool_call_list(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_error_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardErrorItem>, CmdError> {
    let log_db = state.log_db.clone();
    run_blocking(move || dashboard::query_error_list(&log_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_agent_list(
    filter: DashboardFilter,
) -> Result<Vec<dashboard::DashboardAgentItem>, CmdError> {
    run_blocking(move || dashboard::query_agent_list(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_overview_delta(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::OverviewStatsWithDelta, CmdError> {
    let log_db = state.log_db.clone();
    run_blocking(move || dashboard::query_overview_with_delta(&log_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_insights(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::DashboardInsights, CmdError> {
    let log_db = state.log_db.clone();
    run_blocking(move || dashboard::query_insights(&log_db, &filter))
        .await
        .map_err(Into::into)
}

// ── Phase B'4: Learning Dashboard ──────────────────────────────

#[tauri::command]
pub async fn dashboard_learning_overview(
    window_days: u32,
) -> Result<dashboard::LearningOverview, CmdError> {
    run_blocking(move || dashboard::query_learning_overview(window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_learning_timeline(
    window_days: u32,
) -> Result<Vec<dashboard::TimelinePoint>, CmdError> {
    run_blocking(move || dashboard::query_skill_timeline(window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_top_skills(
    window_days: u32,
    limit: usize,
) -> Result<Vec<dashboard::SkillUsage>, CmdError> {
    run_blocking(move || dashboard::query_top_skills(window_days, limit))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_recall_stats(window_days: u32) -> Result<dashboard::RecallStats, CmdError> {
    run_blocking(move || dashboard::query_recall_stats(window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_coding_improvement(
    filter: DashboardFilter,
    limit: Option<usize>,
) -> Result<dashboard::CodingImprovementDashboard, CmdError> {
    run_blocking(move || dashboard::query_coding_improvement_dashboard(&filter, limit.unwrap_or(8)))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_plan_stats(
    filter: DashboardFilter,
    _state: State<'_, AppState>,
) -> Result<dashboard::PlanStats, CmdError> {
    run_blocking(move || dashboard::query_plan_stats(&filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_local_model_usage(
    filter: DashboardFilter,
) -> Result<dashboard::LocalModelUsage, CmdError> {
    run_blocking(move || {
        let names = dashboard::local_provider_names();
        dashboard::query_local_model_usage(&filter, &names)
    })
    .await
    .map_err(Into::into)
}
