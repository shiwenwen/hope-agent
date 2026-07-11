use crate::commands::CmdError;
use ha_core::review::{
    ReviewFinding, ReviewFindingStatus, ReviewRun, ReviewRunSnapshot, RunReviewInput,
};
use ha_core::session::SessionIdeContext;

#[tauri::command]
pub async fn list_review_runs(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ReviewRun>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_review_runs_for_session(&session_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_review_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<ReviewRunSnapshot>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.review_run_snapshot(&run_id, 200))
        .await
        .map_err(Into::into)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn run_code_review(
    session_id: String,
    scope: Option<String>,
    base_ref: Option<String>,
    goal_id: Option<String>,
    profiles: Option<Vec<String>>,
    focus_paths: Option<Vec<String>>,
    ide_context: Option<SessionIdeContext>,
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
            ide_context,
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.update_review_finding_status(&finding_id, status))
        .await
        .map_err(Into::into)
}
