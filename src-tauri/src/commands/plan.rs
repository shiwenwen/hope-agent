use crate::commands::CmdError;
use crate::plan::{self, PlanModeState, PlanStep, PlanStepStatus, PlanVersionInfo};
use ha_core::app_info;
use ha_core::ask_user::AskUserQuestionAnswer;

#[tauri::command]
pub async fn get_plan_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<String, CmdError> {
    if let Ok(Some(meta)) = app_state.session_db.get_session(&session_id) {
        if meta.plan_mode == "off" {
            plan::set_plan_state(&session_id, PlanModeState::Off).await;
            return Ok("off".to_string());
        }
        if meta.plan_mode != "off" {
            // Restore in-memory state from DB + plan file
            plan::restore_from_db(&session_id, &meta.plan_mode).await;
            return Ok(meta.plan_mode);
        }
    }
    let state = plan::get_plan_state(&session_id).await;
    if state != PlanModeState::Off {
        return Ok(state.as_str().to_string());
    }
    Ok("off".to_string())
}

#[tauri::command]
pub async fn set_plan_mode(
    session_id: String,
    state: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
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
    db.update_session_plan_mode(&session_id, &state)?;
    Ok(())
}

#[tauri::command]
pub async fn get_plan_content(session_id: String) -> Result<Option<String>, CmdError> {
    plan::load_plan_file(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn save_plan_content(session_id: String, content: String) -> Result<(), CmdError> {
    // Save file
    plan::save_plan_file(&session_id, &content)?;
    // Parse steps and update in-memory state
    let steps = plan::parse_plan_steps(&content);
    plan::update_plan_steps(&session_id, steps).await;
    Ok(())
}

#[tauri::command]
pub async fn get_plan_steps(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<PlanStep>, CmdError> {
    if let Ok(Some(session_meta)) = app_state.session_db.get_session(&session_id) {
        if session_meta.plan_mode == "off" {
            plan::set_plan_state(&session_id, PlanModeState::Off).await;
            return Ok(Vec::new());
        }
        if session_meta.plan_mode != "off" {
            plan::restore_from_db(&session_id, &session_meta.plan_mode).await;
            if let Some(meta) = plan::get_plan_meta(&session_id).await {
                return Ok(meta.steps);
            }
        }
    }
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if !meta.steps.is_empty() {
            return Ok(meta.steps);
        }
    }
    Ok(Vec::new())
}

#[tauri::command]
pub async fn update_plan_step_status(
    session_id: String,
    step_index: usize,
    status: String,
) -> Result<(), CmdError> {
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
pub async fn get_pending_ask_user_group(
    session_id: String,
) -> Result<Option<ha_core::ask_user::AskUserQuestionGroup>, CmdError> {
    ha_core::ask_user::find_live_pending_group_for_session(&session_id)
        .await
        .map_err(Into::into)
}

/// Submit the user's answers to an `ask_user_question` tool call.
#[tauri::command]
pub async fn respond_ask_user_question(
    request_id: String,
    answers: Vec<AskUserQuestionAnswer>,
) -> Result<(), CmdError> {
    ha_core::ask_user::submit_ask_user_question_response(&request_id, answers)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_plan_versions(session_id: String) -> Result<Vec<PlanVersionInfo>, CmdError> {
    plan::list_plan_versions(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn load_plan_version_content(file_path: String) -> Result<String, CmdError> {
    plan::load_plan_version(&file_path).map_err(Into::into)
}

#[tauri::command]
pub async fn restore_plan_version(session_id: String, file_path: String) -> Result<(), CmdError> {
    let content = plan::load_plan_version(&file_path)?;
    plan::save_plan_file(&session_id, &content)?;
    let steps = plan::parse_plan_steps(&content);
    plan::update_plan_steps(&session_id, steps).await;
    Ok(())
}

#[tauri::command]
pub async fn plan_rollback(session_id: String) -> Result<String, CmdError> {
    let checkpoint = plan::get_checkpoint_ref(&session_id)
        .await
        .ok_or_else(|| CmdError::msg("No git checkpoint found for this plan execution"))?;

    let msg = plan::rollback_to_checkpoint(&checkpoint)?;

    // Clear checkpoint ref after rollback
    let mut map = plan::store().write().await;
    if let Some(meta) = map.get_mut(&session_id) {
        meta.checkpoint_ref = None;
    }

    Ok(msg)
}

#[tauri::command]
pub async fn get_plan_checkpoint(session_id: String) -> Result<Option<String>, CmdError> {
    Ok(plan::get_checkpoint_ref(&session_id).await)
}

#[tauri::command]
pub async fn get_plan_file_path(session_id: String) -> Result<Option<String>, CmdError> {
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if !meta.file_path.is_empty() {
            return Ok(Some(meta.file_path));
        }
    }
    Ok(None)
}

#[tauri::command]
pub async fn cancel_plan_subagent(session_id: String) -> Result<(), CmdError> {
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
            Err(CmdError::msg("Cancel registry not initialized"))
        }
    } else {
        Ok(()) // No active plan sub-agent — nothing to cancel
    }
}
