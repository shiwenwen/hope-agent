use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::blocking::run_blocking;
use ha_core::cron;

use crate::error::AppError;
use crate::routes::helpers::{cron_db as db, session_db};

fn managed_workspace_write_policy() -> ha_cron::cron::CronManagedWorkspaceWritePolicy {
    if ha_core::config::cached_config()
        .filesystem
        .allow_remote_writes
    {
        ha_cron::cron::CronManagedWorkspaceWritePolicy::Allowed
    } else {
        ha_cron::cron::CronManagedWorkspaceWritePolicy::Denied
    }
}

fn ensure_workspace_writes_allowed() -> Result<(), AppError> {
    if managed_workspace_write_policy() == ha_cron::cron::CronManagedWorkspaceWritePolicy::Allowed {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "remote workspace writes are disabled; enable filesystem.allowRemoteWrites to allow them",
        ))
    }
}

fn ensure_workspace_policy_allowed(policy: &cron::CronWorkspacePolicy) -> Result<(), AppError> {
    if policy.mode == cron::CronWorkspaceMode::Project {
        Ok(())
    } else {
        ensure_workspace_writes_allowed()
    }
}

fn workspace_conflict(error: anyhow::Error) -> AppError {
    AppError::conflict_with_code(
        ha_cron::cron::workspace_error_code(&error),
        error.to_string(),
    )
}

/// `GET /api/cron/jobs`
pub async fn list_jobs() -> Result<Json<Vec<ha_core::loop_control::CronJobView>>, AppError> {
    let (cdb, sdb) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || sdb.list_cron_job_views(cdb)).await?,
    ))
}

/// `GET /api/cron/jobs/{id}`
pub async fn get_job(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let db = db()?;
    let job = run_blocking(move || db.get_job(&id)).await?;
    Ok(Json(serde_json::to_value(job)?))
}

#[derive(Debug, Deserialize)]
pub struct PreflightBody {
    pub request: ha_cron::cron::CronPreflightRequest,
}

/// `POST /api/cron/preflight` — read-only candidate/live execution preview.
pub async fn preflight(
    Json(body): Json<PreflightBody>,
) -> Result<Json<ha_cron::cron::CronPreflightReport>, AppError> {
    Ok(Json(
        ha_cron::cron::evaluate_cron_preflight(
            db()?.clone(),
            session_db()?.clone(),
            body.request,
            managed_workspace_write_policy(),
        )
        .await?,
    ))
}

/// Body wrapper used by `cron_create_job` / `cron_update_job` — frontend
/// ships `{ job: <CronJob> }` to mirror the Tauri command's single
/// `job:` parameter.
#[derive(Debug, Deserialize)]
pub struct CreateJobBody {
    pub job: cron::NewCronJob,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateJobBody {
    pub job: cron::CronJob,
    pub expected_revision: u64,
}

/// `POST /api/cron/jobs`
pub async fn create_job(Json(body): Json<CreateJobBody>) -> Result<Json<cron::CronJob>, AppError> {
    ensure_workspace_policy_allowed(&body.job.workspace_policy)?;
    let report = ha_cron::cron::evaluate_cron_preflight(
        db()?.clone(),
        session_db()?.clone(),
        ha_cron::cron::CronPreflightRequest::Create {
            job: body.job.clone(),
        },
        managed_workspace_write_policy(),
    )
    .await?;
    if !report.can_proceed {
        return Err(AppError::conflict_with_code(
            "cron_preflight_blocked",
            "Cron preflight blocked",
        ));
    }
    let db = db()?;
    Ok(Json(run_blocking(move || db.add_job(&body.job)).await?))
}

/// `PUT /api/cron/jobs/{id}`
pub async fn update_job(
    Path(id): Path<String>,
    Json(body): Json<UpdateJobBody>,
) -> Result<Response, AppError> {
    if body.job.id != id {
        return Err(AppError::bad_request("path id does not match job id"));
    }
    ensure_workspace_policy_allowed(&body.job.workspace_policy)?;
    let report = ha_cron::cron::evaluate_cron_preflight(
        db()?.clone(),
        session_db()?.clone(),
        ha_cron::cron::CronPreflightRequest::Update {
            job: body.job.clone(),
            expected_revision: body.expected_revision,
        },
        managed_workspace_write_policy(),
    )
    .await?;
    if !report.can_proceed {
        return Err(AppError::conflict_with_code(
            "cron_preflight_blocked",
            "Cron preflight blocked",
        ));
    }
    let db = db()?;
    let result = run_blocking(move || db.update_job_cas(&body.job, body.expected_revision)).await?;
    if result.updated {
        Ok(Json(result).into_response())
    } else {
        Ok((StatusCode::CONFLICT, Json(result)).into_response())
    }
}

/// `POST /api/cron/runs/{runLogId}/cancel` — exact occurrence cancel.
pub async fn cancel_run(
    Path(run_log_id): Path<i64>,
) -> Result<Json<cron::CronRunCancelResult>, AppError> {
    let result = ha_cron::cron::cancel_run(run_log_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("cron run not found: {run_log_id}")))?;
    Ok(Json(result))
}

/// `DELETE /api/cron/jobs/{id}` — logical delete; history/chats are retained.
pub async fn delete_job(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let (cdb, sdb) = (db()?, session_db()?);
    run_blocking(move || ha_cron::cron::delete_job_and_legacy_sessions(cdb, sdb, &id)).await?;
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
    let db = db()?;
    if body.enabled {
        let lookup_db = db.clone();
        let lookup_id = id.clone();
        if let Some(job) = run_blocking(move || lookup_db.get_job(&lookup_id)).await? {
            ensure_workspace_policy_allowed(&job.workspace_policy)?;
        }
    }
    run_blocking(move || db.toggle_job(&id, body.enabled)).await?;
    Ok(Json(json!({ "toggled": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunNowBody {
    pub expected_revision: u64,
}

/// `POST /api/cron/jobs/{id}/run` — live preflight + exact synchronous claim.
pub async fn run_now(
    Path(id): Path<String>,
    Json(body): Json<RunNowBody>,
) -> Result<Json<ha_cron::cron::CronRunNowResult>, AppError> {
    Ok(Json(
        ha_cron::cron::start_cron_run_now(
            db()?.clone(),
            session_db()?.clone(),
            id,
            body.expected_revision,
            managed_workspace_write_policy(),
        )
        .await?,
    ))
}

/// `GET /api/cron/jobs-referencing-account/{accountId}` — §8: cron jobs whose
/// delivery targets reference the channel account, for the delete confirmation.
pub async fn jobs_referencing_account(
    Path(account_id): Path<String>,
) -> Result<Json<Vec<cron::CronAccountRef>>, AppError> {
    let db = db()?;
    Ok(Json(
        run_blocking(move || db.jobs_referencing_account(&account_id)).await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// `GET /api/cron/jobs/{id}/logs`
pub async fn get_run_logs(
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<cron::CronRunLog>>, AppError> {
    let db = db()?;
    let sessions = session_db()?;
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::visible_cron_run_logs(
                &db,
                &sessions,
                &id,
                q.limit.unwrap_or(50).min(200),
                q.offset.unwrap_or(0),
            )
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// `GET /api/cron/timeline?limit=&offset=` — cross-job run timeline, including
/// retained rows from logically deleted tasks (`jobDeleted=true`).
pub async fn run_timeline(
    Query(q): Query<TimelineQuery>,
) -> Result<Json<Vec<cron::CronTimelineRow>>, AppError> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let (cdb, sdb) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || ha_cron::cron::cron_run_timeline(cdb, sdb, limit, offset)).await?,
    ))
}

/// `GET /api/cron/unread` — total unread across all cron sessions (badge).
/// Returns a bare number to mirror the Tauri `cron_unread_total` command shape.
pub async fn unread_total() -> Result<Json<i64>, AppError> {
    Ok(Json(session_db()?.run(|db| db.cron_unread_total()).await?))
}

/// `POST /api/cron/read-all` — mark every cron session read (badge → 0).
/// Returns the count of updated sessions (mirrors the Tauri command shape).
pub async fn mark_all_read() -> Result<Json<usize>, AppError> {
    let n = session_db()?
        .run(|db| db.mark_all_cron_sessions_read())
        .await?;
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit("cron:unread_changed", json!({ "total": 0 }));
    }
    Ok(Json(n))
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
    let db = db()?;
    Ok(Json(
        run_blocking(move || db.get_calendar_events(&start_dt, &end_dt)).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResourcesQuery {
    pub job_id: Option<String>,
}

pub async fn workspace_resources(
    Query(query): Query<WorkspaceResourcesQuery>,
) -> Result<Json<Vec<ha_cron::cron::CronWorkspaceResource>>, AppError> {
    let (cron_db, sessions) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::workspace_resources(&cron_db, &sessions, query.job_id.as_deref())
        })
        .await?,
    ))
}

pub async fn workspace_resource_for_run(
    Path(run_log_id): Path<i64>,
) -> Result<Json<Option<ha_cron::cron::CronWorkspaceResource>>, AppError> {
    let (cron_db, sessions) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::workspace_resource_for_run(&cron_db, &sessions, run_log_id)
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSessionBody {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceReturnBody {
    pub session_id: String,
    #[serde(default)]
    pub resume: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiscardRunBody {
    pub session_id: String,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceDiscardTaskBody {
    #[serde(default)]
    pub confirm: bool,
}

pub async fn workspace_takeover(
    Path(job_id): Path<String>,
    Json(body): Json<WorkspaceSessionBody>,
) -> Result<Json<ha_cron::cron::CronWorkspaceActionResult>, AppError> {
    ensure_workspace_writes_allowed()?;
    let (cron_db, sessions) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::take_over_persistent_worktree(
                &cron_db,
                &sessions,
                &job_id,
                &body.session_id,
            )
        })
        .await
        .map_err(workspace_conflict)?,
    ))
}

pub async fn workspace_return(
    Path(job_id): Path<String>,
    Json(body): Json<WorkspaceReturnBody>,
) -> Result<Json<ha_cron::cron::CronWorkspaceActionResult>, AppError> {
    ensure_workspace_writes_allowed()?;
    let (cron_db, sessions) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::return_persistent_worktree(
                &cron_db,
                &sessions,
                &job_id,
                &body.session_id,
                body.resume,
            )
        })
        .await
        .map_err(workspace_conflict)?,
    ))
}

pub async fn workspace_discard_run(
    Path(run_log_id): Path<i64>,
    Json(body): Json<WorkspaceDiscardRunBody>,
) -> Result<Json<ha_cron::cron::CronWorkspaceActionResult>, AppError> {
    ensure_workspace_writes_allowed()?;
    if !body.confirm {
        return Err(AppError::bad_request(
            "discard requires explicit confirmation",
        ));
    }
    let (cron_db, sessions) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::discard_run_worktree(&cron_db, &sessions, run_log_id, &body.session_id)
        })
        .await
        .map_err(workspace_conflict)?,
    ))
}

pub async fn workspace_discard_task(
    Path(job_id): Path<String>,
    Json(body): Json<WorkspaceDiscardTaskBody>,
) -> Result<Json<ha_cron::cron::CronWorkspaceActionResult>, AppError> {
    ensure_workspace_writes_allowed()?;
    if !body.confirm {
        return Err(AppError::bad_request(
            "discard requires explicit confirmation",
        ));
    }
    let (cron_db, sessions) = (db()?, session_db()?);
    Ok(Json(
        run_blocking(move || {
            ha_cron::cron::discard_persistent_worktree(&cron_db, &sessions, &job_id)
        })
        .await
        .map_err(workspace_conflict)?,
    ))
}
