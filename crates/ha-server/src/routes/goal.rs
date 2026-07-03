use axum::extract::Path;
use axum::Json;
use ha_core::goal::{CreateGoalInput, GoalSnapshot, UpdateGoalInput};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn get_active_goal(
    Path(session_id): Path<String>,
) -> Result<Json<Option<GoalSnapshot>>, AppError> {
    Ok(Json(session_db()?.active_goal_for_session(&session_id)?))
}

pub async fn get_goal(Path(goal_id): Path<String>) -> Result<Json<Option<GoalSnapshot>>, AppError> {
    Ok(Json(session_db()?.goal_snapshot(&goal_id, 200)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGoalBody {
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: Option<String>,
    #[serde(default)]
    pub budget_token_limit: Option<i64>,
    #[serde(default)]
    pub budget_time_limit_secs: Option<i64>,
    #[serde(default)]
    pub budget_turn_limit: Option<i64>,
}

pub async fn create_goal(
    Path(session_id): Path<String>,
    Json(body): Json<CreateGoalBody>,
) -> Result<Json<GoalSnapshot>, AppError> {
    session_db()?
        .create_goal(CreateGoalInput {
            session_id,
            objective: body.objective,
            completion_criteria: body.completion_criteria.unwrap_or_default(),
            budget_token_limit: body.budget_token_limit,
            budget_time_limit_secs: body.budget_time_limit_secs,
            budget_turn_limit: body.budget_turn_limit,
        })
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGoalBody {
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub completion_criteria: Option<String>,
}

pub async fn update_goal(
    Path(goal_id): Path<String>,
    Json(body): Json<UpdateGoalBody>,
) -> Result<Json<GoalSnapshot>, AppError> {
    session_db()?
        .update_goal(UpdateGoalInput {
            goal_id,
            objective: body.objective,
            completion_criteria: body.completion_criteria,
        })
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn pause_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    session_db()?
        .pause_goal(&goal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn resume_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    session_db()?
        .resume_goal(&goal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn clear_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    session_db()?
        .clear_goal(&goal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    session_db()?
        .evaluate_goal(&goal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
