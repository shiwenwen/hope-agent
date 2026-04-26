//! Local Ollama embedding assistant routes.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::local_embedding::{
    list_models_with_status, pull_and_activate, OllamaEmbeddingModel,
    EVENT_LOCAL_EMBEDDING_PULL_PROGRESS,
};

use crate::error::AppError;
use crate::AppContext;

#[derive(Debug, Deserialize)]
pub struct PullBody {
    pub model: OllamaEmbeddingModel,
}

/// `GET /api/local-embedding/models` — static catalog plus local install state.
pub async fn list_models() -> Json<Value> {
    Json(json!(list_models_with_status().await))
}

/// `POST /api/local-embedding/pull` — pull an Ollama embedding model and
/// write the memory embedding config. Progress is broadcast on EventBus.
pub async fn pull(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<PullBody>,
) -> Result<Json<Value>, AppError> {
    let bus = ctx.event_bus.clone();
    let config = pull_and_activate(body.model, move |p| {
        bus.emit(EVENT_LOCAL_EMBEDDING_PULL_PROGRESS, json!(p));
    })
    .await?;
    Ok(Json(json!(config)))
}
