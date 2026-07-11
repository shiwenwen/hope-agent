use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::helpers::session_db;

/// `GET /api/sessions/{session_id}/execution-mode`
pub async fn get_execution_mode(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    let db = session_db()?;
    let sid = session_id.clone();
    let mode = db
        .run(move |db| db.get_session_execution_mode(&sid))
        .await?
        .ok_or_else(|| AppError::not_found(format!("Session not found: {session_id}")))?;
    Ok(Json(json!({ "mode": mode.as_str() })))
}

#[derive(Debug, Deserialize)]
pub struct SetExecutionModeBody {
    pub mode: String,
}

/// `POST /api/sessions/{session_id}/execution-mode`
pub async fn set_execution_mode(
    Path(session_id): Path<String>,
    Json(body): Json<SetExecutionModeBody>,
) -> Result<Json<Value>, AppError> {
    let mode = ha_core::execution_mode::ExecutionMode::from_str(&body.mode)
        .ok_or_else(|| AppError::bad_request("Invalid execution mode"))?;
    let db = session_db()?;
    db.run(move |db| db.update_session_execution_mode(&session_id, mode))
        .await
        .map_err(map_session_mode_error)?;
    Ok(Json(json!({ "mode": mode.as_str() })))
}

/// `GET /api/sessions/{session_id}/workflow-mode`
pub async fn get_workflow_mode(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    let db = session_db()?;
    let sid = session_id.clone();
    let mode = db
        .run(move |db| db.get_session_workflow_mode(&sid))
        .await?
        .ok_or_else(|| AppError::not_found(format!("Session not found: {session_id}")))?;
    Ok(Json(json!({ "mode": mode.as_str() })))
}

#[derive(Debug, Deserialize)]
pub struct SetWorkflowModeBody {
    pub mode: String,
}

/// `POST /api/sessions/{session_id}/workflow-mode`
pub async fn set_workflow_mode(
    Path(session_id): Path<String>,
    Json(body): Json<SetWorkflowModeBody>,
) -> Result<Json<Value>, AppError> {
    let mode = ha_core::workflow_mode::WorkflowMode::from_str(&body.mode)
        .ok_or_else(|| AppError::bad_request("Invalid workflow mode"))?;
    let db = session_db()?;
    db.run(move |db| db.update_session_workflow_mode(&session_id, mode))
        .await
        .map_err(map_session_mode_error)?;
    Ok(Json(json!({ "mode": mode.as_str() })))
}

fn map_session_mode_error(err: anyhow::Error) -> AppError {
    let message = err.to_string();
    if message.contains("Session not found") {
        AppError::not_found(message)
    } else if message.contains("incognito session") {
        AppError::bad_request(message)
    } else {
        AppError::internal(message)
    }
}
