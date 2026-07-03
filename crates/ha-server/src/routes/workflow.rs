use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::helpers::session_db;

fn workflow_owner(label: &str) -> String {
    format!("server:{label}:pid:{}", std::process::id())
}

pub async fn list_workflow_runs(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ha_core::workflow::WorkflowRun>>, AppError> {
    Ok(Json(
        session_db()?.list_workflow_runs_for_session(&session_id, 100)?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRunBody {
    pub kind: Option<String>,
    pub execution_mode: Option<String>,
    pub script_source: String,
    pub budget: Option<Value>,
    pub parent_run_id: Option<String>,
    pub origin: Option<String>,
    pub goal_id: Option<String>,
    pub worktree_id: Option<String>,
    pub run_immediately: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWorkflowScriptBody {
    pub script_source: String,
    pub execution_mode: Option<String>,
}

pub async fn preview_workflow_script(
    Path(session_id): Path<String>,
    Json(body): Json<PreviewWorkflowScriptBody>,
) -> Result<Json<ha_core::workflow::WorkflowScriptPreview>, AppError> {
    let db = session_db()?;
    Ok(Json(
        ha_core::workflow::preview_workflow_script_for_session(
            &db,
            &session_id,
            &body.script_source,
            body.execution_mode.as_deref(),
        ),
    ))
}

pub async fn create_workflow_run(
    Path(session_id): Path<String>,
    Json(body): Json<CreateWorkflowRunBody>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    let mode = body.execution_mode.unwrap_or_else(|| "guarded".to_string());
    let parsed_mode = ha_core::execution_mode::ExecutionMode::from_str(&mode)
        .ok_or_else(|| AppError::bad_request("Invalid execution mode"))?;
    let run_now = body.run_immediately.unwrap_or(false);
    if run_now {
        ha_core::workflow::ensure_workflow_launcher_primary()
            .map_err(|e| AppError::bad_request(e.to_string()))?;
    }
    let db = session_db()?;
    let script_source = body.script_source;
    ha_core::workflow::ensure_workflow_script_can_create(
        &db,
        &session_id,
        &script_source,
        Some(parsed_mode.as_str()),
    )
    .map_err(|e| AppError::bad_request(e.to_string()))?;
    let run = db
        .create_workflow_run(ha_core::workflow::CreateWorkflowRunInput {
            session_id,
            kind: body.kind.unwrap_or_else(|| "general.workflow".to_string()),
            execution_mode: parsed_mode.as_str().to_string(),
            script_source,
            budget: body.budget.unwrap_or_else(|| json!({})),
            parent_run_id: body.parent_run_id,
            origin: body.origin,
            goal_id: body.goal_id,
            worktree_id: body.worktree_id,
        })
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    if run_now {
        ha_core::workflow::spawn_workflow_run_if_primary(
            db.clone(),
            run.id.clone(),
            workflow_owner("create"),
        );
    }
    Ok(Json(run))
}

pub async fn get_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<Option<ha_core::workflow::WorkflowRunSnapshot>>, AppError> {
    Ok(Json(session_db()?.workflow_run_snapshot(&run_id, 200)?))
}

pub async fn run_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    ha_core::workflow::ensure_workflow_launcher_primary()
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let db = session_db()?;
    let run = db
        .get_workflow_run(&run_id)?
        .ok_or_else(|| AppError::not_found("workflow run not found"))?;
    ha_core::workflow::spawn_workflow_run_if_primary(
        db.clone(),
        run.id.clone(),
        workflow_owner("run"),
    );
    Ok(Json(run))
}

pub async fn pause_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    session_db()?
        .pause_workflow_run(&run_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn resume_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    ha_core::workflow::ensure_workflow_launcher_primary()
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let db = session_db()?;
    let run = db
        .resume_workflow_run(&run_id)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    ha_core::workflow::spawn_workflow_run_if_primary(
        db.clone(),
        run.id.clone(),
        workflow_owner("resume"),
    );
    Ok(Json(run))
}

pub async fn approve_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    ha_core::workflow::ensure_workflow_launcher_primary()
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let db = session_db()?;
    let run = db
        .approve_workflow_run(&run_id)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    ha_core::workflow::spawn_workflow_run_if_primary(
        db.clone(),
        run.id.clone(),
        workflow_owner("approve"),
    );
    Ok(Json(run))
}

pub async fn cancel_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    let run = ha_core::workflow::cancel_workflow_run_with_children(session_db()?.clone(), &run_id)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(run))
}
