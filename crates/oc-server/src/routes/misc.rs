use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct WriteExportBody {
    pub path: String,
    pub content: String,
}

/// `POST /api/misc/write-export-file`
pub async fn write_export_file(Json(body): Json<WriteExportBody>) -> Result<Json<Value>, AppError> {
    std::fs::write(&body.path, body.content)
        .map_err(|e| AppError::internal(format!("Failed to write export file: {}", e)))?;
    Ok(Json(json!({ "ok": true })))
}
