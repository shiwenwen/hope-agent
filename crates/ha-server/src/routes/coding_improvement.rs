use axum::extract::{Path, Query};
use axum::Json;
use ha_core::coding_improvement::{
    ApplyCodingImprovementProposalResult, CodingBenchmarkCenterInput, CodingBenchmarkCenterReport,
    CodingEvalReleaseGateInput, CodingEvalReleaseGateReport, CodingEvalRunRecord,
    CodingImprovementActionPlan, CodingImprovementPromotionPlan, CodingImprovementProposal,
    CodingLearningGeneralizationInput, CodingLearningGeneralizationReport, CodingTrendReport,
    DistillCodingImprovementResult, GenerateCodingImprovementProposalsResult,
    PromoteCodingImprovementProposalResult, RecordCodingEvalRunInput,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendQuery {
    pub window_days: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateProposalsBody {
    pub window_days: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProposalStatusBody {
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEvalRunBody {
    pub input: RecordCodingEvalRunInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGateBody {
    pub input: CodingEvalReleaseGateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningGeneralizationBody {
    pub input: CodingLearningGeneralizationInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCenterBody {
    pub input: CodingBenchmarkCenterInput,
}

pub async fn get_coding_trend_report(
    Path(session_id): Path<String>,
    Query(query): Query<TrendQuery>,
) -> Result<Json<CodingTrendReport>, AppError> {
    Ok(Json(
        session_db()?.coding_trend_report(&session_id, query.window_days)?,
    ))
}

pub async fn list_coding_improvement_proposals(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<CodingImprovementProposal>>, AppError> {
    Ok(Json(
        session_db()?.list_coding_improvement_proposals(&session_id)?,
    ))
}

pub async fn generate_coding_improvement_proposals(
    Path(session_id): Path<String>,
    Json(body): Json<GenerateProposalsBody>,
) -> Result<Json<GenerateCodingImprovementProposalsResult>, AppError> {
    Ok(Json(session_db()?.generate_coding_improvement_proposals(
        &session_id,
        body.window_days,
    )?))
}

pub async fn distill_coding_improvement_proposals(
    Path(session_id): Path<String>,
    Json(body): Json<GenerateProposalsBody>,
) -> Result<Json<DistillCodingImprovementResult>, AppError> {
    Ok(Json(session_db()?.distill_coding_improvement_proposals(
        &session_id,
        body.window_days,
    )?))
}

pub async fn update_coding_improvement_proposal_status(
    Path(proposal_id): Path<String>,
    Json(body): Json<UpdateProposalStatusBody>,
) -> Result<Json<CodingImprovementProposal>, AppError> {
    session_db()?
        .update_coding_improvement_proposal_status(&proposal_id, &body.status)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn preview_coding_improvement_proposal_action(
    Path(proposal_id): Path<String>,
) -> Result<Json<CodingImprovementActionPlan>, AppError> {
    session_db()?
        .preview_coding_improvement_proposal_action(&proposal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn apply_coding_improvement_proposal(
    Path(proposal_id): Path<String>,
) -> Result<Json<ApplyCodingImprovementProposalResult>, AppError> {
    session_db()?
        .apply_coding_improvement_proposal(&proposal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn preview_coding_improvement_proposal_promotion(
    Path(proposal_id): Path<String>,
) -> Result<Json<CodingImprovementPromotionPlan>, AppError> {
    session_db()?
        .preview_coding_improvement_proposal_promotion(&proposal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn promote_coding_improvement_proposal(
    Path(proposal_id): Path<String>,
) -> Result<Json<PromoteCodingImprovementProposalResult>, AppError> {
    session_db()?
        .promote_coding_improvement_proposal(&proposal_id)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn record_coding_eval_run(
    Json(body): Json<RecordEvalRunBody>,
) -> Result<Json<CodingEvalRunRecord>, AppError> {
    session_db()?
        .record_coding_eval_run(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_coding_eval_release_gate(
    Json(body): Json<ReleaseGateBody>,
) -> Result<Json<CodingEvalReleaseGateReport>, AppError> {
    session_db()?
        .evaluate_coding_eval_release_gate(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_coding_learning_generalization(
    Json(body): Json<LearningGeneralizationBody>,
) -> Result<Json<CodingLearningGeneralizationReport>, AppError> {
    session_db()?
        .evaluate_coding_learning_generalization(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_coding_benchmark_center(
    Json(body): Json<BenchmarkCenterBody>,
) -> Result<Json<CodingBenchmarkCenterReport>, AppError> {
    session_db()?
        .get_coding_benchmark_center(body.input)
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
