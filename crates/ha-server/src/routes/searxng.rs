//! Docker SearXNG management routes.
//!
//! Thin wrappers around `ha_core::docker::*`. All real work (docker CLI calls,
//! deploy progress tracking, lifecycle) lives in ha-core so these handlers
//! stay under ~15 lines each.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::AppContext;

/// `GET /api/searxng/status` — combined Docker + SearXNG container status.
pub async fn status() -> Result<Json<ha_core::docker::SearxngDockerStatus>, AppError> {
    Ok(Json(ha_core::docker::status().await))
}

/// `POST /api/searxng/deploy` — deploy the SearXNG container, blocking
/// until the deploy completes. Progress is emitted to the shared
/// `EventBus` under [`ha_core::docker::EVENT_SEARXNG_DEPLOY_PROGRESS`];
/// browsers receive the stream via `/ws/events`.
pub async fn deploy(State(ctx): State<Arc<AppContext>>) -> Result<Json<Value>, AppError> {
    let bus = ctx.event_bus.clone();
    let url = ha_core::docker::deploy(move |progress| {
        bus.emit(
            ha_core::docker::EVENT_SEARXNG_DEPLOY_PROGRESS,
            json!(progress),
        );
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "url": url })))
}

/// `POST /api/searxng/start` — start an existing SearXNG container.
pub async fn start() -> Result<Json<Value>, AppError> {
    ha_core::docker::start()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/searxng/stop` — stop a running SearXNG container.
pub async fn stop() -> Result<Json<Value>, AppError> {
    ha_core::docker::stop()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/searxng` — remove the SearXNG container entirely.
pub async fn remove() -> Result<Json<Value>, AppError> {
    ha_core::docker::remove()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}
