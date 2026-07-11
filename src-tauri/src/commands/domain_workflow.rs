use crate::commands::CmdError;
use ha_core::domain_workflow::{
    DomainArtifactExportGuardInput, DomainArtifactExportGuardReport,
    DomainConnectorActionGuardInput, DomainConnectorActionGuardReport, DomainConnectorE2EGateInput,
    DomainConnectorE2EGateReport, DomainEvidenceItem, DomainWorkflowDraft, DomainWorkflowTemplate,
    ListDomainEvidenceInput, ListDomainWorkflowTemplatesInput, PreviewDomainWorkflowInput,
    RecordDomainEvidenceInput, SaveDomainWorkflowTemplateInput,
};

#[tauri::command]
pub async fn list_domain_workflow_templates(
    input: ListDomainWorkflowTemplatesInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainWorkflowTemplate>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_workflow_templates(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_domain_workflow_template(
    input: SaveDomainWorkflowTemplateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainWorkflowTemplate, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.save_domain_workflow_template(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_domain_workflow(
    input: PreviewDomainWorkflowInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainWorkflowDraft, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.preview_domain_workflow(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn record_domain_evidence(
    input: RecordDomainEvidenceInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvidenceItem, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.record_domain_evidence(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_evidence(
    input: ListDomainEvidenceInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvidenceItem>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_evidence(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_artifact_export_guard(
    input: DomainArtifactExportGuardInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainArtifactExportGuardReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_domain_artifact_export_guard(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_connector_action_guard(
    input: DomainConnectorActionGuardInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainConnectorActionGuardReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_domain_connector_action_guard(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_connector_e2e_gate(
    input: DomainConnectorE2EGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainConnectorE2EGateReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_domain_connector_e2e_gate(input))
        .await
        .map_err(Into::into)
}
