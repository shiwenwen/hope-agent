use axum::Json;

use oc_core::dashboard::{self, *};

use crate::error::AppError;
use crate::routes::helpers::app_state;

pub async fn overview(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<OverviewStats>, AppError> {
    let s = app_state()?;
    Ok(Json(query_overview(
        &s.session_db,
        &s.log_db,
        &s.cron_db,
        &filter,
    )?))
}

pub async fn token_usage(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<DashboardTokenData>, AppError> {
    Ok(Json(query_token_usage(&app_state()?.session_db, &filter)?))
}

pub async fn tool_usage(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<Vec<ToolUsageStats>>, AppError> {
    Ok(Json(query_tool_usage(&app_state()?.session_db, &filter)?))
}

pub async fn sessions(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<DashboardSessionData>, AppError> {
    Ok(Json(query_sessions(&app_state()?.session_db, &filter)?))
}

pub async fn errors(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<DashboardErrorData>, AppError> {
    Ok(Json(query_errors(&app_state()?.log_db, &filter)?))
}

pub async fn tasks(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<DashboardTaskData>, AppError> {
    let s = app_state()?;
    Ok(Json(query_tasks(&s.session_db, &s.cron_db, &filter)?))
}

pub async fn system_metrics() -> Result<Json<dashboard::SystemMetrics>, AppError> {
    let metrics = tokio::task::spawn_blocking(dashboard::query_system_metrics)
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
    Ok(Json(metrics))
}

pub async fn session_list(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<Vec<dashboard::DashboardSessionItem>>, AppError> {
    Ok(Json(dashboard::query_session_list(
        &app_state()?.session_db,
        &filter,
    )?))
}

pub async fn message_list(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<Vec<dashboard::DashboardMessageItem>>, AppError> {
    Ok(Json(dashboard::query_message_list(
        &app_state()?.session_db,
        &filter,
    )?))
}

pub async fn tool_call_list(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<Vec<dashboard::DashboardToolCallItem>>, AppError> {
    Ok(Json(dashboard::query_tool_call_list(
        &app_state()?.session_db,
        &filter,
    )?))
}

pub async fn error_list(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<Vec<dashboard::DashboardErrorItem>>, AppError> {
    Ok(Json(dashboard::query_error_list(
        &app_state()?.log_db,
        &filter,
    )?))
}

pub async fn agent_list(
    Json(filter): Json<DashboardFilter>,
) -> Result<Json<Vec<dashboard::DashboardAgentItem>>, AppError> {
    Ok(Json(dashboard::query_agent_list(
        &app_state()?.session_db,
        &filter,
    )?))
}
