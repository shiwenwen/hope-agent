//! Developer reset routes.
//!
//! Thin wrappers over `ha_core::dev_tools::*` so that the settings panel's
//! "reset config / clear sessions / clear memory / clear all" buttons work
//! in HTTP mode (remote / headless install). All underlying operations
//! mutate `~/.hope-agent/` files and are reachable from anywhere the
//! server has filesystem access.

use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

async fn wrap<F>(fut: F) -> Result<Json<Value>, AppError>
where
    F: std::future::Future<Output = Result<(), String>>,
{
    fut.await.map_err(AppError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/dev/clear-sessions` — wipe the sessions SQLite DB.
pub async fn clear_sessions() -> Result<Json<Value>, AppError> {
    wrap(ha_core::dev_tools::dev_clear_sessions()).await
}

/// `POST /api/dev/clear-cron` — wipe the cron SQLite DB.
pub async fn clear_cron() -> Result<Json<Value>, AppError> {
    wrap(ha_core::dev_tools::dev_clear_cron()).await
}

/// `POST /api/dev/clear-memory` — wipe the memory SQLite DB.
pub async fn clear_memory() -> Result<Json<Value>, AppError> {
    wrap(ha_core::dev_tools::dev_clear_memory()).await
}

/// `POST /api/dev/reset-config` — reset `config.json` to defaults.
pub async fn reset_config() -> Result<Json<Value>, AppError> {
    wrap(ha_core::dev_tools::dev_reset_config()).await
}

/// `POST /api/dev/clear-all` — wipe every Hope Agent data file.
pub async fn clear_all() -> Result<Json<Value>, AppError> {
    wrap(ha_core::dev_tools::dev_clear_all()).await
}
