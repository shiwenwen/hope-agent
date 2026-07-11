use axum::Json;
use ha_core::domain_workflow::{
    DomainArtifactExportGuardInput, DomainArtifactExportGuardReport,
    DomainConnectorActionGuardInput, DomainConnectorActionGuardReport, DomainConnectorE2EGateInput,
    DomainConnectorE2EGateReport, DomainEvidenceItem, DomainWorkflowDraft, DomainWorkflowTemplate,
    ListDomainEvidenceInput, ListDomainWorkflowTemplatesInput, PreviewDomainWorkflowInput,
    RecordDomainEvidenceInput, SaveDomainWorkflowTemplateInput,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTemplatesBody {
    pub input: ListDomainWorkflowTemplatesInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTemplateBody {
    pub input: SaveDomainWorkflowTemplateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWorkflowBody {
    pub input: PreviewDomainWorkflowInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEvidenceBody {
    pub input: RecordDomainEvidenceInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEvidenceBody {
    pub input: ListDomainEvidenceInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateExportGuardBody {
    pub input: DomainArtifactExportGuardInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateConnectorActionGuardBody {
    pub input: DomainConnectorActionGuardInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateConnectorE2EGateBody {
    pub input: DomainConnectorE2EGateInput,
}

pub async fn list_domain_workflow_templates(
    Json(body): Json<ListTemplatesBody>,
) -> Result<Json<Vec<DomainWorkflowTemplate>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_domain_workflow_templates(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn save_domain_workflow_template(
    Json(body): Json<SaveTemplateBody>,
) -> Result<Json<DomainWorkflowTemplate>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.save_domain_workflow_template(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn preview_domain_workflow(
    Json(body): Json<PreviewWorkflowBody>,
) -> Result<Json<DomainWorkflowDraft>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.preview_domain_workflow(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn record_domain_evidence(
    Json(body): Json<RecordEvidenceBody>,
) -> Result<Json<DomainEvidenceItem>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.record_domain_evidence(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn list_domain_evidence(
    Json(body): Json<ListEvidenceBody>,
) -> Result<Json<Vec<DomainEvidenceItem>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_domain_evidence(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_domain_artifact_export_guard(
    Json(body): Json<EvaluateExportGuardBody>,
) -> Result<Json<DomainArtifactExportGuardReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_domain_artifact_export_guard(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_domain_connector_action_guard(
    Json(body): Json<EvaluateConnectorActionGuardBody>,
) -> Result<Json<DomainConnectorActionGuardReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_domain_connector_action_guard(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_domain_connector_e2e_gate(
    Json(body): Json<EvaluateConnectorE2EGateBody>,
) -> Result<Json<DomainConnectorE2EGateReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_domain_connector_e2e_gate(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
