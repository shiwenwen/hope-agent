use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use oc_core::cron;

use crate::error::AppError;
use crate::routes::helpers::{cron_db as db, session_db};

/// `GET /api/cron/jobs`
pub async fn list_jobs() -> Result<Json<Vec<cron::CronJob>>, AppError> {
    Ok(Json(db()?.list_jobs()?))
}

/// `GET /api/cron/jobs/{id}`
pub async fn get_job(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let job = db()?.get_job(&id)?;
    Ok(Json(serde_json::to_value(job)?))
}

/// Body wrapper used by `cron_create_job` / `cron_update_job` — frontend
/// ships `{ job: <CronJob> }` to mirror the Tauri command's single
/// `job:` parameter.
#[derive(Debug, Deserialize)]
pub struct CreateJobBody {
    pub job: cron::NewCronJob,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJobBody {
    pub job: cron::CronJob,
}

/// `POST /api/cron/jobs`
pub async fn create_job(Json(body): Json<CreateJobBody>) -> Result<Json<cron::CronJob>, AppError> {
    Ok(Json(db()?.add_job(&body.job)?))
}

/// `PUT /api/cron/jobs/{id}`
pub async fn update_job(
    Path(_id): Path<String>,
    Json(body): Json<UpdateJobBody>,
) -> Result<Json<Value>, AppError> {
    db()?.update_job(&body.job)?;
    Ok(Json(json!({ "updated": true })))
}

/// `DELETE /api/cron/jobs/{id}`
pub async fn delete_job(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    db()?.delete_job(&id)?;
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Debug, Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}

/// `POST /api/cron/jobs/{id}/toggle`
pub async fn toggle_job(
    Path(id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> Result<Json<Value>, AppError> {
    db()?.toggle_job(&id, body.enabled)?;
    Ok(Json(json!({ "toggled": true })))
}

/// `POST /api/cron/jobs/{id}/run`
pub async fn run_now(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let job = db()?
        .get_job(&id)?
        .ok_or_else(|| AppError::not_found(format!("job not found: {}", id)))?;
    let cdb = db()?.clone();
    let sdb = session_db()?.clone();
    tokio::spawn(async move {
        cron::execute_job_public(&cdb, &sdb, &job).await;
    });
    Ok(Json(json!({ "scheduled": true })))
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub limit: Option<usize>,
}

/// `GET /api/cron/jobs/{id}/logs`
pub async fn get_run_logs(
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<cron::CronRunLog>>, AppError> {
    Ok(Json(db()?.get_run_logs(&id, q.limit.unwrap_or(50))?))
}

#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    pub start: String,
    pub end: String,
}

/// `GET /api/cron/calendar?start=...&end=...`
pub async fn get_calendar_events(
    Query(q): Query<CalendarQuery>,
) -> Result<Json<Vec<cron::CalendarEvent>>, AppError> {
    let start_dt = chrono::DateTime::parse_from_rfc3339(&q.start)
        .map_err(|e| AppError::bad_request(format!("Invalid start date: {}", e)))?
        .with_timezone(&chrono::Utc);
    let end_dt = chrono::DateTime::parse_from_rfc3339(&q.end)
        .map_err(|e| AppError::bad_request(format!("Invalid end date: {}", e)))?
        .with_timezone(&chrono::Utc);
    Ok(Json(db()?.get_calendar_events(&start_dt, &end_dt)?))
}
