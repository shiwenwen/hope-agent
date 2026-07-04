use crate::commands::CmdError;
use ha_core::domain_eval::{
    DomainEvalCalibrationRecord, DomainEvalFixtureReport, DomainEvalRunRecord, DomainEvalTask,
    DomainQualityGateInput, DomainQualityGateReport, ImportDomainEvalCaseInput,
    ImportDomainEvalCaseResult, ListDomainEvalCalibrationsInput, ListDomainEvalFixtureRunsInput,
    ListDomainEvalRunsInput, ListDomainEvalTasksInput, RecordDomainEvalCalibrationInput,
    RunDomainEvalFixtureInput, RunDomainEvalTaskInput,
};
use ha_core::session::SessionDB;

#[tauri::command]
pub async fn list_domain_eval_tasks(
    input: ListDomainEvalTasksInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalTask>, CmdError> {
    app_state
        .session_db
        .list_domain_eval_tasks(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_domain_eval_task(
    input: RunDomainEvalTaskInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalRunRecord, CmdError> {
    app_state
        .session_db
        .run_domain_eval_task(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_domain_eval_fixture(
    input: RunDomainEvalFixtureInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalFixtureReport, CmdError> {
    SessionDB::run_domain_eval_fixture(app_state.session_db.clone(), input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn import_domain_eval_case(
    input: ImportDomainEvalCaseInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ImportDomainEvalCaseResult, CmdError> {
    app_state
        .session_db
        .import_domain_eval_case(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn record_domain_eval_calibration(
    input: RecordDomainEvalCalibrationInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalCalibrationRecord, CmdError> {
    app_state
        .session_db
        .record_domain_eval_calibration(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_eval_calibrations(
    input: ListDomainEvalCalibrationsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalCalibrationRecord>, CmdError> {
    app_state
        .session_db
        .list_domain_eval_calibrations(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_eval_runs(
    input: ListDomainEvalRunsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalRunRecord>, CmdError> {
    app_state
        .session_db
        .list_domain_eval_runs(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_eval_fixture_runs(
    input: ListDomainEvalFixtureRunsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ha_core::domain_eval::DomainEvalFixtureRunRecord>, CmdError> {
    app_state
        .session_db
        .list_domain_eval_fixture_runs(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_quality_gate(
    input: DomainQualityGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainQualityGateReport, CmdError> {
    app_state
        .session_db
        .evaluate_domain_quality_gate(input)
        .map_err(Into::into)
}
