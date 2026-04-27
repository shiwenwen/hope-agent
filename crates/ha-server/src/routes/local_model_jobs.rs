//! Local model background job routes.

use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::local_embedding::OllamaEmbeddingModel;
use ha_core::local_llm::{ModelCandidate, OllamaPullRequest};
use ha_core::local_model_jobs::{self, LocalModelJobLogEntry, LocalModelJobSnapshot};

use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct StartChatBody {
    pub model: ModelCandidate,
}

#[derive(Debug, Deserialize)]
pub struct StartEmbeddingBody {
    pub model: OllamaEmbeddingModel,
}

#[derive(Debug, Deserialize)]
pub struct StartOllamaPullBody {
    pub request: OllamaPullRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOllamaPreloadBody {
    pub model_id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsQuery {
    #[serde(alias = "after_seq")]
    pub after_seq: Option<i64>,
}

/// `POST /api/local-model-jobs/chat-model`
pub async fn start_chat_model(
    Json(body): Json<StartChatBody>,
) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::start_chat_model_job(
        body.model, None,
    )?))
}

/// `POST /api/local-model-jobs/embedding`
pub async fn start_embedding(
    Json(body): Json<StartEmbeddingBody>,
) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::start_embedding_job(body.model)?))
}

/// `POST /api/local-model-jobs/ollama-install`
pub async fn start_ollama_install() -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::start_ollama_install_job()?))
}

/// `POST /api/local-model-jobs/ollama-pull`
pub async fn start_ollama_pull(
    Json(body): Json<StartOllamaPullBody>,
) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::start_ollama_pull_job(body.request)?))
}

/// `POST /api/local-model-jobs/ollama-preload`
pub async fn start_ollama_preload(
    Json(body): Json<StartOllamaPreloadBody>,
) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::start_ollama_preload_job(
        body.model_id,
        body.display_name,
    )?))
}

/// `GET /api/local-model-jobs`
pub async fn list_jobs() -> Result<Json<Vec<LocalModelJobSnapshot>>, AppError> {
    Ok(Json(local_model_jobs::list_jobs()?))
}

/// `GET /api/local-model-jobs/{id}`
pub async fn get_job(Path(id): Path<String>) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    let job = local_model_jobs::get_job(&id)?
        .ok_or_else(|| AppError::not_found(format!("local model job not found: {id}")))?;
    Ok(Json(job))
}

/// `GET /api/local-model-jobs/{id}/logs?afterSeq=...`
pub async fn get_logs(
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<LocalModelJobLogEntry>>, AppError> {
    Ok(Json(local_model_jobs::get_logs(&id, q.after_seq)?))
}

/// `POST /api/local-model-jobs/{id}/cancel`
pub async fn cancel_job(Path(id): Path<String>) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::cancel_job(&id)?))
}

/// `POST /api/local-model-jobs/{id}/pause`
pub async fn pause_job(Path(id): Path<String>) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::pause_job(&id)?))
}

/// `POST /api/local-model-jobs/{id}/retry`
pub async fn retry_job(Path(id): Path<String>) -> Result<Json<LocalModelJobSnapshot>, AppError> {
    Ok(Json(local_model_jobs::retry_job(&id, None)?))
}

/// `DELETE /api/local-model-jobs/{id}`
pub async fn clear_job(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    local_model_jobs::clear_job(&id)?;
    Ok(Json(json!({ "cleared": true })))
}
