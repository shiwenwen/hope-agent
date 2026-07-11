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
    let session_db = state.session_db.clone();
    let log_db = state.log_db.clone();
    let cron_db = state.cron_db.clone();
    run_blocking(move || query_overview(&session_db, &log_db, &cron_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_token_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTokenData, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || query_token_usage(&session_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tool_usage(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<ToolUsageStats>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || query_tool_usage(&session_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_sessions(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardSessionData, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || query_sessions(&session_db, &filter))
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
pub async fn dashboard_tasks(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<DashboardTaskData, CmdError> {
    let session_db = state.session_db.clone();
    let cron_db = state.cron_db.clone();
    run_blocking(move || query_tasks(&session_db, &cron_db, &filter))
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
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardSessionItem>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_session_list(&session_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_message_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardMessageItem>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_message_list(&session_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_tool_call_list(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardToolCallItem>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_tool_call_list(&session_db, &filter))
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
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::DashboardAgentItem>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_agent_list(&session_db, &filter))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_overview_delta(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::OverviewStatsWithDelta, CmdError> {
    let session_db = state.session_db.clone();
    let log_db = state.log_db.clone();
    let cron_db = state.cron_db.clone();
    run_blocking(move || {
        dashboard::query_overview_with_delta(&session_db, &log_db, &cron_db, &filter)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_insights(
    filter: DashboardFilter,
    state: State<'_, AppState>,
) -> Result<dashboard::DashboardInsights, CmdError> {
    let session_db = state.session_db.clone();
    let log_db = state.log_db.clone();
    let cron_db = state.cron_db.clone();
    run_blocking(move || dashboard::query_insights(&session_db, &log_db, &cron_db, &filter))
        .await
        .map_err(Into::into)
}

// ── Phase B'4: Learning Dashboard ──────────────────────────────

#[tauri::command]
pub async fn dashboard_learning_overview(
    window_days: u32,
    state: State<'_, AppState>,
) -> Result<dashboard::LearningOverview, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_learning_overview(&session_db, window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_learning_timeline(
    window_days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::TimelinePoint>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_skill_timeline(&session_db, window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_top_skills(
    window_days: u32,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<dashboard::SkillUsage>, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_top_skills(&session_db, window_days, limit))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_recall_stats(
    window_days: u32,
    state: State<'_, AppState>,
) -> Result<dashboard::RecallStats, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || dashboard::query_recall_stats(&session_db, window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dashboard_coding_improvement(
    filter: DashboardFilter,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<dashboard::CodingImprovementDashboard, CmdError> {
    dashboard::query_coding_improvement_dashboard(&state.session_db, &filter, limit.unwrap_or(8))
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
    state: State<'_, AppState>,
) -> Result<dashboard::LocalModelUsage, CmdError> {
    let session_db = state.session_db.clone();
    run_blocking(move || {
        let names = dashboard::local_provider_names();
        dashboard::query_local_model_usage(&session_db, &filter, &names)
    })
    .await
    .map_err(Into::into)
}
