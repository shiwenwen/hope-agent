use axum::extract::Path;
use axum::Json;
use ha_core::review::{ReviewFindingStatus, RunReviewInput};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn list_review_runs(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ha_core::review::ReviewRun>>, AppError> {
    Ok(Json(
        session_db()?.list_review_runs_for_session(&session_id, 100)?,
    ))
}

pub async fn get_review_run(
    Path(run_id): Path<String>,
) -> Result<Json<Option<ha_core::review::ReviewRunSnapshot>>, AppError> {
    Ok(Json(session_db()?.review_run_snapshot(&run_id, 200)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReviewBody {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub focus_paths: Vec<String>,
}

pub async fn run_code_review(
    Path(session_id): Path<String>,
    Json(body): Json<RunReviewBody>,
) -> Result<Json<ha_core::review::ReviewRunSnapshot>, AppError> {
    ha_core::review::run_review_for_session(
        session_db()?.clone(),
        session_id,
        RunReviewInput {
            scope: body.scope,
            base_ref: body.base_ref,
            goal_id: body.goal_id,
            profiles: body.profiles,
            focus_paths: body.focus_paths,
        },
    )
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReviewFindingStatusBody {
    pub status: String,
}

pub async fn update_review_finding_status(
    Path(finding_id): Path<String>,
    Json(body): Json<UpdateReviewFindingStatusBody>,
) -> Result<Json<ha_core::review::ReviewFinding>, AppError> {
    let status = ReviewFindingStatus::from_str(&body.status)
        .ok_or_else(|| AppError::bad_request("invalid review finding status"))?;
    session_db()?
        .update_review_finding_status(&finding_id, status)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
