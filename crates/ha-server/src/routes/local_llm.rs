//! Local LLM assistant routes.
//!
//! Long-running install / pull operations now live in the `local_model_jobs`
//! background task system (`/api/local-model-jobs/*`); these routes only
//! expose the cheap one-shot probes used by the GUI to decide what to offer.

use axum::extract::Query;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::local_llm::{
    add_ollama_model_as_embedding_config, delete_ollama_model, detect_hardware, detect_ollama,
    get_ollama_library_model, list_local_ollama_models, model_catalog, preload_ollama_model,
    recommend_model, register_ollama_model_as_provider, search_ollama_library, start_ollama,
    stop_ollama_model,
};
use ha_core::provider::known_local_backends;

use crate::error::AppError;

/// `GET /api/local-llm/hardware` — current memory + GPU snapshot.
pub async fn get_hardware() -> Json<Value> {
    Json(json!(detect_hardware()))
}

/// `GET /api/local-llm/recommendation` — best model + alternatives.
pub async fn get_recommendation() -> Json<Value> {
    Json(json!(recommend_model(&detect_hardware())))
}

/// `GET /api/local-llm/chat-catalog` — full chat-model catalog regardless
/// of hardware budget. Used by `MissingModelDialog` to find redownload
/// candidates that exceed the current recommended budget.
pub async fn get_chat_catalog() -> Json<Value> {
    Json(json!(model_catalog()))
}

/// `GET /api/local-llm/ollama-status` — installed / running probe.
pub async fn get_ollama_status() -> Json<Value> {
    Json(json!(detect_ollama().await))
}

/// `GET /api/local-llm/known-backends` — static local backend catalog.
pub async fn get_known_backends() -> Json<Value> {
    Json(json!(known_local_backends()))
}

/// `POST /api/local-llm/start` — best-effort `ollama serve` spawn.
pub async fn start() -> Result<Json<Value>, AppError> {
    start_ollama().await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySearchQuery {
    pub query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelIdBody {
    pub model_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryModelBody {
    pub model: String,
}

/// `GET /api/local-llm/models` — installed Ollama models with Hope usage state.
pub async fn list_models() -> Result<Json<Value>, AppError> {
    Ok(Json(json!(list_local_ollama_models().await?)))
}

/// `GET /api/local-llm/library/search?query=...`
pub async fn search_library(Query(q): Query<LibrarySearchQuery>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        search_ollama_library(q.query.as_deref().unwrap_or("")).await?
    )))
}

/// `POST /api/local-llm/library/model` — fetch one Ollama Library family.
pub async fn get_library_model(
    Json(body): Json<LibraryModelBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(get_ollama_library_model(&body.model).await?)))
}

/// `POST /api/local-llm/preload` — keep one installed model resident.
pub async fn preload(Json(body): Json<ModelIdBody>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(preload_ollama_model(&body.model_id).await?)))
}

/// `POST /api/local-llm/stop-model` — unload one model from memory.
pub async fn stop_model(Json(body): Json<ModelIdBody>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(stop_ollama_model(&body.model_id).await?)))
}

/// `POST /api/local-llm/delete-model` — delete model files and Hope references.
pub async fn delete_model(Json(body): Json<ModelIdBody>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(delete_ollama_model(&body.model_id).await?)))
}

/// `POST /api/local-llm/provider-model` — register an installed model in the Ollama provider.
pub async fn add_provider_model(Json(body): Json<ModelIdBody>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        register_ollama_model_as_provider(&body.model_id, None, false).await?
    )))
}

/// `POST /api/local-llm/default-model` — register and set an installed model active.
pub async fn set_default_model(Json(body): Json<ModelIdBody>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        register_ollama_model_as_provider(&body.model_id, None, true).await?
    )))
}

/// `POST /api/local-llm/embedding-config` — add an installed Ollama embedding model to reusable configs.
pub async fn add_embedding_config(Json(body): Json<ModelIdBody>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        add_ollama_model_as_embedding_config(&body.model_id).await?
    )))
}
