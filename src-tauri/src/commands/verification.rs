use crate::commands::CmdError;
use ha_core::verification::{PlanVerificationInput, VerificationRun, VerificationRunSnapshot};

#[tauri::command]
pub async fn list_verification_runs(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<VerificationRun>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_verification_runs_for_session(&session_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_verification_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<VerificationRunSnapshot>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.verification_run_snapshot(&run_id, 200))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn plan_smart_verification(
    session_id: String,
    scope: Option<String>,
    goal_id: Option<String>,
    max_commands: Option<usize>,
    focus_paths: Option<Vec<String>>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<VerificationRunSnapshot, CmdError> {
    ha_core::verification::plan_verification_for_session(
        app_state.session_db.clone(),
        session_id,
        PlanVerificationInput {
            scope,
            goal_id,
            max_commands,
            focus_paths: focus_paths.unwrap_or_default(),
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn run_smart_verification(
    session_id: String,
    scope: Option<String>,
    goal_id: Option<String>,
    max_commands: Option<usize>,
    focus_paths: Option<Vec<String>>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<VerificationRunSnapshot, CmdError> {
    ha_core::verification::run_verification_for_session(
        app_state.session_db.clone(),
        session_id,
        PlanVerificationInput {
            scope,
            goal_id,
            max_commands,
            focus_paths: focus_paths.unwrap_or_default(),
        },
    )
    .await
    .map_err(Into::into)
}
