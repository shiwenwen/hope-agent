use crate::commands::CmdError;
use ha_core::workflow::{
    CreateWorkflowRunInput, WorkflowRun, WorkflowRunSnapshot, WorkflowScriptPreview,
};
use serde_json::{json, Value};

fn workflow_owner(label: &str) -> String {
    format!("desktop:{label}:pid:{}", std::process::id())
}

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
pub async fn preview_workflow_script(
    session_id: String,
    script_source: String,
    execution_mode: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowScriptPreview, CmdError> {
    Ok(ha_core::workflow::preview_workflow_script_for_session(
        &app_state.session_db,
        &session_id,
        &script_source,
        execution_mode.as_deref(),
    ))
}

#[tauri::command]
pub async fn create_workflow_run(
    session_id: String,
    script_source: String,
    kind: Option<String>,
    execution_mode: Option<String>,
    budget: Option<Value>,
    parent_run_id: Option<String>,
    origin: Option<String>,
    goal_id: Option<String>,
    worktree_id: Option<String>,
    run_immediately: Option<bool>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    let mode = execution_mode.unwrap_or_else(|| "guarded".to_string());
    let parsed_mode = ha_core::execution_mode::ExecutionMode::from_str(&mode)
        .ok_or_else(|| CmdError::msg("Invalid execution mode"))?;
    ha_core::workflow::ensure_workflow_script_can_create(
        &app_state.session_db,
        &session_id,
        &script_source,
        Some(parsed_mode.as_str()),
    )?;
    let run = app_state
        .session_db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id,
            kind: kind.unwrap_or_else(|| "coding.workflow".to_string()),
            execution_mode: parsed_mode.as_str().to_string(),
            script_source,
            budget: budget.unwrap_or_else(|| json!({})),
            parent_run_id,
            origin,
            goal_id,
            worktree_id,
        })?;
    if run_immediately.unwrap_or(false) {
        ha_core::workflow::spawn_workflow_run_if_primary(
            app_state.session_db.clone(),
            run.id.clone(),
            workflow_owner("create"),
        );
    }
    Ok(run)
}

#[tauri::command]
pub async fn run_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    let run = app_state
        .session_db
        .get_workflow_run(&run_id)?
        .ok_or_else(|| CmdError::msg(format!("Workflow run not found: {run_id}")))?;
    ha_core::workflow::spawn_workflow_run_if_primary(
        app_state.session_db.clone(),
        run.id.clone(),
        workflow_owner("run"),
    );
    Ok(run)
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
    let run = app_state
        .session_db
        .resume_workflow_run(&run_id)
        .map_err(CmdError::from)?;
    ha_core::workflow::spawn_workflow_run_if_primary(
        app_state.session_db.clone(),
        run.id.clone(),
        workflow_owner("resume"),
    );
    Ok(run)
}

#[tauri::command]
pub async fn approve_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    let run = app_state
        .session_db
        .approve_workflow_run(&run_id)
        .map_err(CmdError::from)?;
    ha_core::workflow::spawn_workflow_run_if_primary(
        app_state.session_db.clone(),
        run.id.clone(),
        workflow_owner("approve"),
    );
    Ok(run)
}

#[tauri::command]
pub async fn cancel_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    ha_core::workflow::cancel_workflow_run_with_children(app_state.session_db.clone(), &run_id)
        .await
        .map_err(Into::into)
}
