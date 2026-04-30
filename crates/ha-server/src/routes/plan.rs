use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::ask_user::{self as ask_user_mod, AskUserQuestionAnswer};
use ha_core::plan::{self, PlanModeState, PlanStep, PlanStepStatus, PlanVersionInfo};

use crate::error::AppError;
use crate::routes::helpers::session_db;

/// `GET /api/plan/{session_id}/mode`
pub async fn get_plan_mode(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    if let Ok(Some(meta)) = session_db()?.get_session(&session_id) {
        if meta.plan_mode == PlanModeState::Off {
            plan::set_plan_state(&session_id, PlanModeState::Off).await;
            return Ok(Json(json!({ "state": "off" })));
        }
        plan::restore_from_db(&session_id, meta.plan_mode).await;
        return Ok(Json(json!({ "state": meta.plan_mode.as_str() })));
    }
    let state = plan::get_plan_state(&session_id).await;
    if state != PlanModeState::Off {
        return Ok(Json(json!({ "state": state.as_str() })));
    }
    Ok(Json(json!({ "state": "off" })))
}

#[derive(Debug, Deserialize)]
pub struct SetModeBody {
    pub state: String,
}

/// `POST /api/plan/{session_id}/mode`
pub async fn set_plan_mode(
    Path(session_id): Path<String>,
    Json(body): Json<SetModeBody>,
) -> Result<Json<Value>, AppError> {
    let plan_state = PlanModeState::from_str(&body.state);
    let previous_state = plan::get_plan_state(&session_id).await;
    let persisted_plan_mode = session_db()?
        .get_session(&session_id)
        .map_err(|e| AppError::internal(e.to_string()))?
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

    if plan_state == PlanModeState::Off {
        if let Some(run_id) = plan::get_active_plan_run_id(&session_id).await {
            if let Some(cancels) = ha_core::get_subagent_cancels() {
                cancels.cancel(&run_id);
            }
        }
    }

    if !plan::set_plan_state(&session_id, plan_state).await {
        return Err(AppError::bad_request(format!(
            "Invalid plan mode transition to '{}'",
            plan_state.as_str()
        )));
    }

    if let Some(ref_name) = checkpoint_to_cleanup {
        plan::cleanup_checkpoint(&ref_name);
    }

    if should_create_checkpoint {
        plan::create_checkpoint_for_session(&session_id).await;
    }
    session_db()?
        .update_session_plan_mode(&session_id, plan_state)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "updated": true })))
}

/// `GET /api/plan/{session_id}/content`
pub async fn get_plan_content(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    let content = plan::load_plan_file(&session_id)?;
    Ok(Json(json!({ "content": content })))
}

#[derive(Debug, Deserialize)]
pub struct SaveContentBody {
    pub content: String,
}

/// `PUT /api/plan/{session_id}/content`
pub async fn save_plan_content(
    Path(session_id): Path<String>,
    Json(body): Json<SaveContentBody>,
) -> Result<Json<Value>, AppError> {
    plan::save_plan_file(&session_id, &body.content)?;
    let steps = plan::parse_plan_steps(&body.content);
    plan::update_plan_steps(&session_id, steps).await;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/plan/{session_id}/steps`
pub async fn get_plan_steps(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<PlanStep>>, AppError> {
    if let Ok(Some(session_meta)) = session_db()?.get_session(&session_id) {
        if session_meta.plan_mode == PlanModeState::Off {
            plan::set_plan_state(&session_id, PlanModeState::Off).await;
            return Ok(Json(Vec::new()));
        }
        plan::restore_from_db(&session_id, session_meta.plan_mode).await;
        if let Some(meta) = plan::get_plan_meta(&session_id).await {
            return Ok(Json(meta.steps));
        }
    }
    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if !meta.steps.is_empty() {
            return Ok(Json(meta.steps));
        }
    }
    Ok(Json(Vec::new()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStepBody {
    pub step_index: usize,
    pub status: String,
}

/// `POST /api/plan/{session_id}/steps/update`
pub async fn update_plan_step_status(
    Path(session_id): Path<String>,
    Json(body): Json<UpdateStepBody>,
) -> Result<Json<Value>, AppError> {
    let step_status = PlanStepStatus::from_str(&body.status);
    plan::update_step_status(&session_id, body.step_index, step_status, None).await;

    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "plan_step_updated",
            json!({
                "sessionId": session_id,
                "stepIndex": body.step_index,
                "status": body.status,
            }),
        );
    }

    if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if meta.all_terminal() && meta.state == PlanModeState::Executing {
            if let Some(ref_name) = plan::get_checkpoint_ref(&session_id).await {
                plan::cleanup_checkpoint(&ref_name);
            }
            plan::set_plan_state(&session_id, PlanModeState::Completed).await;
            let _ = session_db()?.update_session_plan_mode(&session_id, PlanModeState::Completed);
            if let Some(bus) = ha_core::get_event_bus() {
                bus.emit(
                    "plan_mode_changed",
                    json!({
                        "sessionId": session_id,
                        "state": "completed",
                        "reason": "all_steps_completed",
                    }),
                );
            }
        }
    }

    Ok(Json(json!({ "updated": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondQuestionBody {
    pub request_id: String,
    pub answers: Vec<AskUserQuestionAnswer>,
}

/// `POST /api/ask_user/respond`
pub async fn respond_ask_user_question(
    Json(body): Json<RespondQuestionBody>,
) -> Result<Json<Value>, AppError> {
    ask_user_mod::submit_ask_user_question_response(&body.request_id, body.answers).await?;
    Ok(Json(json!({ "submitted": true })))
}

/// `GET /api/plan/{session_id}/pending-ask-user`
pub async fn get_pending_ask_user_group(
    Path(session_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let group = ask_user_mod::find_live_pending_group_for_session(&session_id).await?;
    Ok(Json(json!(group)))
}

/// `GET /api/plan/{session_id}/versions`
pub async fn get_plan_versions(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<PlanVersionInfo>>, AppError> {
    Ok(Json(plan::list_plan_versions(&session_id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadVersionQuery {
    pub file_path: String,
}

/// `POST /api/plan/version/load`
pub async fn load_plan_version_content(
    Json(body): Json<LoadVersionQuery>,
) -> Result<Json<Value>, AppError> {
    let content = plan::load_plan_version(&body.file_path)?;
    Ok(Json(json!({ "content": content })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreVersionBody {
    pub file_path: String,
}

/// `POST /api/plan/{session_id}/version/restore`
pub async fn restore_plan_version(
    Path(session_id): Path<String>,
    Json(body): Json<RestoreVersionBody>,
) -> Result<Json<Value>, AppError> {
    let content = plan::load_plan_version(&body.file_path)?;
    plan::save_plan_file(&session_id, &content)?;
    let steps = plan::parse_plan_steps(&content);
    plan::update_plan_steps(&session_id, steps).await;
    Ok(Json(json!({ "restored": true })))
}

/// `POST /api/plan/{session_id}/rollback`
pub async fn plan_rollback(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    let checkpoint = plan::get_checkpoint_ref(&session_id)
        .await
        .ok_or_else(|| AppError::bad_request("No git checkpoint found for this plan execution"))?;

    let msg = plan::rollback_to_checkpoint(&checkpoint)?;

    let mut map = plan::store().write().await;
    if let Some(meta) = map.get_mut(&session_id) {
        meta.checkpoint_ref = None;
    }
    drop(map);

    Ok(Json(json!({ "message": msg })))
}

/// `GET /api/plan/{session_id}/checkpoint`
pub async fn get_plan_checkpoint(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "checkpoint": plan::get_checkpoint_ref(&session_id).await,
    })))
}

/// `GET /api/plan/{session_id}/file-path`
pub async fn get_plan_file_path(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    let path = if let Some(meta) = plan::get_plan_meta(&session_id).await {
        if !meta.file_path.is_empty() {
            Some(meta.file_path)
        } else {
            None
        }
    } else {
        plan::find_plan_file(&session_id)?.map(|path| path.to_string_lossy().to_string())
    };
    Ok(Json(json!({ "filePath": path })))
}

/// `POST /api/plan/{session_id}/cancel`
pub async fn cancel_plan_subagent(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    if let Some(run_id) = plan::get_active_plan_run_id(&session_id).await {
        let cancels = ha_core::get_subagent_cancels()
            .ok_or_else(|| AppError::internal("Cancel registry not initialized"))?;
        cancels.cancel(&run_id);
    }
    Ok(Json(json!({ "cancelled": true })))
}
