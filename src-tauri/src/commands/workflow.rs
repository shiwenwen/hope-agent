use crate::commands::CmdError;
use ha_core::workflow::{
    CreateWorkflowRunFromTemplateInput, CreateWorkflowRunInput, ListSavedWorkflowTemplatesInput,
    SaveWorkflowTemplateInput, SavedWorkflowTemplate, WorkflowRun, WorkflowRunSnapshot,
    WorkflowScriptPreview, WorkflowWatchdogFinding,
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_workflow_runs_for_session(&session_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_workflow_watchdog_findings(
    session_id: String,
    stale_secs: Option<i64>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<WorkflowWatchdogFinding>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_workflow_watchdog_findings(&session_id, stale_secs.unwrap_or(300)))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_saved_workflow_templates(
    project_id: Option<String>,
    include_disabled: Option<bool>,
    limit: Option<usize>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<SavedWorkflowTemplate>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| {
        db.list_saved_workflow_templates(ListSavedWorkflowTemplatesInput {
            project_id,
            include_disabled: include_disabled.unwrap_or(false),
            limit,
        })
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn save_workflow_template_from_run(
    input: SaveWorkflowTemplateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<SavedWorkflowTemplate, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.save_workflow_template_from_run(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_workflow_run_from_template(
    input: CreateWorkflowRunFromTemplateInput,
    run_immediately: Option<bool>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    let db = app_state.session_db.clone();
    let template_id = input.template_id.clone();
    let template = db
        .run(move |db| db.get_saved_workflow_template(&template_id))
        .await?
        .ok_or_else(|| {
            CmdError::msg(format!(
                "Saved workflow template not found: {}",
                input.template_id
            ))
        })?;
    let parsed_mode = ha_core::execution_mode::ExecutionMode::from_str(&template.execution_mode)
        .ok_or_else(|| CmdError::msg("Invalid execution mode"))?;
    let run_now = run_immediately.unwrap_or(false);
    if run_now {
        ha_core::workflow::ensure_workflow_launcher_primary().map_err(CmdError::from)?;
    }
    let db = app_state.session_db.clone();
    let session_id = input.session_id.clone();
    let script_source = template.script_source.clone();
    let mode_str = parsed_mode.as_str().to_string();
    let run = db
        .run(move |db| {
            ha_core::workflow::ensure_workflow_script_can_create(
                db,
                &session_id,
                &script_source,
                Some(&mode_str),
            )?;
            db.create_workflow_run_from_template(input)
        })
        .await
        .map_err(CmdError::from)?;
    if run_now {
        ha_core::workflow::spawn_workflow_run_if_primary(
            app_state.session_db.clone(),
            run.id.clone(),
            workflow_owner("template"),
        );
    }
    Ok(run)
}

#[tauri::command]
pub async fn get_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<WorkflowRunSnapshot>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.workflow_run_snapshot(&run_id, 200))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_workflow_script(
    session_id: String,
    script_source: String,
    execution_mode: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowScriptPreview, CmdError> {
    let db = app_state.session_db.clone();
    Ok(db
        .run(move |db| {
            ha_core::workflow::preview_workflow_script_for_session(
                db,
                &session_id,
                &script_source,
                execution_mode.as_deref(),
            )
        })
        .await)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_workflow_run(
    session_id: String,
    script_source: String,
    kind: Option<String>,
    execution_mode: Option<String>,
    budget: Option<Value>,
    api_version: Option<i64>,
    meta: Option<Value>,
    args: Option<Value>,
    resume_from_run_id: Option<String>,
    parent_run_id: Option<String>,
    origin: Option<String>,
    goal_id: Option<String>,
    goal_criterion_id: Option<String>,
    worktree_id: Option<String>,
    run_immediately: Option<bool>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    let mode = execution_mode.unwrap_or_else(|| "guarded".to_string());
    let parsed_mode = ha_core::execution_mode::ExecutionMode::from_str(&mode)
        .ok_or_else(|| CmdError::msg("Invalid execution mode"))?;
    let run_now = run_immediately.unwrap_or(false);
    if run_now {
        ha_core::workflow::ensure_workflow_launcher_primary().map_err(CmdError::from)?;
    }
    let mode_str = parsed_mode.as_str().to_string();
    let db = app_state.session_db.clone();
    let run = db
        .run(move |db| {
            ha_core::workflow::ensure_workflow_script_can_create(
                db,
                &session_id,
                &script_source,
                Some(&mode_str),
            )?;
            db.create_workflow_run_with_control(
                CreateWorkflowRunInput {
                    session_id,
                    kind: kind.unwrap_or_else(|| "general.workflow".to_string()),
                    execution_mode: mode_str,
                    script_source,
                    budget: budget.unwrap_or_else(|| json!({})),
                    parent_run_id,
                    origin,
                    goal_id,
                    goal_criterion_id,
                    worktree_id,
                },
                ha_core::workflow::WorkflowRunControlInput {
                    api_version: api_version.unwrap_or(4),
                    meta: meta.unwrap_or_else(|| json!({})),
                    args: args.unwrap_or_else(|| json!({})),
                    resume_from_run_id,
                },
            )
        })
        .await
        .map_err(CmdError::from)?;
    if run_now {
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
    ha_core::workflow::ensure_workflow_launcher_primary().map_err(CmdError::from)?;
    let db = app_state.session_db.clone();
    let lookup_id = run_id.clone();
    let run = db
        .run(move |db| db.get_workflow_run(&lookup_id))
        .await?
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.pause_workflow_run(&run_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_workflow_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<WorkflowRun, CmdError> {
    ha_core::workflow::ensure_workflow_launcher_primary().map_err(CmdError::from)?;
    let db = app_state.session_db.clone();
    let run = db
        .run(move |db| db.resume_workflow_run(&run_id))
        .await
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
    ha_core::workflow::ensure_workflow_launcher_primary().map_err(CmdError::from)?;
    let db = app_state.session_db.clone();
    let run = db
        .run(move |db| db.approve_workflow_run(&run_id))
        .await
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
