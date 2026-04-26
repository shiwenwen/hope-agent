//! Local LLM assistant routes.
//!
//! Long-running install / pull operations now live in the `local_model_jobs`
//! background task system (`/api/local-model-jobs/*`); these routes only
//! expose the cheap one-shot probes used by the GUI to decide what to offer.

use axum::Json;
use serde_json::{json, Value};

use ha_core::local_llm::{detect_hardware, detect_ollama, recommend_model, start_ollama};

use crate::error::AppError;

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

/// `POST /api/local-llm/start` — best-effort `ollama serve` spawn.
pub async fn start() -> Result<Json<Value>, AppError> {
    start_ollama().await?;
    Ok(Json(json!({ "ok": true })))
}
