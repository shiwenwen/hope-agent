use axum::Json;
use ha_core::domain_eval::{
    DomainEvalCalibrationRecord, DomainEvalFixtureReport, DomainEvalRunRecord, DomainEvalTask,
    DomainQualityGateInput, DomainQualityGateReport, ImportDomainEvalCaseInput,
    ImportDomainEvalCaseResult, ListDomainEvalCalibrationsInput, ListDomainEvalRunsInput,
    ListDomainEvalTasksInput, RecordDomainEvalCalibrationInput, RunDomainEvalFixtureInput,
    RunDomainEvalTaskInput,
};
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
pub struct DomainQualityGateBody {
    pub input: DomainQualityGateInput,
}

pub async fn list_domain_eval_tasks(
    Json(body): Json<ListDomainEvalTasksBody>,
) -> Result<Json<Vec<DomainEvalTask>>, AppError> {
    Ok(Json(session_db()?.list_domain_eval_tasks(body.input)?))
}

pub async fn run_domain_eval_task(
    Json(body): Json<RunDomainEvalTaskBody>,
) -> Result<Json<DomainEvalRunRecord>, AppError> {
    Ok(Json(session_db()?.run_domain_eval_task(body.input)?))
}

pub async fn run_domain_eval_fixture(
    Json(body): Json<RunDomainEvalFixtureBody>,
) -> Result<Json<DomainEvalFixtureReport>, AppError> {
    Ok(Json(session_db()?.run_domain_eval_fixture(body.input)?))
}

pub async fn import_domain_eval_case(
    Json(body): Json<ImportDomainEvalCaseBody>,
) -> Result<Json<ImportDomainEvalCaseResult>, AppError> {
    Ok(Json(session_db()?.import_domain_eval_case(body.input)?))
}

pub async fn record_domain_eval_calibration(
    Json(body): Json<RecordDomainEvalCalibrationBody>,
) -> Result<Json<DomainEvalCalibrationRecord>, AppError> {
    Ok(Json(
        session_db()?.record_domain_eval_calibration(body.input)?,
    ))
}

pub async fn list_domain_eval_calibrations(
    Json(body): Json<ListDomainEvalCalibrationsBody>,
) -> Result<Json<Vec<DomainEvalCalibrationRecord>>, AppError> {
    Ok(Json(
        session_db()?.list_domain_eval_calibrations(body.input)?,
    ))
}

pub async fn list_domain_eval_runs(
    Json(body): Json<ListDomainEvalRunsBody>,
) -> Result<Json<Vec<DomainEvalRunRecord>>, AppError> {
    Ok(Json(session_db()?.list_domain_eval_runs(body.input)?))
}

pub async fn evaluate_domain_quality_gate(
    Json(body): Json<DomainQualityGateBody>,
) -> Result<Json<DomainQualityGateReport>, AppError> {
    Ok(Json(
        session_db()?.evaluate_domain_quality_gate(body.input)?,
    ))
}
