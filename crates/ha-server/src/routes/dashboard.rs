use axum::Json;
use serde::Deserialize;

use ha_core::dashboard::{self, *};

use crate::error::AppError;
use crate::routes::helpers::app_state;

/// Body wrapper used by every dashboard route. Frontend ships
/// `{ filter: <DashboardFilter> }` to mirror the Tauri command's single
/// `filter:` parameter.
#[derive(Debug, Deserialize)]
pub struct FilterBody {
    pub filter: DashboardFilter,
}

pub async fn overview(Json(body): Json<FilterBody>) -> Result<Json<OverviewStats>, AppError> {
    let s = app_state()?;
    Ok(Json(query_overview(
        &s.session_db,
        &s.log_db,
        &s.cron_db,
        &body.filter,
    )?))
}

pub async fn token_usage(
    Json(body): Json<FilterBody>,
) -> Result<Json<DashboardTokenData>, AppError> {
    Ok(Json(query_token_usage(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn tool_usage(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<ToolUsageStats>>, AppError> {
    Ok(Json(query_tool_usage(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn sessions(
    Json(body): Json<FilterBody>,
) -> Result<Json<DashboardSessionData>, AppError> {
    Ok(Json(query_sessions(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn errors(Json(body): Json<FilterBody>) -> Result<Json<DashboardErrorData>, AppError> {
    Ok(Json(query_errors(&app_state()?.log_db, &body.filter)?))
}

pub async fn tasks(Json(body): Json<FilterBody>) -> Result<Json<DashboardTaskData>, AppError> {
    let s = app_state()?;
    Ok(Json(query_tasks(&s.session_db, &s.cron_db, &body.filter)?))
}

pub async fn system_metrics() -> Result<Json<dashboard::SystemMetrics>, AppError> {
    let metrics = tokio::task::spawn_blocking(dashboard::query_system_metrics)
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
    Ok(Json(metrics))
}

pub async fn session_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardSessionItem>>, AppError> {
    Ok(Json(dashboard::query_session_list(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn message_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardMessageItem>>, AppError> {
    Ok(Json(dashboard::query_message_list(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn tool_call_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardToolCallItem>>, AppError> {
    Ok(Json(dashboard::query_tool_call_list(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn error_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardErrorItem>>, AppError> {
    Ok(Json(dashboard::query_error_list(
        &app_state()?.log_db,
        &body.filter,
    )?))
}

pub async fn agent_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardAgentItem>>, AppError> {
    Ok(Json(dashboard::query_agent_list(
        &app_state()?.session_db,
        &body.filter,
    )?))
}

pub async fn overview_delta(
    Json(body): Json<FilterBody>,
) -> Result<Json<OverviewStatsWithDelta>, AppError> {
    let s = app_state()?;
    Ok(Json(dashboard::query_overview_with_delta(
        &s.session_db,
        &s.log_db,
        &s.cron_db,
        &body.filter,
    )?))
}

pub async fn insights(Json(body): Json<FilterBody>) -> Result<Json<DashboardInsights>, AppError> {
    let s = app_state()?;
    Ok(Json(dashboard::query_insights(
        &s.session_db,
        &s.log_db,
        &s.cron_db,
        &body.filter,
    )?))
}

// ── Phase B'4: Learning Dashboard ──────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WindowBody {
    #[serde(
        rename = "windowDays",
        alias = "window_days",
        default = "default_window"
    )]
    pub window_days: u32,
    #[serde(default = "default_limit")]
    pub limit: Option<usize>,
}

fn default_window() -> u32 {
    30
}
fn default_limit() -> Option<usize> {
    Some(10)
}

pub async fn learning_overview(
    Json(body): Json<WindowBody>,
) -> Result<Json<dashboard::LearningOverview>, AppError> {
    Ok(Json(dashboard::query_learning_overview(
        &app_state()?.session_db,
        body.window_days,
    )?))
}

pub async fn learning_timeline(
    Json(body): Json<WindowBody>,
) -> Result<Json<Vec<dashboard::TimelinePoint>>, AppError> {
    Ok(Json(dashboard::query_skill_timeline(
        &app_state()?.session_db,
        body.window_days,
    )?))
}

pub async fn top_skills(
    Json(body): Json<WindowBody>,
) -> Result<Json<Vec<dashboard::SkillUsage>>, AppError> {
    Ok(Json(dashboard::query_top_skills(
        &app_state()?.session_db,
        body.window_days,
        body.limit.unwrap_or(10),
    )?))
}

pub async fn recall_stats(
    Json(body): Json<WindowBody>,
) -> Result<Json<dashboard::RecallStats>, AppError> {
    Ok(Json(dashboard::query_recall_stats(
        &app_state()?.session_db,
        body.window_days,
    )?))
}
