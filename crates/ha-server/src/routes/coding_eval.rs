use axum::Json;
use ha_core::coding_eval::{
    self, CodingEvalFixture, FixtureReport, GoldTaskPackReport, GoldTaskPackRunInput,
    GoldTaskPackSummary, StrategyEffectEvalInput, StrategyEffectReport,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCodingTaskEvalFixtureBody {
    pub fixture: CodingEvalFixture,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCodingEvalGoldTaskPackBody {
    pub input: GoldTaskPackRunInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateCodingEvalStrategyEffectBody {
    pub input: StrategyEffectEvalInput,
}

pub async fn run_coding_task_eval_fixture(
    Json(body): Json<RunCodingTaskEvalFixtureBody>,
) -> Result<Json<FixtureReport>, AppError> {
    let db = session_db()?.clone();
    coding_eval::evaluate(db, &body.fixture)
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn list_coding_eval_gold_tasks() -> Result<Json<GoldTaskPackSummary>, AppError> {
    Ok(Json(coding_eval::gold_task_pack_summary()))
}

pub async fn run_coding_eval_gold_task_pack(
    Json(body): Json<RunCodingEvalGoldTaskPackBody>,
) -> Result<Json<GoldTaskPackReport>, AppError> {
    let db = session_db()?.clone();
    coding_eval::run_gold_task_pack(db, body.input)
        .await
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}

pub async fn evaluate_coding_eval_strategy_effect(
    Json(body): Json<EvaluateCodingEvalStrategyEffectBody>,
) -> Result<Json<StrategyEffectReport>, AppError> {
    let db = session_db()?.clone();
    coding_eval::evaluate_strategy_effect_with_recording(&db, body.input)
        .map(Json)
        .map_err(|err| AppError::bad_request(err.to_string()))
}
