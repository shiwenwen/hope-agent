use crate::commands::CmdError;
use crate::cron;
use crate::AppState;
use anyhow::Context;
use tauri::State;

#[tauri::command]
pub async fn cron_list_jobs(state: State<'_, AppState>) -> Result<Vec<cron::CronJob>, CmdError> {
    state.cron_db.list_jobs().map_err(Into::into)
}

#[tauri::command]
pub async fn cron_get_job(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<cron::CronJob>, CmdError> {
    state.cron_db.get_job(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn cron_create_job(
    job: cron::NewCronJob,
    state: State<'_, AppState>,
) -> Result<cron::CronJob, CmdError> {
    state.cron_db.add_job(&job).map_err(Into::into)
}

#[tauri::command]
pub async fn cron_update_job(
    job: cron::CronJob,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state.cron_db.update_job(&job).map_err(Into::into)
}

#[tauri::command]
pub async fn cron_delete_job(id: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    cron::delete_job_and_sessions(&state.cron_db, &state.session_db, &id).map_err(Into::into)
}

#[tauri::command]
pub async fn cron_toggle_job(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state.cron_db.toggle_job(&id, enabled).map_err(Into::into)
}

#[tauri::command]
pub async fn cron_run_now(id: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    // Cron only runs on the Primary instance — `execute_job_public` no-ops on a
    // Secondary (C10). The desktop is normally Primary, but guard anyway so a
    // Secondary desktop reports the failure instead of silently swallowing the run.
    if !ha_core::runtime_lock::is_primary() {
        return Err(CmdError::msg(
            "run-now is unavailable on this instance: scheduled jobs only run on the primary",
        ));
    }
    let job = state
        .cron_db
        .get_job(&id)?
        .ok_or_else(|| CmdError::msg("Job not found"))?;

    let db = state.cron_db.clone();
    let sdb = state.session_db.clone();
    tokio::spawn(async move {
        cron::execute_job_public(&db, &sdb, &job).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn cron_jobs_referencing_account(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronAccountRef>, CmdError> {
    state
        .cron_db
        .jobs_referencing_account(&account_id)
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
    state
        .cron_db
        .get_run_logs(&job_id, limit, offset)
        .map_err(Into::into)
}

/// Cross-job run timeline for the cron panel's "conversations" view: every cron
/// run across all jobs, newest-first, paginated; each row carries the run's
/// session id + title + unread count for the read-only conversation viewer.
#[tauri::command]
pub async fn cron_run_timeline(
    limit: Option<usize>,
    offset: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronTimelineRow>, CmdError> {
    let limit = limit.unwrap_or(50).min(200);
    let offset = offset.unwrap_or(0);
    cron::cron_run_timeline(&state.cron_db, &state.session_db, limit, offset).map_err(Into::into)
}

/// Total unread assistant messages across all cron sessions (sidebar badge).
#[tauri::command]
pub async fn cron_unread_total(state: State<'_, AppState>) -> Result<i64, CmdError> {
    state.session_db.cron_unread_total().map_err(Into::into)
}

/// One-click clear: mark every cron session read (badge → 0) and notify the UI.
#[tauri::command]
pub async fn cron_mark_all_read(state: State<'_, AppState>) -> Result<usize, CmdError> {
    let n = state.session_db.mark_all_cron_sessions_read()?;
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
    state
        .cron_db
        .get_calendar_events(&start_dt, &end_dt)
        .map_err(Into::into)
}
