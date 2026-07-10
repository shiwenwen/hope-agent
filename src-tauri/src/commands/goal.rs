use crate::commands::CmdError;
use ha_core::activity::AutonomyActivity;
use ha_core::goal::{
    AppendGoalFollowUpInput, CloseGoalInput, CreateGoalInput, GoalClosureDecision, GoalSnapshot,
    GoalWatchdogFinding, UpdateGoalInput,
};

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
pub async fn get_autonomy_activity(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<AutonomyActivity, CmdError> {
    app_state
        .session_db
        .autonomy_activity_for_session(&session_id)
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
pub async fn list_goal_watchdog_findings(
    session_id: String,
    stale_secs: Option<i64>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<GoalWatchdogFinding>, CmdError> {
    app_state
        .session_db
        .list_goal_watchdog_findings(&session_id, stale_secs.unwrap_or(300))
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_goal(
    session_id: String,
    objective: String,
    completion_criteria: Option<String>,
    domain: Option<String>,
    workflow_template_id: Option<String>,
    workflow_template_version: Option<String>,
    workflow_task_type: Option<String>,
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
            domain,
            workflow_template_id,
            workflow_template_version,
            workflow_task_type,
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
    domain: Option<String>,
    workflow_template_id: Option<String>,
    workflow_template_version: Option<String>,
    workflow_task_type: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .update_goal(UpdateGoalInput {
            goal_id,
            objective,
            completion_criteria,
            domain,
            workflow_template_id,
            workflow_template_version,
            workflow_task_type,
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

#[tauri::command]
pub async fn close_goal(
    goal_id: String,
    decision: GoalClosureDecision,
    reason: Option<String>,
    follow_up_items: Option<Vec<String>>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .close_goal(CloseGoalInput {
            goal_id,
            decision,
            reason,
            follow_up_items: follow_up_items.unwrap_or_default(),
        })
        .map_err(Into::into)
}

#[tauri::command]
pub async fn append_goal_follow_up(
    goal_id: String,
    items: Vec<String>,
    source: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GoalSnapshot, CmdError> {
    app_state
        .session_db
        .append_goal_follow_up(AppendGoalFollowUpInput {
            goal_id,
            items,
            source,
        })
        .map_err(Into::into)
}
