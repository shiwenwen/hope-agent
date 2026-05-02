use crate::commands::CmdError;
use crate::plan::{self, PlanModeState, PlanVersionInfo};
use ha_core::app_info;
use ha_core::ask_user::AskUserQuestionAnswer;

#[tauri::command]
pub async fn get_plan_mode(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<String, CmdError> {
    if let Ok(Some(meta)) = app_state.session_db.get_session(&session_id) {
        if meta.plan_mode == PlanModeState::Off {
            plan::set_plan_state(&session_id, PlanModeState::Off).await;
            return Ok("off".to_string());
        }
        // Restore in-memory state from DB + plan file
        plan::restore_from_db(&session_id, meta.plan_mode).await;
        return Ok(meta.plan_mode.as_str().to_string());
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
    if state == "paused" {
        return Err(CmdError::msg("plan mode 'paused' state has been removed"));
    }
    let plan_state = PlanModeState::from_str(&state);
    let previous_state = plan::get_plan_state(&session_id).await;
    let persisted_plan_mode = app_state
        .session_db
        .get_session(&session_id)
        .ok()
        .flatten()
        .map(|meta| meta.plan_mode);
    let checkpoint_exists = plan::get_checkpoint_ref(&session_id).await.is_some();
    let should_create_checkpoint = plan::should_create_execution_checkpoint(
        &plan_state,
        &previous_state,
        persisted_plan_mode,
        checkpoint_exists,
    );
    let checkpoint_to_cleanup =
        if plan_state == PlanModeState::Completed || plan_state == PlanModeState::Off {
            plan::get_checkpoint_ref(&session_id).await
        } else {
            None
        };

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

    if !plan::set_plan_state(&session_id, plan_state).await {
        return Err(CmdError::msg(format!(
            "Invalid plan mode transition to '{}'",
            plan_state.as_str()
        )));
    }

    // Clean up checkpoint on successful completion or exit
    if let Some(ref_name) = checkpoint_to_cleanup {
        plan::cleanup_checkpoint(&ref_name);
    }

    // Create git checkpoint AFTER PlanMeta entry exists in the store
    if should_create_checkpoint {
        plan::create_checkpoint_for_session(&session_id).await;
    }
    // Persist to DB
    let db = &app_state.session_db;
    db.update_session_plan_mode(&session_id, plan_state)?;
    Ok(())
}

#[tauri::command]
pub async fn get_plan_content(session_id: String) -> Result<Option<String>, CmdError> {
    plan::load_plan_file(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn save_plan_content(session_id: String, content: String) -> Result<(), CmdError> {
    plan::save_plan_file(&session_id, &content)?;
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
    if let Some(path) = plan::find_plan_file(&session_id)? {
        return Ok(Some(path.to_string_lossy().to_string()));
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
