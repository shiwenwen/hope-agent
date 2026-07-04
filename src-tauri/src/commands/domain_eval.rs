use crate::commands::CmdError;
use ha_core::domain_eval::{
    CreateDomainEvalCampaignInput, DomainEvalCalibrationRecord, DomainEvalCampaign,
    DomainEvalCampaignLeaderboardInput, DomainEvalCampaignLeaderboardReport,
    DomainEvalFixtureReport, DomainEvalFixtureRunRecord, DomainEvalRunRecord, DomainEvalTask,
    DomainOperationalGateInput, DomainOperationalGateReport, DomainQualityGateInput,
    DomainQualityGateReport, DomainReadinessGateInput, DomainReadinessGateReport, DomainSoakReport,
    DomainSoakReportInput, ImportDomainEvalCaseInput, ImportDomainEvalCaseResult,
    ListDomainEvalCalibrationsInput, ListDomainEvalCampaignsInput, ListDomainEvalFixtureRunsInput,
    ListDomainEvalRunsInput, ListDomainEvalTasksInput, RecordDomainEvalCalibrationInput,
    RunDomainEvalCampaignInput, RunDomainEvalFixtureInput, RunDomainEvalTaskInput,
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
) -> Result<Vec<DomainEvalFixtureRunRecord>, CmdError> {
    app_state
        .session_db
        .list_domain_eval_fixture_runs(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_domain_eval_campaign(
    input: CreateDomainEvalCampaignInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalCampaign, CmdError> {
    let run_now = input.run_now;
    let providers = input.providers.clone();
    let campaign = app_state
        .session_db
        .create_domain_eval_campaign(input)
        .map_err(CmdError::from)?;
    if run_now {
        let db = app_state.session_db.clone();
        let campaign_id = campaign.id.clone();
        tokio::spawn(async move {
            let input = RunDomainEvalCampaignInput {
                campaign_id,
                providers,
                retry_failed_only: false,
            };
            let _ = ha_core::domain_eval::run_domain_eval_campaign(db, input).await;
        });
    }
    Ok(campaign)
}

#[tauri::command]
pub async fn list_domain_eval_campaigns(
    input: ListDomainEvalCampaignsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalCampaign>, CmdError> {
    app_state
        .session_db
        .list_domain_eval_campaigns(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_domain_eval_campaign(
    campaign_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<DomainEvalCampaign>, CmdError> {
    app_state
        .session_db
        .get_domain_eval_campaign(&campaign_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_domain_eval_campaign(
    campaign_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<DomainEvalCampaign>, CmdError> {
    app_state
        .session_db
        .cancel_domain_eval_campaign(&campaign_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_domain_eval_campaign(
    input: RunDomainEvalCampaignInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<DomainEvalCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    let campaign_id = input.campaign_id.clone();
    tokio::spawn(async move {
        let _ = ha_core::domain_eval::run_domain_eval_campaign(db, input).await;
    });
    app_state
        .session_db
        .get_domain_eval_campaign(&campaign_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_domain_eval_campaign_leaderboard(
    input: DomainEvalCampaignLeaderboardInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalCampaignLeaderboardReport, CmdError> {
    app_state
        .session_db
        .get_domain_eval_campaign_leaderboard(input)
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

#[tauri::command]
pub async fn evaluate_domain_readiness_gate(
    input: DomainReadinessGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainReadinessGateReport, CmdError> {
    app_state
        .session_db
        .evaluate_domain_readiness_gate(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_operational_gate(
    input: DomainOperationalGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainOperationalGateReport, CmdError> {
    app_state
        .session_db
        .evaluate_domain_operational_gate(input)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn generate_domain_soak_report(
    input: DomainSoakReportInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainSoakReport, CmdError> {
    app_state
        .session_db
        .generate_domain_soak_report(input)
        .map_err(Into::into)
}
