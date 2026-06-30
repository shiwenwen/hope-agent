use crate::commands::CmdError;
use ha_core::workflow::{WorkflowRun, WorkflowRunSnapshot};

#[tauri::command]
pub async fn list_workflow_runs(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<WorkflowRun>, CmdError> {
    app_state
        .session_db
        .list_workflow_runs_for_session(&session_id, 100)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<WorkflowRunSnapshot>, CmdError> {
    app_state
        .session_db
        .workflow_run_snapshot(&run_id, 200)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pause_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    app_state
        .session_db
        .pause_workflow_run(&run_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    app_state
        .session_db
        .resume_workflow_run(&run_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    app_state
        .session_db
        .cancel_workflow_run(&run_id)
        .map_err(Into::into)
}
