use axum::extract::{Path, Query};
use axum::Json;
use ha_core::coding_improvement::{
    ApplyCodingImprovementProposalResult, CodingBenchmarkBacklogItem,
    CodingBenchmarkBacklogListInput, CodingBenchmarkBacklogMaterializeInput,
    CodingBenchmarkBacklogMaterializeResult, CodingBenchmarkBacklogStatusInput,
    CodingBenchmarkCampaign, CodingBenchmarkCampaignCreateInput, CodingBenchmarkCampaignListInput,
    CodingBenchmarkCampaignRunInput, CodingBenchmarkCenterInput, CodingBenchmarkCenterReport,
    CodingBenchmarkComparisonInput, CodingBenchmarkCorpusHealthInput,
    CodingBenchmarkCorpusHealthReport, CodingBenchmarkLeaderboardInput,
    CodingBenchmarkLeaderboardReport, CodingBenchmarkReport, CodingBenchmarkReportGenerateInput,
    CodingBenchmarkReportListInput, CodingBenchmarkReportMarkInput, CodingBenchmarkTaskPack,
    CodingBenchmarkTaskPackImportInput, CodingBenchmarkTaskPackListInput,
    CodingBenchmarkTaskPackStatusInput, CodingBenchmarkTaskPackValidateInput,
    CodingBenchmarkTaskPackValidationReport, CodingContinuousBenchmarkGateInput,
    CodingContinuousBenchmarkGateReport, CodingEvalReleaseGateInput, CodingEvalReleaseGateReport,
    CodingEvalRunRecord, CodingImprovementActionPlan, CodingImprovementPromotionPlan,
    CodingImprovementProposal, CodingLearningGeneralizationInput,
    CodingLearningGeneralizationReport, CodingTrendReport, DistillCodingImprovementResult,
    GenerateCodingImprovementProposalsInput, GenerateCodingImprovementProposalsResult,
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
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    #[serde(default)]
    pub proposal_kinds: Vec<String>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCampaignCreateBody {
    pub input: CodingBenchmarkCampaignCreateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCampaignListBody {
    pub input: CodingBenchmarkCampaignListInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCampaignRunBody {
    pub input: CodingBenchmarkCampaignRunInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkLeaderboardBody {
    pub input: CodingBenchmarkLeaderboardInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkComparisonBody {
    pub input: CodingBenchmarkComparisonInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTaskPackImportBody {
    pub input: CodingBenchmarkTaskPackImportInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTaskPackListBody {
    pub input: CodingBenchmarkTaskPackListInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTaskPackStatusBody {
    pub input: CodingBenchmarkTaskPackStatusInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTaskPackValidateBody {
    pub input: CodingBenchmarkTaskPackValidateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCorpusHealthBody {
    pub input: CodingBenchmarkCorpusHealthInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReportGenerateBody {
    pub input: CodingBenchmarkReportGenerateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReportListBody {
    pub input: CodingBenchmarkReportListInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReportMarkBody {
    pub input: CodingBenchmarkReportMarkInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinuousBenchmarkGateBody {
    pub input: CodingContinuousBenchmarkGateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkBacklogMaterializeBody {
    pub input: CodingBenchmarkBacklogMaterializeInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkBacklogListBody {
    pub input: CodingBenchmarkBacklogListInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkBacklogStatusBody {
    pub input: CodingBenchmarkBacklogStatusInput,
}

pub async fn get_coding_trend_report(
    Path(session_id): Path<String>,
    Query(query): Query<TrendQuery>,
) -> Result<Json<CodingTrendReport>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.coding_trend_report(&session_id, query.window_days))
            .await?,
    ))
}

pub async fn list_coding_improvement_proposals(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<CodingImprovementProposal>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.list_coding_improvement_proposals(&session_id))
            .await?,
    ))
}

pub async fn generate_coding_improvement_proposals(
    Path(session_id): Path<String>,
    Json(body): Json<GenerateProposalsBody>,
) -> Result<Json<GenerateCodingImprovementProposalsResult>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| {
            db.generate_coding_improvement_proposals_with_input(
                &session_id,
                GenerateCodingImprovementProposalsInput {
                    window_days: body.window_days,
                    source_type: body.source_type,
                    source_id: body.source_id,
                    proposal_kinds: body.proposal_kinds,
                },
            )
        })
        .await?,
    ))
}

pub async fn distill_coding_improvement_proposals(
    Path(session_id): Path<String>,
    Json(body): Json<GenerateProposalsBody>,
) -> Result<Json<DistillCodingImprovementResult>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.distill_coding_improvement_proposals(&session_id, body.window_days))
            .await?,
    ))
}

pub async fn update_coding_improvement_proposal_status(
    Path(proposal_id): Path<String>,
    Json(body): Json<UpdateProposalStatusBody>,
) -> Result<Json<CodingImprovementProposal>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.update_coding_improvement_proposal_status(&proposal_id, &body.status))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn preview_coding_improvement_proposal_action(
    Path(proposal_id): Path<String>,
) -> Result<Json<CodingImprovementActionPlan>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.preview_coding_improvement_proposal_action(&proposal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn apply_coding_improvement_proposal(
    Path(proposal_id): Path<String>,
) -> Result<Json<ApplyCodingImprovementProposalResult>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.apply_coding_improvement_proposal(&proposal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn preview_coding_improvement_proposal_promotion(
    Path(proposal_id): Path<String>,
) -> Result<Json<CodingImprovementPromotionPlan>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.preview_coding_improvement_proposal_promotion(&proposal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn promote_coding_improvement_proposal(
    Path(proposal_id): Path<String>,
) -> Result<Json<PromoteCodingImprovementProposalResult>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.promote_coding_improvement_proposal(&proposal_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn record_coding_eval_run(
    Json(body): Json<RecordEvalRunBody>,
) -> Result<Json<CodingEvalRunRecord>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.record_coding_eval_run(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_coding_eval_release_gate(
    Json(body): Json<ReleaseGateBody>,
) -> Result<Json<CodingEvalReleaseGateReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_coding_eval_release_gate(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_coding_learning_generalization(
    Json(body): Json<LearningGeneralizationBody>,
) -> Result<Json<CodingLearningGeneralizationReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_coding_learning_generalization(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_coding_benchmark_center(
    Json(body): Json<BenchmarkCenterBody>,
) -> Result<Json<CodingBenchmarkCenterReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_coding_benchmark_center(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn create_coding_benchmark_campaign(
    Json(body): Json<BenchmarkCampaignCreateBody>,
) -> Result<Json<CodingBenchmarkCampaign>, AppError> {
    let db = session_db()?;
    let mut input = body.input;
    let run_now = input.run_now;
    let references = input
        .models
        .iter()
        .filter_map(|model| {
            Some((
                model.provider_id.clone()?,
                model.model_id.clone()?,
                model.credential_profile_ref.clone(),
            ))
        })
        .collect::<Vec<_>>();
    let providers = if references.is_empty() {
        std::mem::take(&mut input.gold_task_input.providers)
    } else {
        input.gold_task_input.providers.clear();
        ha_core::evaluation::resolve_owner_provider_refs(&references)
            .map_err(|error| AppError::bad_request(error.to_string()))?
    };
    let campaign = db
        .run(move |db| db.create_coding_benchmark_campaign(input))
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    if run_now {
        let run_db = db.clone();
        let campaign_id = campaign.id.clone();
        tokio::spawn(async move {
            let input = CodingBenchmarkCampaignRunInput {
                campaign_id,
                providers,
                retry_failed_only: false,
            };
            let _ = ha_core::coding_eval::run_benchmark_campaign(run_db, input).await;
        });
    }
    Ok(Json(campaign))
}

pub async fn list_coding_benchmark_campaigns(
    Json(body): Json<BenchmarkCampaignListBody>,
) -> Result<Json<Vec<CodingBenchmarkCampaign>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_coding_benchmark_campaigns(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_coding_benchmark_campaign(
    Path(campaign_id): Path<String>,
) -> Result<Json<Option<CodingBenchmarkCampaign>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_coding_benchmark_campaign(&campaign_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn cancel_coding_benchmark_campaign(
    Path(campaign_id): Path<String>,
) -> Result<Json<Option<CodingBenchmarkCampaign>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.cancel_coding_benchmark_campaign(&campaign_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn run_coding_benchmark_campaign(
    Json(body): Json<BenchmarkCampaignRunBody>,
) -> Result<Json<Option<CodingBenchmarkCampaign>>, AppError> {
    let db = session_db()?;
    let mut input = body.input;
    let campaign_id = input.campaign_id.clone();
    if input.providers.is_empty() {
        let lookup_id = campaign_id.clone();
        let campaign = db
            .run(move |db| db.get_coding_benchmark_campaign(&lookup_id))
            .await
            .map_err(|error| AppError::bad_request(error.to_string()))?
            .ok_or_else(|| AppError::not_found("benchmark campaign not found"))?;
        let references = campaign
            .model_matrix
            .iter()
            .filter_map(|model| Some((model.provider_id.clone()?, model.model_id.clone()?, None)))
            .collect::<Vec<_>>();
        input.providers = ha_core::evaluation::resolve_owner_provider_refs(&references)
            .map_err(|error| AppError::bad_request(error.to_string()))?;
    }
    let spawn_db = db.clone();
    tokio::spawn(async move {
        let _ = ha_core::coding_eval::run_benchmark_campaign(spawn_db, input).await;
    });
    db.run(move |db| db.get_coding_benchmark_campaign(&campaign_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_benchmark_leaderboard(
    Json(body): Json<BenchmarkLeaderboardBody>,
) -> Result<Json<CodingBenchmarkLeaderboardReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_benchmark_leaderboard(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn compare_benchmark_models(
    Json(body): Json<BenchmarkComparisonBody>,
) -> Result<Json<CodingBenchmarkLeaderboardReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.compare_benchmark_models(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn import_benchmark_task_pack(
    Json(body): Json<BenchmarkTaskPackImportBody>,
) -> Result<Json<CodingBenchmarkTaskPack>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.import_benchmark_task_pack(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn list_benchmark_task_packs(
    Json(body): Json<BenchmarkTaskPackListBody>,
) -> Result<Json<Vec<CodingBenchmarkTaskPack>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_benchmark_task_packs(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_benchmark_task_pack(
    Path((pack_id, version)): Path<(String, String)>,
) -> Result<Json<Option<CodingBenchmarkTaskPack>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_benchmark_task_pack(&pack_id, &version))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn update_benchmark_task_pack_status(
    Json(body): Json<BenchmarkTaskPackStatusBody>,
) -> Result<Json<CodingBenchmarkTaskPack>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.update_benchmark_task_pack_status(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn validate_benchmark_task_pack(
    Json(body): Json<BenchmarkTaskPackValidateBody>,
) -> Result<Json<CodingBenchmarkTaskPackValidationReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.validate_benchmark_task_pack(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_benchmark_corpus_health(
    Json(body): Json<BenchmarkCorpusHealthBody>,
) -> Result<Json<CodingBenchmarkCorpusHealthReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_benchmark_corpus_health(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn generate_benchmark_report(
    Json(body): Json<BenchmarkReportGenerateBody>,
) -> Result<Json<CodingBenchmarkReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.generate_benchmark_report(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn list_benchmark_reports(
    Json(body): Json<BenchmarkReportListBody>,
) -> Result<Json<Vec<CodingBenchmarkReport>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_benchmark_reports(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn get_benchmark_report(
    Path(report_id): Path<String>,
) -> Result<Json<Option<CodingBenchmarkReport>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_benchmark_report(&report_id))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn mark_benchmark_report_release_evidence(
    Json(body): Json<BenchmarkReportMarkBody>,
) -> Result<Json<CodingBenchmarkReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.mark_benchmark_report_release_evidence(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn evaluate_continuous_benchmark_gate(
    Json(body): Json<ContinuousBenchmarkGateBody>,
) -> Result<Json<CodingContinuousBenchmarkGateReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.evaluate_continuous_benchmark_gate(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn materialize_benchmark_backlog(
    Json(body): Json<BenchmarkBacklogMaterializeBody>,
) -> Result<Json<CodingBenchmarkBacklogMaterializeResult>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.materialize_benchmark_backlog(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn list_benchmark_backlog(
    Json(body): Json<BenchmarkBacklogListBody>,
) -> Result<Json<Vec<CodingBenchmarkBacklogItem>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_benchmark_backlog(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn update_benchmark_backlog_status(
    Json(body): Json<BenchmarkBacklogStatusBody>,
) -> Result<Json<CodingBenchmarkBacklogItem>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.update_benchmark_backlog_status(body.input))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}
