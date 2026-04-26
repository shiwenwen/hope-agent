use axum::Json;
use serde::Deserialize;

use crate::error::AppError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelRuntimeTaskBody {
    pub kind: ha_core::runtime_tasks::RuntimeTaskKind,
    pub id: String,
}

pub async fn cancel_runtime_task(
    Json(body): Json<CancelRuntimeTaskBody>,
) -> Result<Json<ha_core::runtime_tasks::CancelRuntimeTaskResult>, AppError> {
    Ok(Json(
        ha_core::runtime_tasks::cancel_runtime_task(body.kind, &body.id).await?,
    ))
}
