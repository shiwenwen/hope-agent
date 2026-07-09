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

/// `GET /api/security/dangerous-status`
///
/// Returns whether Dangerous Mode is active and which source(s) activated it.
/// Consumed by the frontend for the persistent banner and Settings toggle.
pub async fn dangerous_mode_status() -> Json<ha_core::security::dangerous::DangerousModeStatus> {
    Json(ha_core::security::dangerous::status())
}

#[derive(Debug, Deserialize)]
pub struct SetDangerousBody {
    pub enabled: bool,
}

/// `POST /api/security/dangerous-skip-all-approvals`
///
/// Toggles the persisted `AppConfig.permission.global_yolo` field. The CLI
/// flag (the other OR'd source) is process-scoped and unaffected here.
pub async fn set_dangerous_skip_all_approvals(
    Json(body): Json<SetDangerousBody>,
) -> Result<Json<Value>, AppError> {
    // mutate_config_async handles the save-reason scoping + `config:changed`
    // emit internally (the manual scope_save_reason / bus.emit dance this
    // replaced is exactly what the config contract centralizes).
    ha_core::config::mutate_config_async(("security", "ui"), move |store| {
        store.permission.global_yolo = body.enabled;
        Ok(())
    })
    .await?;
    Ok(Json(json!({ "saved": true })))
}
