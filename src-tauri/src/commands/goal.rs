use crate::commands::CmdError;
use ha_core::goal::{CreateGoalInput, GoalSnapshot, UpdateGoalInput};

#[tauri::command]
pub async fn get_active_goal(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<GoalSnapshot>, CmdError> {
    app_state
        .session_db
        .active_goal_for_session(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_goal(
    goal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<GoalSnapshot>, CmdError> {
    app_state
        .session_db
        .goal_snapshot(&goal_id, 200)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_goal(
    session_id: String,
    objective: String,
    completion_criteria: Option<String>,
    budget_token_limit: Option<i64>,
    budget_time_limit_secs: Option<i64>,
    budget_turn_limit: Option<i64>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .create_goal(CreateGoalInput {
            session_id,
            objective,
            completion_criteria: completion_criteria.unwrap_or_default(),
            budget_token_limit,
            budget_time_limit_secs,
            budget_turn_limit,
        })
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_goal(
    goal_id: String,
    objective: Option<String>,
    completion_criteria: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .update_goal(UpdateGoalInput {
            goal_id,
            objective,
            completion_criteria,
        })
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pause_goal(
    goal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .pause_goal(&goal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_goal(
    goal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .resume_goal(&goal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_goal(
    goal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .clear_goal(&goal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_goal(
    goal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .evaluate_goal(&goal_id)
        .map_err(Into::into)
}
