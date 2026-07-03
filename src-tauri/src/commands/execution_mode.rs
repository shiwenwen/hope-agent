use crate::commands::CmdError;
use serde_json::{json, Value};

#[tauri::command]
pub async fn get_execution_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Value, CmdError> {
    let mode = app_state
        .session_db
        .get_session_execution_mode(&session_id)?
        .ok_or_else(|| CmdError::msg(format!("Session not found: {session_id}")))?;
    Ok(json!({ "mode": mode.as_str() }))
}

#[tauri::command]
pub async fn set_execution_mode(
    session_id: String,
    mode: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Value, CmdError> {
    let parsed = ha_core::execution_mode::ExecutionMode::from_str(&mode)
        .ok_or_else(|| CmdError::msg("Invalid execution mode"))?;
    app_state
        .session_db
        .update_session_execution_mode(&session_id, parsed)?;
    Ok(json!({ "mode": parsed.as_str() }))
}

#[tauri::command]
pub async fn get_workflow_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Value, CmdError> {
    let mode = app_state
        .session_db
        .get_session_workflow_mode(&session_id)?
        .ok_or_else(|| CmdError::msg(format!("Session not found: {session_id}")))?;
    Ok(json!({ "mode": mode.as_str() }))
}

#[tauri::command]
pub async fn set_workflow_mode(
    session_id: String,
    mode: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Value, CmdError> {
    let parsed = ha_core::workflow_mode::WorkflowMode::from_str(&mode)
        .ok_or_else(|| CmdError::msg("Invalid workflow mode"))?;
    app_state
        .session_db
        .update_session_workflow_mode(&session_id, parsed)?;
    Ok(json!({ "mode": parsed.as_str() }))
}
