use axum::extract::Path;
use axum::Json;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn list_workflow_runs(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ha_core::workflow::WorkflowRun>>, AppError> {
    Ok(Json(
        session_db()?.list_workflow_runs_for_session(&session_id, 100)?,
    ))
}

pub async fn get_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<Option<ha_core::workflow::WorkflowRunSnapshot>>, AppError> {
    Ok(Json(session_db()?.workflow_run_snapshot(&run_id, 200)?))
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
    session_db()?
        .resume_workflow_run(&run_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn cancel_workflow_run(
    Path(run_id): Path<String>,
) -> Result<Json<ha_core::workflow::WorkflowRun>, AppError> {
    session_db()?
        .cancel_workflow_run(&run_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
