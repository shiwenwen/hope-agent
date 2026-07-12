use axum::extract::{Path, Query};
use axum::Json;
use ha_core::activity::AutonomyActivity;
use ha_core::goal::{
    AppendGoalFollowUpInput, CloseGoalInput, CreateGoalInput, GoalClosureDecision, GoalSnapshot,
    GoalWatchdogFinding, UpdateGoalInput,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn get_active_goal(
    Path(session_id): Path<String>,
) -> Result<Json<Option<GoalSnapshot>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.active_goal_for_session(&session_id))
            .await?,
    ))
}

pub async fn get_autonomy_activity(
    Path(session_id): Path<String>,
) -> Result<Json<AutonomyActivity>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.autonomy_activity_for_session(&session_id))
            .await?,
    ))
}

pub async fn get_goal(Path(goal_id): Path<String>) -> Result<Json<Option<GoalSnapshot>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.goal_snapshot(&goal_id, 200)).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListGoalWatchdogQuery {
    #[serde(default)]
    pub stale_secs: Option<i64>,
}

pub async fn list_goal_watchdog_findings(
    Path(session_id): Path<String>,
    Query(query): Query<ListGoalWatchdogQuery>,
) -> Result<Json<Vec<GoalWatchdogFinding>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| {
            db.list_goal_watchdog_findings(&session_id, query.stale_secs.unwrap_or(300))
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGoalBody {
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub workflow_template_id: Option<String>,
    #[serde(default)]
    pub workflow_template_version: Option<String>,
    #[serde(default)]
    pub workflow_task_type: Option<String>,
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
    let db = session_db()?;
    db.run(move |db| {
        db.create_goal(CreateGoalInput {
            session_id,
            objective: body.objective,
            completion_criteria: body.completion_criteria.unwrap_or_default(),
            domain: body.domain,
            workflow_template_id: body.workflow_template_id,
            workflow_template_version: body.workflow_template_version,
            workflow_task_type: body.workflow_task_type,
            budget_token_limit: body.budget_token_limit,
            budget_time_limit_secs: body.budget_time_limit_secs,
            budget_turn_limit: body.budget_turn_limit,
        })
    })
    .await
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
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub workflow_template_id: Option<String>,
    #[serde(default)]
    pub workflow_template_version: Option<String>,
    #[serde(default)]
    pub workflow_task_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseGoalBody {
    pub decision: GoalClosureDecision,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub follow_up_items: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendGoalFollowUpBody {
    pub items: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}

pub async fn update_goal(
    Path(goal_id): Path<String>,
    Json(body): Json<UpdateGoalBody>,
) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| {
        db.update_goal(UpdateGoalInput {
            goal_id,
            objective: body.objective,
            completion_criteria: body.completion_criteria,
            domain: body.domain,
            workflow_template_id: body.workflow_template_id,
            workflow_template_version: body.workflow_template_version,
            workflow_task_type: body.workflow_task_type,
        })
    })
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn pause_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.pause_goal(&goal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn resume_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.resume_goal(&goal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn clear_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.clear_goal(&goal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_goal(Path(goal_id): Path<String>) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_goal(&goal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn close_goal(
    Path(goal_id): Path<String>,
    Json(body): Json<CloseGoalBody>,
) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| {
        db.close_goal(CloseGoalInput {
            goal_id,
            decision: body.decision,
            reason: body.reason,
            follow_up_items: body.follow_up_items,
        })
    })
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn append_goal_follow_up(
    Path(goal_id): Path<String>,
    Json(body): Json<AppendGoalFollowUpBody>,
) -> Result<Json<GoalSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| {
        db.append_goal_follow_up(AppendGoalFollowUpInput {
            goal_id,
            items: body.items,
            source: body.source,
        })
    })
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}
