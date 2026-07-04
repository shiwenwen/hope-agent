use axum::Json;
use ha_core::domain_workflow::{
    DomainArtifactExportGuardInput, DomainArtifactExportGuardReport, DomainEvidenceItem,
    DomainWorkflowDraft, DomainWorkflowTemplate, ListDomainEvidenceInput,
    ListDomainWorkflowTemplatesInput, PreviewDomainWorkflowInput, RecordDomainEvidenceInput,
    SaveDomainWorkflowTemplateInput,
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

pub async fn list_domain_workflow_templates(
    Json(body): Json<ListTemplatesBody>,
) -> Result<Json<Vec<DomainWorkflowTemplate>>, AppError> {
    session_db()?
        .list_domain_workflow_templates(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn save_domain_workflow_template(
    Json(body): Json<SaveTemplateBody>,
) -> Result<Json<DomainWorkflowTemplate>, AppError> {
    session_db()?
        .save_domain_workflow_template(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn preview_domain_workflow(
    Json(body): Json<PreviewWorkflowBody>,
) -> Result<Json<DomainWorkflowDraft>, AppError> {
    session_db()?
        .preview_domain_workflow(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn record_domain_evidence(
    Json(body): Json<RecordEvidenceBody>,
) -> Result<Json<DomainEvidenceItem>, AppError> {
    session_db()?
        .record_domain_evidence(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn list_domain_evidence(
    Json(body): Json<ListEvidenceBody>,
) -> Result<Json<Vec<DomainEvidenceItem>>, AppError> {
    session_db()?
        .list_domain_evidence(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_domain_artifact_export_guard(
    Json(body): Json<EvaluateExportGuardBody>,
) -> Result<Json<DomainArtifactExportGuardReport>, AppError> {
    session_db()?
        .evaluate_domain_artifact_export_guard(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
