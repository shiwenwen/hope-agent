use axum::Json;
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
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalTasksBody {
    pub input: ListDomainEvalTasksInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainEvalTaskBody {
    pub input: RunDomainEvalTaskInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainEvalFixtureBody {
    pub input: RunDomainEvalFixtureInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDomainEvalCaseBody {
    pub input: ImportDomainEvalCaseInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDomainEvalCalibrationBody {
    pub input: RecordDomainEvalCalibrationInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalCalibrationsBody {
    pub input: ListDomainEvalCalibrationsInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalRunsBody {
    pub input: ListDomainEvalRunsInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalFixtureRunsBody {
    pub input: ListDomainEvalFixtureRunsInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDomainEvalCampaignBody {
    pub input: CreateDomainEvalCampaignInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalCampaignsBody {
    pub input: ListDomainEvalCampaignsInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainEvalCampaignBody {
    pub input: RunDomainEvalCampaignInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignLeaderboardBody {
    pub input: DomainEvalCampaignLeaderboardInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityGateBody {
    pub input: DomainQualityGateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReadinessGateBody {
    pub input: DomainReadinessGateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationalGateBody {
    pub input: DomainOperationalGateInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSoakReportBody {
    pub input: DomainSoakReportInput,
}

pub async fn list_domain_eval_tasks(
    Json(body): Json<ListDomainEvalTasksBody>,
) -> Result<Json<Vec<DomainEvalTask>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.list_domain_eval_tasks(body.input))
            .await?,
    ))
}

pub async fn run_domain_eval_task(
    Json(body): Json<RunDomainEvalTaskBody>,
) -> Result<Json<DomainEvalRunRecord>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.run_domain_eval_task(body.input))
            .await?,
    ))
}

pub async fn run_domain_eval_fixture(
    Json(body): Json<RunDomainEvalFixtureBody>,
) -> Result<Json<DomainEvalFixtureReport>, AppError> {
    let db = session_db()?.clone();
    SessionDB::run_domain_eval_fixture(db, body.input)
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn import_domain_eval_case(
    Json(body): Json<ImportDomainEvalCaseBody>,
) -> Result<Json<ImportDomainEvalCaseResult>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.import_domain_eval_case(body.input))
            .await?,
    ))
}

pub async fn record_domain_eval_calibration(
    Json(body): Json<RecordDomainEvalCalibrationBody>,
) -> Result<Json<DomainEvalCalibrationRecord>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.record_domain_eval_calibration(body.input))
            .await?,
    ))
}

pub async fn list_domain_eval_calibrations(
    Json(body): Json<ListDomainEvalCalibrationsBody>,
) -> Result<Json<Vec<DomainEvalCalibrationRecord>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.list_domain_eval_calibrations(body.input))
            .await?,
    ))
}

pub async fn list_domain_eval_runs(
    Json(body): Json<ListDomainEvalRunsBody>,
) -> Result<Json<Vec<DomainEvalRunRecord>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.list_domain_eval_runs(body.input))
            .await?,
    ))
}

pub async fn list_domain_eval_fixture_runs(
    Json(body): Json<ListDomainEvalFixtureRunsBody>,
) -> Result<Json<Vec<DomainEvalFixtureRunRecord>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.list_domain_eval_fixture_runs(body.input))
            .await?,
    ))
}

pub async fn create_domain_eval_campaign(
    Json(body): Json<CreateDomainEvalCampaignBody>,
) -> Result<Json<DomainEvalCampaign>, AppError> {
    let db = session_db()?;
    let run_now = body.input.run_now;
    let providers = body.input.providers.clone();
    let campaign = db
        .run(move |db| db.create_domain_eval_campaign(body.input))
        .await
        .map_err(|err| AppError::bad_request(err.to_string()))?;
    if run_now {
        let run_db = db.clone();
        let campaign_id = campaign.id.clone();
        tokio::spawn(async move {
            let input = RunDomainEvalCampaignInput {
                campaign_id,
                providers,
                retry_failed_only: false,
            };
            let _ = ha_core::domain_eval::run_domain_eval_campaign(run_db, input).await;
        });
    }
    Ok(Json(campaign))
}

pub async fn list_domain_eval_campaigns(
    Json(body): Json<ListDomainEvalCampaignsBody>,
) -> Result<Json<Vec<DomainEvalCampaign>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.list_domain_eval_campaigns(body.input))
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn get_domain_eval_campaign(
    axum::extract::Path(campaign_id): axum::extract::Path<String>,
) -> Result<Json<Option<DomainEvalCampaign>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_domain_eval_campaign(&campaign_id))
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn cancel_domain_eval_campaign(
    axum::extract::Path(campaign_id): axum::extract::Path<String>,
) -> Result<Json<Option<DomainEvalCampaign>>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.cancel_domain_eval_campaign(&campaign_id))
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn run_domain_eval_campaign(
    Json(body): Json<RunDomainEvalCampaignBody>,
) -> Result<Json<Option<DomainEvalCampaign>>, AppError> {
    let db = session_db()?;
    let campaign_id = body.input.campaign_id.clone();
    let spawn_db = db.clone();
    tokio::spawn(async move {
        let _ = ha_core::domain_eval::run_domain_eval_campaign(spawn_db, body.input).await;
    });
    db.run(move |db| db.get_domain_eval_campaign(&campaign_id))
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn get_domain_eval_campaign_leaderboard(
    Json(body): Json<DomainEvalCampaignLeaderboardBody>,
) -> Result<Json<DomainEvalCampaignLeaderboardReport>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.get_domain_eval_campaign_leaderboard(body.input))
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn evaluate_domain_quality_gate(
    Json(body): Json<DomainQualityGateBody>,
) -> Result<Json<DomainQualityGateReport>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.evaluate_domain_quality_gate(body.input))
            .await?,
    ))
}

pub async fn evaluate_domain_readiness_gate(
    Json(body): Json<DomainReadinessGateBody>,
) -> Result<Json<DomainReadinessGateReport>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.evaluate_domain_readiness_gate(body.input))
            .await?,
    ))
}

pub async fn evaluate_domain_operational_gate(
    Json(body): Json<DomainOperationalGateBody>,
) -> Result<Json<DomainOperationalGateReport>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.evaluate_domain_operational_gate(body.input))
            .await?,
    ))
}

pub async fn generate_domain_soak_report(
    Json(body): Json<DomainSoakReportBody>,
) -> Result<Json<DomainSoakReport>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.generate_domain_soak_report(body.input))
            .await?,
    ))
}
