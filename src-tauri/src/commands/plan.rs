use crate::plan::{
    self, PlanModeState, PlanQuestionAnswer, PlanStep, PlanStepStatus, PlanVersionInfo,
};
use oc_core::app_info;

#[tauri::command]
pub async fn get_plan_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    let state = plan::get_plan_state(&session_id).await;
    if state != PlanModeState::Off {
        return Ok(state.as_str().to_string());
    }
    // Fallback: check DB (in-memory store may be empty after restart)
    if let Ok(Some(meta)) = app_state.session_db.get_session(&session_id) {
        if meta.plan_mode != "off" {
            // Restore in-memory state from DB + plan file
            plan::restore_from_db(&session_id, &meta.plan_mode).await;
            return Ok(meta.plan_mode);
        }
    }
    Ok("off".to_string())
}

#[tauri::command]
pub async fn set_plan_mode(
    session_id: String,
    state: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let plan_state = PlanModeState::from_str(&state);
    let is_executing = plan_state == PlanModeState::Executing;

    // Clean up checkpoint on successful completion or exit
    if plan_state == PlanModeState::Completed || plan_state == PlanModeState::Off {
        if let Some(ref_name) = plan::get_checkpoint_ref(&session_id).await {
            plan::cleanup_checkpoint(&ref_name);
        }
    }

    // Cancel active plan sub-agent when exiting plan mode or transitioning away from Planning
    if plan_state == PlanModeState::Off {
        if let Some(run_id) = plan::get_active_plan_run_id(&session_id).await {
            if let Some(cancels) = crate::get_subagent_cancels() {
                cancels.cancel(&run_id);
                app_info!(
                    "plan",
                    "set_plan_mode",
                    "Cancelled plan sub-agent: {}",
                    run_id
                );
            }
        }
    }

    plan::set_plan_state(&session_id, plan_state).await;

    // Create git checkpoint AFTER PlanMeta entry exists in the store
    if is_executing {
        plan::create_checkpoint_for_session(&session_id).await;
    }
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
pub async fn get_plan_steps(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<PlanStep>, String> {
    // Try in-memory first
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if !meta.steps.is_empty() {
            return Ok(meta.steps);
        }
    }
    // Fallback: restore from DB + plan file (after restart)
    if let Ok(Some(session_meta)) = app_state.session_db.get_session(&session_id) {
        if session_meta.plan_mode != "off" {
            plan::restore_from_db(&session_id, &session_meta.plan_mode).await;
            if let Some(meta) = plan::get_plan_meta(&session_id).await {
                return Ok(meta.steps);
            }
        }
    }
    Ok(Vec::new())
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
        let _ = app_handle.emit(
            "plan_step_updated",
            serde_json::json!({
                "sessionId": session_id,
                "stepIndex": step_index,
                "status": status,
            }),
        );
    }

    // Check if all steps are terminal → auto-transition to Completed
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if meta.all_terminal() && meta.state == PlanModeState::Executing {
            // Clean up git checkpoint on successful completion
            if let Some(ref_name) = plan::get_checkpoint_ref(&session_id).await {
                plan::cleanup_checkpoint(&ref_name);
            }
            plan::set_plan_state(&session_id, PlanModeState::Completed).await;
            // Persist completed state to DB for crash safety
            if let Some(session_db) = crate::get_session_db() {
                let _ = session_db.update_session_plan_mode(&session_id, "completed");
            }
            if let Some(app_handle) = crate::get_app_handle() {
                use tauri::Emitter;
                let _ = app_handle.emit(
                    "plan_mode_changed",
                    serde_json::json!({
                        "sessionId": session_id,
                        "state": "completed",
                        "reason": "all_steps_completed",
                    }),
                );
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn respond_plan_question(
    request_id: String,
    answers: Vec<PlanQuestionAnswer>,
) -> Result<(), String> {
    plan::submit_plan_question_response(&request_id, answers)
        .await
        .map_err(|e| e.to_string())
}

/// Canonical name for the interactive Q&A response command.
/// Forwards to [`respond_plan_question`] and is available in any conversation
/// (not only Plan Mode). Kept as a separate command so the Transport layer can
/// pick the preferred name while historical code paths keep working.
#[tauri::command]
pub async fn respond_ask_user(
    request_id: String,
    answers: Vec<PlanQuestionAnswer>,
) -> Result<(), String> {
    plan::submit_plan_question_response(&request_id, answers)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_plan_versions(session_id: String) -> Result<Vec<PlanVersionInfo>, String> {
    plan::list_plan_versions(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_plan_version_content(file_path: String) -> Result<String, String> {
    plan::load_plan_version(&file_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_plan_version(session_id: String, file_path: String) -> Result<(), String> {
    let content = plan::load_plan_version(&file_path).map_err(|e| e.to_string())?;
    plan::save_plan_file(&session_id, &content).map_err(|e| e.to_string())?;
    let steps = plan::parse_plan_steps(&content);
    plan::update_plan_steps(&session_id, steps).await;
    Ok(())
}

#[tauri::command]
pub async fn plan_rollback(session_id: String) -> Result<String, String> {
    let checkpoint = plan::get_checkpoint_ref(&session_id)
        .await
        .ok_or_else(|| "No git checkpoint found for this plan execution".to_string())?;

    let msg = plan::rollback_to_checkpoint(&checkpoint).map_err(|e| e.to_string())?;

    // Clear checkpoint ref after rollback
    let mut map = plan::store().write().await;
    if let Some(meta) = map.get_mut(&session_id) {
        meta.checkpoint_ref = None;
    }

    Ok(msg)
}

#[tauri::command]
pub async fn get_plan_checkpoint(session_id: String) -> Result<Option<String>, String> {
    Ok(plan::get_checkpoint_ref(&session_id).await)
}

#[tauri::command]
pub async fn get_plan_file_path(session_id: String) -> Result<Option<String>, String> {
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if !meta.file_path.is_empty() {
            return Ok(Some(meta.file_path));
        }
    }
    Ok(None)
}

#[tauri::command]
pub async fn cancel_plan_subagent(session_id: String) -> Result<(), String> {
    if let Some(run_id) = plan::get_active_plan_run_id(&session_id).await {
        if let Some(cancels) = crate::get_subagent_cancels() {
            cancels.cancel(&run_id);
            app_info!(
                "plan",
                "cancel_plan_subagent",
                "Cancelled plan sub-agent: {}",
                run_id
            );
            Ok(())
        } else {
            Err("Cancel registry not initialized".to_string())
        }
    } else {
        Ok(()) // No active plan sub-agent — nothing to cancel
    }
}
