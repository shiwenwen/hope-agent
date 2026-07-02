use crate::commands::CmdError;
use ha_core::coding_improvement::{
    ApplyCodingImprovementProposalResult, CodingEvalRunRecord, CodingImprovementActionPlan,
    CodingImprovementPromotionPlan, CodingImprovementProposal, CodingTrendReport,
    GenerateCodingImprovementProposalsResult, PromoteCodingImprovementProposalResult,
    RecordCodingEvalRunInput,
};

#[tauri::command]
pub async fn get_coding_trend_report(
    session_id: String,
    window_days: Option<u32>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingTrendReport, CmdError> {
    app_state
        .session_db
        .coding_trend_report(&session_id, window_days)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_coding_improvement_proposals(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CodingImprovementProposal>, CmdError> {
    app_state
        .session_db
        .list_coding_improvement_proposals(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn generate_coding_improvement_proposals(
    session_id: String,
    window_days: Option<u32>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<GenerateCodingImprovementProposalsResult, CmdError> {
    app_state
        .session_db
        .generate_coding_improvement_proposals(&session_id, window_days)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_coding_improvement_proposal_status(
    proposal_id: String,
    status: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingImprovementProposal, CmdError> {
    app_state
        .session_db
        .update_coding_improvement_proposal_status(&proposal_id, &status)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_coding_improvement_proposal_action(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingImprovementActionPlan, CmdError> {
    app_state
        .session_db
        .preview_coding_improvement_proposal_action(&proposal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_coding_improvement_proposal(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ApplyCodingImprovementProposalResult, CmdError> {
    app_state
        .session_db
        .apply_coding_improvement_proposal(&proposal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_coding_improvement_proposal_promotion(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingImprovementPromotionPlan, CmdError> {
    app_state
        .session_db
        .preview_coding_improvement_proposal_promotion(&proposal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn promote_coding_improvement_proposal(
    proposal_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<PromoteCodingImprovementProposalResult, CmdError> {
    app_state
        .session_db
        .promote_coding_improvement_proposal(&proposal_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn record_coding_eval_run(
    input: RecordCodingEvalRunInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<CodingEvalRunRecord, CmdError> {
    app_state
        .session_db
        .record_coding_eval_run(input)
        .map_err(Into::into)
}
