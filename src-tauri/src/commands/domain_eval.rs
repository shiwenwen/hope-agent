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
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_eval_tasks(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn run_domain_eval_task(
    input: RunDomainEvalTaskInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalRunRecord, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.run_domain_eval_task(input))
        .await
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.import_domain_eval_case(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn record_domain_eval_calibration(
    input: RecordDomainEvalCalibrationInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalCalibrationRecord, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.record_domain_eval_calibration(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_eval_calibrations(
    input: ListDomainEvalCalibrationsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalCalibrationRecord>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_eval_calibrations(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_eval_runs(
    input: ListDomainEvalRunsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalRunRecord>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_eval_runs(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_domain_eval_fixture_runs(
    input: ListDomainEvalFixtureRunsInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DomainEvalFixtureRunRecord>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_eval_fixture_runs(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_domain_eval_campaign(
    input: CreateDomainEvalCampaignInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalCampaign, CmdError> {
    let run_now = input.run_now;
    let providers = input.providers.clone();
    let db = app_state.session_db.clone();
    let campaign = db
        .run(move |db| db.create_domain_eval_campaign(input))
        .await
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_domain_eval_campaigns(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_domain_eval_campaign(
    campaign_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<DomainEvalCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_domain_eval_campaign(&campaign_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_domain_eval_campaign(
    campaign_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<DomainEvalCampaign>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.cancel_domain_eval_campaign(&campaign_id))
        .await
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
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_domain_eval_campaign(&campaign_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_domain_eval_campaign_leaderboard(
    input: DomainEvalCampaignLeaderboardInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainEvalCampaignLeaderboardReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_domain_eval_campaign_leaderboard(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_quality_gate(
    input: DomainQualityGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainQualityGateReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_domain_quality_gate(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_readiness_gate(
    input: DomainReadinessGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainReadinessGateReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_domain_readiness_gate(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn evaluate_domain_operational_gate(
    input: DomainOperationalGateInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainOperationalGateReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.evaluate_domain_operational_gate(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn generate_domain_soak_report(
    input: DomainSoakReportInput,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<DomainSoakReport, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.generate_domain_soak_report(input))
        .await
        .map_err(Into::into)
}
