use tauri::State;
use crate::AppState;
use crate::cron;

#[tauri::command]
pub async fn cron_list_jobs(
    state: State<'_, AppState>,
) -> Result<Vec<cron::CronJob>, String> {
    state.cron_db.list_jobs().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_get_job(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<cron::CronJob>, String> {
    state.cron_db.get_job(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_create_job(
    job: cron::NewCronJob,
    state: State<'_, AppState>,
) -> Result<cron::CronJob, String> {
    state.cron_db.add_job(&job).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_update_job(
    job: cron::CronJob,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.cron_db.update_job(&job).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_delete_job(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.cron_db.delete_job(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_toggle_job(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.cron_db.toggle_job(&id, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_run_now(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let job = state.cron_db.get_job(&id).map_err(|e| e.to_string())?
        .ok_or_else(|| "Job not found".to_string())?;

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
) -> Result<Vec<cron::CronRunLog>, String> {
    let limit = limit.unwrap_or(50);
    state.cron_db.get_run_logs(&job_id, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cron_get_calendar_events(
    start: String,
    end: String,
    state: State<'_, AppState>,
) -> Result<Vec<cron::CalendarEvent>, String> {
    let start_dt = chrono::DateTime::parse_from_rfc3339(&start)
        .map_err(|e| format!("Invalid start date: {}", e))?
        .with_timezone(&chrono::Utc);
    let end_dt = chrono::DateTime::parse_from_rfc3339(&end)
        .map_err(|e| format!("Invalid end date: {}", e))?
        .with_timezone(&chrono::Utc);
    state.cron_db.get_calendar_events(&start_dt, &end_dt).map_err(|e| e.to_string())
}
