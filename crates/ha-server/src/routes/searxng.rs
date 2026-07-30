//! Docker SearXNG management routes.
//!
//! Thin wrappers around `ha_vcs::docker::*`. All real work (docker CLI calls,
//! deploy progress tracking, lifecycle) lives in ha-vcs so these handlers
//! stay under ~15 lines each.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use ha_core::event_bus::EventBusProgressExt;

use crate::error::AppError;
use crate::AppContext;

/// `GET /api/searxng/status` — combined Docker + SearXNG container status.
pub async fn status() -> Result<Json<ha_vcs::docker::SearxngDockerStatus>, AppError> {
    Ok(Json(ha_vcs::docker::status().await))
}

/// `POST /api/searxng/deploy` — deploy the SearXNG container, blocking
/// until the deploy completes. Progress is emitted to the shared
/// `EventBus` under [`ha_vcs::docker::EVENT_SEARXNG_DEPLOY_PROGRESS`];
/// browsers receive the stream via `/ws/events`.
pub async fn deploy(State(ctx): State<Arc<AppContext>>) -> Result<Json<Value>, AppError> {
    let url = ha_vcs::docker::deploy(
        ctx.event_bus
            .emit_progress(ha_vcs::docker::EVENT_SEARXNG_DEPLOY_PROGRESS),
    )
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "url": url })))
}

/// `POST /api/searxng/start` — start an existing SearXNG container.
pub async fn start() -> Result<Json<Value>, AppError> {
    ha_vcs::docker::start()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/searxng/stop` — stop a running SearXNG container.
pub async fn stop() -> Result<Json<Value>, AppError> {
    ha_vcs::docker::stop()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/searxng` — remove the SearXNG container entirely.
pub async fn remove() -> Result<Json<Value>, AppError> {
    ha_vcs::docker::remove()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}
