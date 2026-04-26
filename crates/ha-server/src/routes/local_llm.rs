//! Local LLM assistant routes.
//!
//! Long-running operations (`install`, `pull`) emit progress through the
//! shared event bus on the local-LLM progress channels. Browsers subscribe
//! via the `/api/ws/events` WebSocket.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::event_bus::EventBusProgressExt;
use ha_core::local_llm::{
    detect_hardware, detect_ollama, install_ollama_via_script, pull_and_activate, recommend_model,
    start_ollama, ModelCandidate, EVENT_LOCAL_LLM_INSTALL_PROGRESS, EVENT_LOCAL_LLM_PULL_PROGRESS,
};
use ha_core::provider::known_local_backends;

use crate::error::AppError;
use crate::AppContext;

#[derive(Debug, Deserialize)]
pub struct PullBody {
    pub model: ModelCandidate,
}

/// `GET /api/local-llm/hardware` — current memory + GPU snapshot.
pub async fn get_hardware() -> Json<Value> {
    Json(json!(detect_hardware()))
}

/// `GET /api/local-llm/recommendation` — best model + alternatives.
pub async fn get_recommendation() -> Json<Value> {
    Json(json!(recommend_model(&detect_hardware())))
}

/// `GET /api/local-llm/ollama-status` — installed / running probe.
pub async fn get_ollama_status() -> Json<Value> {
    Json(json!(detect_ollama().await))
}

/// `GET /api/local-llm/known-backends` — static local backend catalog.
pub async fn get_known_backends() -> Json<Value> {
    Json(json!(known_local_backends()))
}

/// `POST /api/local-llm/install` — run the bundled installer (Unix only).
/// Streams progress to local-LLM install progress events.
pub async fn install_ollama(State(ctx): State<Arc<AppContext>>) -> Result<Json<Value>, AppError> {
    install_ollama_via_script(
        ctx.event_bus
            .emit_progress(EVENT_LOCAL_LLM_INSTALL_PROGRESS),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-llm/start` — best-effort `ollama serve` spawn.
pub async fn start() -> Result<Json<Value>, AppError> {
    start_ollama().await?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-llm/pull` — pull `model.id`, register Ollama provider,
/// switch active model. Streams progress to local-LLM pull progress events.
pub async fn pull(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<PullBody>,
) -> Result<Json<Value>, AppError> {
    let (provider_id, model_id) = pull_and_activate(
        body.model,
        ctx.event_bus.emit_progress(EVENT_LOCAL_LLM_PULL_PROGRESS),
    )
    .await?;
    Ok(Json(
        json!({ "providerId": provider_id, "modelId": model_id }),
    ))
}
