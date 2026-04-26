//! Local LLM assistant routes.
//!
//! Long-running operations (`install`, `pull`) emit progress through the
//! shared event bus on the `local_llm:install_progress` /
//! `local_llm:pull_progress` channels. Browsers subscribe via the
//! `/api/ws/events` WebSocket.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::local_llm::{
    detect_hardware, detect_ollama, install_ollama_via_script, pull_and_activate, recommend_model,
    start_ollama, ModelCandidate,
};

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

/// `POST /api/local-llm/install` — run the bundled installer (Unix only).
/// Streams progress to `local_llm:install_progress` events.
pub async fn install_ollama(State(ctx): State<Arc<AppContext>>) -> Result<Json<Value>, AppError> {
    let bus = ctx.event_bus.clone();
    install_ollama_via_script(move |p| {
        bus.emit("local_llm:install_progress", json!(p));
    })
    .await?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-llm/start` — best-effort `ollama serve` spawn.
pub async fn start() -> Result<Json<Value>, AppError> {
    start_ollama().await?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/local-llm/pull` — pull `model.id`, register Ollama provider,
/// switch active model. Streams progress to `local_llm:pull_progress`.
pub async fn pull(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<PullBody>,
) -> Result<Json<Value>, AppError> {
    let bus = ctx.event_bus.clone();
    let (provider_id, model_id) = pull_and_activate(body.model, move |p| {
        bus.emit("local_llm:pull_progress", json!(p));
    })
    .await?;
    Ok(Json(
        json!({ "providerId": provider_id, "modelId": model_id }),
    ))
}
