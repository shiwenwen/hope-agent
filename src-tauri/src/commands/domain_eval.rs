use crate::commands::CmdError;
use ha_core::domain_eval::{
    DomainEvalRunRecord, DomainEvalTask, DomainQualityGateInput, DomainQualityGateReport,
    ListDomainEvalRunsInput, ListDomainEvalTasksInput, RunDomainEvalTaskInput,
};

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
pub async fn evaluate_domain_quality_gate(
    input: DomainQualityGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainQualityGateReport, CmdError> {
    app_state
        .session_db
        .evaluate_domain_quality_gate(input)
        .map_err(Into::into)
}
