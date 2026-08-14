use crate::commands::CmdError;
use crate::cron;
use crate::AppState;
use anyhow::Context;
use tauri::State;

#[tauri::command]
pub async fn cron_list_jobs(
    state: State<'_, AppState>,
) -> Result<Vec<ha_core::loop_control::CronJobView>, CmdError> {
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || session_db.list_cron_job_views(&cron_db))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_get_job(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<cron::CronJob>, CmdError> {
    let cron_db = state.cron_db.clone();
    ha_core::blocking::run_blocking(move || cron_db.get_job(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_preflight(
    request: ha_cron::cron::CronPreflightRequest,
    state: State<'_, AppState>,
) -> Result<ha_cron::cron::CronPreflightReport, CmdError> {
    ha_cron::cron::evaluate_cron_preflight(
        state.cron_db.clone(),
        state.session_db.clone(),
        request,
        ha_cron::cron::CronManagedWorkspaceWritePolicy::Allowed,
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_create_job(
    job: cron::NewCronJob,
    state: State<'_, AppState>,
) -> Result<cron::CronJob, CmdError> {
    let report = ha_cron::cron::evaluate_cron_preflight(
        state.cron_db.clone(),
        state.session_db.clone(),
        ha_cron::cron::CronPreflightRequest::Create { job: job.clone() },
        ha_cron::cron::CronManagedWorkspaceWritePolicy::Allowed,
    )
    .await?;
    if !report.can_proceed {
        return Err(CmdError::msg("Cron preflight blocked"));
    }
    let cron_db = state.cron_db.clone();
    ha_core::blocking::run_blocking(move || cron_db.add_job(&job))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_update_job(
    job: cron::CronJob,
    expected_revision: u64,
    state: State<'_, AppState>,
) -> Result<cron::CronUpdateResult, CmdError> {
    let report = ha_cron::cron::evaluate_cron_preflight(
        state.cron_db.clone(),
        state.session_db.clone(),
        ha_cron::cron::CronPreflightRequest::Update {
            job: job.clone(),
            expected_revision,
        },
        ha_cron::cron::CronManagedWorkspaceWritePolicy::Allowed,
    )
    .await?;
    if !report.can_proceed {
        return Err(CmdError::msg("Cron preflight blocked"));
    }
    let cron_db = state.cron_db.clone();
    ha_core::blocking::run_blocking(move || cron_db.update_job_cas(&job, expected_revision))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_cancel_run(run_log_id: i64) -> Result<cron::CronRunCancelResult, CmdError> {
    ha_cron::cron::cancel_run(run_log_id)
        .await?
        .ok_or_else(|| CmdError::msg("Cron run not found"))
}

#[tauri::command]
/// Logically delete a task. Run logs and linked conversations stay available
/// through the history endpoints.
pub async fn cron_delete_job(id: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::delete_job_and_legacy_sessions(&cron_db, &session_db, &id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_toggle_job(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let cron_db = state.cron_db.clone();
    ha_core::blocking::run_blocking(move || cron_db.toggle_job(&id, enabled))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_run_now(
    id: String,
    expected_revision: u64,
    state: State<'_, AppState>,
) -> Result<ha_cron::cron::CronRunNowResult, CmdError> {
    ha_cron::cron::start_cron_run_now(
        state.cron_db.clone(),
        state.session_db.clone(),
        id,
        expected_revision,
        ha_cron::cron::CronManagedWorkspaceWritePolicy::Allowed,
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_jobs_referencing_account(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronAccountRef>, CmdError> {
    let cron_db = state.cron_db.clone();
    ha_core::blocking::run_blocking(move || cron_db.jobs_referencing_account(&account_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_get_run_logs(
    job_id: String,
    limit: Option<usize>,
    offset: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronRunLog>, CmdError> {
    let limit = limit.unwrap_or(50).min(200);
    let offset = offset.unwrap_or(0);
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::visible_cron_run_logs(&cron_db, &session_db, &job_id, limit, offset)
    })
    .await
    .map_err(Into::into)
}

/// Cross-job run timeline for the cron panel's "conversations" view: every cron
/// run across live and deleted jobs, newest-first, paginated; each row carries
/// the run's session id, title, unread count, and task-deleted marker.
#[tauri::command]
pub async fn cron_run_timeline(
    limit: Option<usize>,
    offset: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronTimelineRow>, CmdError> {
    let limit = limit.unwrap_or(50).min(200);
    let offset = offset.unwrap_or(0);
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::cron_run_timeline(&cron_db, &session_db, limit, offset)
    })
    .await
    .map_err(Into::into)
}

/// Total unread cron run conversations (each session contributes at most one).
#[tauri::command]
pub async fn cron_unread_total(state: State<'_, AppState>) -> Result<i64, CmdError> {
    state
        .session_db
        .run(move |db| db.cron_unread_total())
        .await
        .map_err(Into::into)
}

/// One-click clear: mark every cron session read (badge → 0) and notify the UI.
#[tauri::command]
pub async fn cron_mark_all_read(state: State<'_, AppState>) -> Result<usize, CmdError> {
    let n = state
        .session_db
        .run(move |db| db.mark_all_cron_sessions_read())
        .await?;
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit("cron:unread_changed", serde_json::json!({ "total": 0 }));
    }
    Ok(n)
}

#[tauri::command]
pub async fn cron_get_calendar_events(
    start: String,
    end: String,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CalendarEvent>, CmdError> {
    let start_dt = chrono::DateTime::parse_from_rfc3339(&start)
        .context("Invalid start date")?
        .with_timezone(&chrono::Utc);
    let end_dt = chrono::DateTime::parse_from_rfc3339(&end)
        .context("Invalid end date")?
        .with_timezone(&chrono::Utc);
    let cron_db = state.cron_db.clone();
    ha_core::blocking::run_blocking(move || cron_db.get_calendar_events(&start_dt, &end_dt))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_workspace_resources(
    job_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ha_cron::cron::CronWorkspaceResource>, CmdError> {
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::workspace_resources(&cron_db, &session_db, job_id.as_deref())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_workspace_resource_for_run(
    run_log_id: i64,
    state: State<'_, AppState>,
) -> Result<Option<ha_cron::cron::CronWorkspaceResource>, CmdError> {
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::workspace_resource_for_run(&cron_db, &session_db, run_log_id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_workspace_takeover(
    job_id: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<ha_cron::cron::CronWorkspaceActionResult, CmdError> {
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::take_over_persistent_worktree(&cron_db, &session_db, &job_id, &session_id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_workspace_return(
    job_id: String,
    session_id: String,
    resume: bool,
    state: State<'_, AppState>,
) -> Result<ha_cron::cron::CronWorkspaceActionResult, CmdError> {
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::return_persistent_worktree(
            &cron_db,
            &session_db,
            &job_id,
            &session_id,
            resume,
        )
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_workspace_discard_run(
    run_log_id: i64,
    session_id: String,
    confirm: bool,
    state: State<'_, AppState>,
) -> Result<ha_cron::cron::CronWorkspaceActionResult, CmdError> {
    if !confirm {
        return Err(CmdError::msg("discard requires explicit confirmation"));
    }
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::discard_run_worktree(&cron_db, &session_db, run_log_id, &session_id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn cron_workspace_discard_task(
    job_id: String,
    confirm: bool,
    state: State<'_, AppState>,
) -> Result<ha_cron::cron::CronWorkspaceActionResult, CmdError> {
    if !confirm {
        return Err(CmdError::msg("discard requires explicit confirmation"));
    }
    let cron_db = state.cron_db.clone();
    let session_db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        ha_cron::cron::discard_persistent_worktree(&cron_db, &session_db, &job_id)
    })
    .await
    .map_err(Into::into)
}
