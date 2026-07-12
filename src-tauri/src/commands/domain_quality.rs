use crate::commands::CmdError;
use ha_core::domain_quality::{DomainQualityRun, DomainQualityRunSnapshot, RunDomainQualityInput};

#[tauri::command]
pub async fn list_domain_quality_runs(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainQualityRun>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_quality_runs_for_session(&session_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_domain_quality_run(
    run_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<DomainQualityRunSnapshot>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.domain_quality_run_snapshot(&run_id, 100))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_domain_quality(
    input: RunDomainQualityInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainQualityRunSnapshot, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.run_domain_quality_for_session(input))
        .await
        .map_err(Into::into)
}
