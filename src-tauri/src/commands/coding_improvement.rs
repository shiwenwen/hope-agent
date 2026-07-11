use crate::commands::CmdError;
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

#[tauri::command]
pub async fn get_coding_trend_report(
    session_id: String,
    window_days: Option<u32>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingTrendReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.coding_trend_report(&session_id, window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_coding_improvement_proposals(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CodingImprovementProposal>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_coding_improvement_proposals(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn generate_coding_improvement_proposals(
    session_id: String,
    window_days: Option<u32>,
    source_type: Option<String>,
    source_id: Option<String>,
    proposal_kinds: Option<Vec<String>>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GenerateCodingImprovementProposalsResult, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| {
        db.generate_coding_improvement_proposals_with_input(
            &session_id,
            GenerateCodingImprovementProposalsInput {
                window_days,
                source_type,
                source_id,
                proposal_kinds: proposal_kinds.unwrap_or_default(),
            },
        )
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn distill_coding_improvement_proposals(
    session_id: String,
    window_days: Option<u32>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DistillCodingImprovementResult, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.distill_coding_improvement_proposals(&session_id, window_days))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_coding_improvement_proposal_status(
    proposal_id: String,
    status: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingImprovementProposal, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.update_coding_improvement_proposal_status(&proposal_id, &status))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_coding_improvement_proposal_action(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingImprovementActionPlan, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.preview_coding_improvement_proposal_action(&proposal_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_coding_improvement_proposal(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ApplyCodingImprovementProposalResult, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.apply_coding_improvement_proposal(&proposal_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_coding_improvement_proposal_promotion(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingImprovementPromotionPlan, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.preview_coding_improvement_proposal_promotion(&proposal_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn promote_coding_improvement_proposal(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<PromoteCodingImprovementProposalResult, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.promote_coding_improvement_proposal(&proposal_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn record_coding_eval_run(
    input: RecordCodingEvalRunInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingEvalRunRecord, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.record_coding_eval_run(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_coding_eval_release_gate(
    input: CodingEvalReleaseGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingEvalReleaseGateReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_coding_eval_release_gate(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_coding_learning_generalization(
    input: CodingLearningGeneralizationInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingLearningGeneralizationReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_coding_learning_generalization(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_coding_benchmark_center(
    input: CodingBenchmarkCenterInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkCenterReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_coding_benchmark_center(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_coding_benchmark_campaign(
    input: CodingBenchmarkCampaignCreateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkCampaign, CmdError> {
    let run_now = input.run_now;
    let providers = input.gold_task_input.providers.clone();
    let db = app_state.session_db.clone();
    let campaign = db
        .run(move |db| db.create_coding_benchmark_campaign(input))
        .await
        .map_err(CmdError::from)?;
    if run_now {
        let db = app_state.session_db.clone();
        let campaign_id = campaign.id.clone();
        tokio::spawn(async move {
            let input = CodingBenchmarkCampaignRunInput {
                campaign_id,
                providers,
                retry_failed_only: false,
            };
            let _ = ha_core::coding_eval::run_benchmark_campaign(db, input).await;
        });
    }
    Ok(campaign)
}

#[tauri::command]
pub async fn list_coding_benchmark_campaigns(
    input: CodingBenchmarkCampaignListInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CodingBenchmarkCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_coding_benchmark_campaigns(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_coding_benchmark_campaign(
    campaign_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<CodingBenchmarkCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_coding_benchmark_campaign(&campaign_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_coding_benchmark_campaign(
    campaign_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<CodingBenchmarkCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.cancel_coding_benchmark_campaign(&campaign_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_coding_benchmark_campaign(
    input: CodingBenchmarkCampaignRunInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<CodingBenchmarkCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    let campaign_id = input.campaign_id.clone();
    tokio::spawn(async move {
        let _ = ha_core::coding_eval::run_benchmark_campaign(db, input).await;
    });
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_coding_benchmark_campaign(&campaign_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_benchmark_leaderboard(
    input: CodingBenchmarkLeaderboardInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkLeaderboardReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_benchmark_leaderboard(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn compare_benchmark_models(
    input: CodingBenchmarkComparisonInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkLeaderboardReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.compare_benchmark_models(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn import_benchmark_task_pack(
    input: CodingBenchmarkTaskPackImportInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkTaskPack, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.import_benchmark_task_pack(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_benchmark_task_packs(
    input: CodingBenchmarkTaskPackListInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CodingBenchmarkTaskPack>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_benchmark_task_packs(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_benchmark_task_pack(
    pack_id: String,
    version: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<CodingBenchmarkTaskPack>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_benchmark_task_pack(&pack_id, &version))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_benchmark_task_pack_status(
    input: CodingBenchmarkTaskPackStatusInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkTaskPack, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.update_benchmark_task_pack_status(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn validate_benchmark_task_pack(
    input: CodingBenchmarkTaskPackValidateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkTaskPackValidationReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.validate_benchmark_task_pack(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_benchmark_corpus_health(
    input: CodingBenchmarkCorpusHealthInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkCorpusHealthReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_benchmark_corpus_health(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn generate_benchmark_report(
    input: CodingBenchmarkReportGenerateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.generate_benchmark_report(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_benchmark_reports(
    input: CodingBenchmarkReportListInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CodingBenchmarkReport>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_benchmark_reports(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_benchmark_report(
    report_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<CodingBenchmarkReport>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_benchmark_report(&report_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mark_benchmark_report_release_evidence(
    input: CodingBenchmarkReportMarkInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.mark_benchmark_report_release_evidence(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_continuous_benchmark_gate(
    input: CodingContinuousBenchmarkGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingContinuousBenchmarkGateReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_continuous_benchmark_gate(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn materialize_benchmark_backlog(
    input: CodingBenchmarkBacklogMaterializeInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkBacklogMaterializeResult, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.materialize_benchmark_backlog(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_benchmark_backlog(
    input: CodingBenchmarkBacklogListInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CodingBenchmarkBacklogItem>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_benchmark_backlog(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_benchmark_backlog_status(
    input: CodingBenchmarkBacklogStatusInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingBenchmarkBacklogItem, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.update_benchmark_backlog_status(input))
        .await
        .map_err(Into::into)
}
