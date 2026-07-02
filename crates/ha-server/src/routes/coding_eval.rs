use axum::Json;
use ha_core::coding_eval::{self, CodingEvalFixture, FixtureReport};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCodingTaskEvalFixtureBody {
    pub fixture: CodingEvalFixture,
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
