use crate::plan::{self, PlanModeState, PlanStep, PlanStepStatus};

#[tauri::command]
pub async fn get_plan_mode(session_id: String) -> Result<String, String> {
    let state = plan::get_plan_state(&session_id).await;
    Ok(state.as_str().to_string())
}

#[tauri::command]
pub async fn set_plan_mode(
    session_id: String,
    state: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let plan_state = PlanModeState::from_str(&state);
    plan::set_plan_state(&session_id, plan_state).await;
    // Persist to DB
    let db = &app_state.session_db;
    db.update_session_plan_mode(&session_id, &state)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_plan_content(session_id: String) -> Result<Option<String>, String> {
    plan::load_plan_file(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_plan_content(session_id: String, content: String) -> Result<(), String> {
    // Save file
    plan::save_plan_file(&session_id, &content).map_err(|e| e.to_string())?;
    // Parse steps and update in-memory state
    let steps = plan::parse_plan_steps(&content);
    plan::update_plan_steps(&session_id, steps).await;
    Ok(())
}

#[tauri::command]
pub async fn get_plan_steps(session_id: String) -> Result<Vec<PlanStep>, String> {
    match plan::get_plan_meta(&session_id).await {
        Some(meta) => Ok(meta.steps),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn update_plan_step_status(
    session_id: String,
    step_index: usize,
    status: String,
) -> Result<(), String> {
    let step_status = PlanStepStatus::from_str(&status);
    plan::update_step_status(&session_id, step_index, step_status, None).await;

    // Emit Tauri global event for frontend real-time update
    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app_handle.emit("plan_step_updated", serde_json::json!({
            "sessionId": session_id,
            "stepIndex": step_index,
            "status": status,
        }));
    }

    // Check if all steps are terminal → auto-transition to Off
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if meta.all_terminal() && meta.state == PlanModeState::Executing {
            plan::set_plan_state(&session_id, PlanModeState::Off).await;
            if let Some(app_handle) = crate::get_app_handle() {
                use tauri::Emitter;
                let _ = app_handle.emit("plan_mode_changed", serde_json::json!({
                    "sessionId": session_id,
                    "state": "off",
                    "reason": "all_steps_completed",
                }));
            }
        }
    }

    Ok(())
}
