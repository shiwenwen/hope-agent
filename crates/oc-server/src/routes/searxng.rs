//! Docker SearXNG management routes.
//!
//! Thin wrappers around `oc_core::docker::*`. All real work (docker CLI calls,
//! deploy progress tracking, lifecycle) lives in oc-core so these handlers
//! stay under ~15 lines each.

use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

/// `GET /api/searxng/status` — combined Docker + SearXNG container status.
pub async fn status() -> Result<Json<oc_core::docker::SearxngDockerStatus>, AppError> {
    Ok(Json(oc_core::docker::status().await))
}

/// `POST /api/searxng/deploy` — deploy the SearXNG container, blocking until
/// the deploy completes. Progress messages are dropped on the floor in
/// server mode (the desktop shell forwards them to a `Channel<String>`; the
/// equivalent for HTTP would be a WebSocket which we can add later if the
/// UI needs live deploy logs over the network).
pub async fn deploy() -> Result<Json<Value>, AppError> {
    let url = oc_core::docker::deploy(|_line| {})
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "url": url })))
}

/// `POST /api/searxng/start` — start an existing SearXNG container.
pub async fn start() -> Result<Json<Value>, AppError> {
    oc_core::docker::start()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/searxng/stop` — stop a running SearXNG container.
pub async fn stop() -> Result<Json<Value>, AppError> {
    oc_core::docker::stop()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/searxng` — remove the SearXNG container entirely.
pub async fn remove() -> Result<Json<Value>, AppError> {
    oc_core::docker::remove()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}
