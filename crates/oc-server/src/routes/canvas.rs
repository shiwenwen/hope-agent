use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct SnapshotBody {
    pub data_url: Option<String>,
    pub error: Option<String>,
}

/// `POST /api/canvas/snapshot/{request_id}`
pub async fn canvas_submit_snapshot(
    Path(request_id): Path<String>,
    Json(body): Json<SnapshotBody>,
) -> Result<Json<Value>, AppError> {
    oc_core::tools::canvas::canvas_submit_snapshot(request_id, body.data_url, body.error)
        .await
        .map_err(|e| AppError::internal(e))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct EvalBody {
    pub result: Option<String>,
    pub error: Option<String>,
}

/// `POST /api/canvas/eval/{request_id}`
pub async fn canvas_submit_eval_result(
    Path(request_id): Path<String>,
    Json(body): Json<EvalBody>,
) -> Result<Json<Value>, AppError> {
    oc_core::tools::canvas::canvas_submit_eval_result(request_id, body.result, body.error)
        .await
        .map_err(|e| AppError::internal(e))?;
    Ok(Json(json!({ "ok": true })))
}
