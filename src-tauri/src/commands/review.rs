use crate::commands::CmdError;
use ha_core::review::{
    ReviewFinding, ReviewFindingStatus, ReviewRun, ReviewRunSnapshot, RunReviewInput,
};

#[tauri::command]
pub async fn list_review_runs(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ReviewRun>, CmdError> {
    app_state
        .session_db
        .list_review_runs_for_session(&session_id, 100)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_review_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<ReviewRunSnapshot>, CmdError> {
    app_state
        .session_db
        .review_run_snapshot(&run_id, 200)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_code_review(
    session_id: String,
    scope: Option<String>,
    base_ref: Option<String>,
    goal_id: Option<String>,
    profiles: Option<Vec<String>>,
    focus_paths: Option<Vec<String>>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ReviewRunSnapshot, CmdError> {
    ha_core::review::run_review_for_session(
        app_state.session_db.clone(),
        session_id,
        RunReviewInput {
            scope,
            base_ref,
            goal_id,
            profiles: profiles.unwrap_or_default(),
            focus_paths: focus_paths.unwrap_or_default(),
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn update_review_finding_status(
    finding_id: String,
    status: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ReviewFinding, CmdError> {
    let status = ReviewFindingStatus::from_str(&status)
        .ok_or_else(|| CmdError::msg(format!("Invalid review finding status: {status}")))?;
    app_state
        .session_db
        .update_review_finding_status(&finding_id, status)
        .map_err(Into::into)
}
