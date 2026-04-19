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
/// Toggles the persisted `AppConfig.dangerousSkipAllApprovals` field. The CLI
/// flag (the other OR'd source) is process-scoped and unaffected here.
pub async fn set_dangerous_skip_all_approvals(
    Json(body): Json<SetDangerousBody>,
) -> Result<Json<Value>, AppError> {
    let mut store = ha_core::config::load_config()?;
    store.dangerous_skip_all_approvals = body.enabled;
    let _reason = ha_core::backup::scope_save_reason("security", "ui");
    ha_core::config::save_config(&store)?;
    drop(_reason);
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": "security" }),
        );
    }
    Ok(Json(json!({ "saved": true })))
}
