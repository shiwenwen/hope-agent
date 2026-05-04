//! Local model auto-maintainer dismiss / control routes.

use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use ha_core::local_llm::auto_maintainer;

use crate::error::AppError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertModelBody {
    pub model_id: String,
}

/// `POST /api/local-model/alert/dismiss-temporary` — bump 5 min cooldown.
pub async fn dismiss_temporary(Json(body): Json<AlertModelBody>) -> Result<Json<Value>, AppError> {
    auto_maintainer::dismiss_alert_temporary(&body.model_id).await;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-model/alert/silence-session` — process-lifetime silence.
pub async fn silence_session(Json(body): Json<AlertModelBody>) -> Result<Json<Value>, AppError> {
    auto_maintainer::silence_for_session(&body.model_id).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoMaintenanceState {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAutoMaintenanceBody {
    pub enabled: bool,
}

/// `GET /api/local-model/auto-maintenance` — read the current toggle state.
pub async fn get_auto_maintenance() -> Result<Json<AutoMaintenanceState>, AppError> {
    Ok(Json(AutoMaintenanceState {
        enabled: auto_maintainer::get_auto_maintenance_enabled(),
    }))
}

/// `PUT /api/local-model/auto-maintenance` — flip the toggle.
pub async fn set_auto_maintenance(
    Json(body): Json<SetAutoMaintenanceBody>,
) -> Result<Json<Value>, AppError> {
    auto_maintainer::set_auto_maintenance_enabled(body.enabled)?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-model/auto-maintenance/disable` — alert-dialog shortcut.
/// Tagged with a distinct config-mutation source so backup history can
/// distinguish "user flipped settings toggle" from "user clicked Turn off
/// in alert dialog".
pub async fn disable() -> Result<Json<Value>, AppError> {
    auto_maintainer::disable_via_alert_dialog()?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-model/auto-maintenance/trigger` — kick the watchdog
/// into running immediately (used by the frontend after a model swap or
/// redownload completion so the new default gets preloaded right away).
pub async fn trigger() -> Result<Json<Value>, AppError> {
    auto_maintainer::trigger();
    Ok(Json(json!({ "ok": true })))
}
