use axum::extract::Path;
use axum::Json;
use ha_core::verification::PlanVerificationInput;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn list_verification_runs(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ha_core::verification::VerificationRun>>, AppError> {
    Ok(Json(
        session_db()?.list_verification_runs_for_session(&session_id, 100)?,
    ))
}

pub async fn get_verification_run(
    Path(run_id): Path<String>,
) -> Result<Json<Option<ha_core::verification::VerificationRunSnapshot>>, AppError> {
    Ok(Json(session_db()?.verification_run_snapshot(&run_id, 200)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanVerificationBody {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub max_commands: Option<usize>,
    #[serde(default)]
    pub focus_paths: Vec<String>,
}

pub async fn plan_smart_verification(
    Path(session_id): Path<String>,
    Json(body): Json<PlanVerificationBody>,
) -> Result<Json<ha_core::verification::VerificationRunSnapshot>, AppError> {
    ha_core::verification::plan_verification_for_session(
        session_db()?.clone(),
        session_id,
        PlanVerificationInput {
            scope: body.scope,
            goal_id: body.goal_id,
            max_commands: body.max_commands,
            focus_paths: body.focus_paths,
        },
    )
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn run_smart_verification(
    Path(session_id): Path<String>,
    Json(body): Json<PlanVerificationBody>,
) -> Result<Json<ha_core::verification::VerificationRunSnapshot>, AppError> {
    ha_core::verification::run_verification_for_session(
        session_db()?.clone(),
        session_id,
        PlanVerificationInput {
            scope: body.scope,
            goal_id: body.goal_id,
            max_commands: body.max_commands,
            focus_paths: body.focus_paths,
        },
    )
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}
