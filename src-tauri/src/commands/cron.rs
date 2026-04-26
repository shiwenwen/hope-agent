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
    state.cron_db.delete_job(&id).map_err(Into::into)
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
pub async fn cron_get_run_logs(
    job_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronRunLog>, CmdError> {
    let limit = limit.unwrap_or(50);
    state
        .cron_db
        .get_run_logs(&job_id, limit)
        .map_err(Into::into)
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
