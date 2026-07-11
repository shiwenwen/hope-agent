use crate::commands::CmdError;
use serde_json::{json, Value};

#[tauri::command]
pub async fn get_execution_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Value, CmdError> {
    let db = app_state.session_db.clone();
    let sid = session_id.clone();
    let mode = db
        .run(move |db| db.get_session_execution_mode(&sid))
        .await?
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.update_session_execution_mode(&session_id, parsed))
        .await?;
    Ok(json!({ "mode": parsed.as_str() }))
}

#[tauri::command]
pub async fn get_workflow_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Value, CmdError> {
    let db = app_state.session_db.clone();
    let sid = session_id.clone();
    let mode = db
        .run(move |db| db.get_session_workflow_mode(&sid))
        .await?
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.update_session_workflow_mode(&session_id, parsed))
        .await?;
    Ok(json!({ "mode": parsed.as_str() }))
}
