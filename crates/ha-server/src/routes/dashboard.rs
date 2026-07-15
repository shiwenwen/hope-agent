use axum::Json;
use serde::Deserialize;

use ha_core::blocking::run_blocking;
use ha_core::dashboard::{self, *};

use crate::error::AppError;
use crate::routes::helpers::{cron_db, log_db, session_db};

/// Body wrapper used by every dashboard route. Frontend ships
/// `{ filter: <DashboardFilter> }` to mirror the Tauri command's single
/// `filter:` parameter.
#[derive(Debug, Deserialize)]
pub struct FilterBody {
    pub filter: DashboardFilter,
}

#[derive(Debug, Deserialize)]
pub struct ControlPlaneFilterBody {
    pub filter: ControlPlaneDashboardFilter,
}

#[derive(Debug, Deserialize)]
pub struct FilterLimitBody {
    pub filter: DashboardFilter,
    #[serde(default = "default_limit")]
    pub limit: Option<usize>,
}

pub async fn overview(Json(body): Json<FilterBody>) -> Result<Json<OverviewStats>, AppError> {
    let (session_db, log_db, cron_db) = (session_db()?, log_db()?, cron_db()?);
    Ok(Json(
        run_blocking(move || query_overview(session_db, log_db, cron_db, &body.filter)).await?,
    ))
}

pub async fn token_usage(
    Json(body): Json<FilterBody>,
) -> Result<Json<DashboardTokenData>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || query_token_usage(session_db, &body.filter)).await?,
    ))
}

pub async fn tool_usage(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<ToolUsageStats>>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || query_tool_usage(session_db, &body.filter)).await?,
    ))
}

pub async fn sessions(
    Json(body): Json<FilterBody>,
) -> Result<Json<DashboardSessionData>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || query_sessions(session_db, &body.filter)).await?,
    ))
}

pub async fn errors(Json(body): Json<FilterBody>) -> Result<Json<DashboardErrorData>, AppError> {
    let log_db = log_db()?;
    Ok(Json(
        run_blocking(move || query_errors(log_db, &body.filter)).await?,
    ))
}

pub async fn tasks(Json(body): Json<FilterBody>) -> Result<Json<DashboardTaskData>, AppError> {
    let (session_db, cron_db) = (session_db()?, cron_db()?);
    Ok(Json(
        run_blocking(move || query_tasks(session_db, cron_db, &body.filter)).await?,
    ))
}

pub async fn control_plane(
    Json(body): Json<ControlPlaneFilterBody>,
) -> Result<Json<ControlPlaneDashboard>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || query_control_plane_dashboard(session_db, &body.filter)).await?,
    ))
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
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_session_list(session_db, &body.filter)).await?,
    ))
}

pub async fn message_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardMessageItem>>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_message_list(session_db, &body.filter)).await?,
    ))
}

pub async fn tool_call_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardToolCallItem>>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_tool_call_list(session_db, &body.filter)).await?,
    ))
}

pub async fn error_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardErrorItem>>, AppError> {
    let log_db = log_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_error_list(log_db, &body.filter)).await?,
    ))
}

pub async fn agent_list(
    Json(body): Json<FilterBody>,
) -> Result<Json<Vec<dashboard::DashboardAgentItem>>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_agent_list(session_db, &body.filter)).await?,
    ))
}

pub async fn overview_delta(
    Json(body): Json<FilterBody>,
) -> Result<Json<OverviewStatsWithDelta>, AppError> {
    let (session_db, log_db, cron_db) = (session_db()?, log_db()?, cron_db()?);
    Ok(Json(
        run_blocking(move || {
            dashboard::query_overview_with_delta(session_db, log_db, cron_db, &body.filter)
        })
        .await?,
    ))
}

pub async fn insights(Json(body): Json<FilterBody>) -> Result<Json<DashboardInsights>, AppError> {
    let (session_db, log_db, cron_db) = (session_db()?, log_db()?, cron_db()?);
    Ok(Json(
        run_blocking(move || dashboard::query_insights(session_db, log_db, cron_db, &body.filter))
            .await?,
    ))
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
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_learning_overview(session_db, body.window_days))
            .await?,
    ))
}

pub async fn learning_timeline(
    Json(body): Json<WindowBody>,
) -> Result<Json<Vec<dashboard::TimelinePoint>>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_skill_timeline(session_db, body.window_days)).await?,
    ))
}

pub async fn top_skills(
    Json(body): Json<WindowBody>,
) -> Result<Json<Vec<dashboard::SkillUsage>>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || {
            dashboard::query_top_skills(session_db, body.window_days, body.limit.unwrap_or(10))
        })
        .await?,
    ))
}

pub async fn recall_stats(
    Json(body): Json<WindowBody>,
) -> Result<Json<dashboard::RecallStats>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || dashboard::query_recall_stats(session_db, body.window_days)).await?,
    ))
}

pub async fn coding_improvement(
    Json(body): Json<FilterLimitBody>,
) -> Result<Json<dashboard::CodingImprovementDashboard>, AppError> {
    Ok(Json(dashboard::query_coding_improvement_dashboard(
        session_db()?,
        &body.filter,
        body.limit.unwrap_or(10),
    )?))
}

pub async fn plan_stats(
    Json(body): Json<FilterBody>,
) -> Result<Json<dashboard::PlanStats>, AppError> {
    Ok(Json(
        run_blocking(move || dashboard::query_plan_stats(&body.filter)).await?,
    ))
}

pub async fn local_model_usage(
    Json(body): Json<FilterBody>,
) -> Result<Json<dashboard::LocalModelUsage>, AppError> {
    let session_db = session_db()?;
    Ok(Json(
        run_blocking(move || {
            let names = dashboard::local_provider_names();
            dashboard::query_local_model_usage(session_db, &body.filter, &names)
        })
        .await?,
    ))
}
