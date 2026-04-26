use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::provider::{self, AvailableModel, ProviderConfig, ProviderWriteError};

use crate::error::AppError;

// ── Request / Response Types ───────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveModelRequest {
    pub provider_id: String,
    pub model_id: String,
}

fn provider_write_error(err: ProviderWriteError) -> AppError {
    match err {
        ProviderWriteError::NotFound(_) | ProviderWriteError::ModelNotFound { .. } => {
            AppError::not_found(err.to_string())
        }
        ProviderWriteError::UnknownLocalBackend(_) => AppError::bad_request(err.to_string()),
        ProviderWriteError::Config(err) => AppError::internal(err.to_string()),
    }
}

// ── Handlers ───────────────────────────────────────────────────

/// `GET /api/providers` — list all providers (API keys masked).
pub async fn list_providers() -> Result<Json<Vec<ProviderConfig>>, AppError> {
    let store = ha_core::config::cached_config();
    let masked: Vec<ProviderConfig> = store.providers.iter().map(|p| p.masked()).collect();
    Ok(Json(masked))
}

/// `GET /api/providers/has-any` — whether any provider is configured.
/// [App.tsx] uses this at startup to decide whether to show the first-run
/// Provider wizard; missing route made HTTP clients crash on startup.
pub async fn has_providers() -> Result<Json<bool>, AppError> {
    let store = ha_core::config::cached_config();
    Ok(Json(!store.providers.is_empty()))
}

/// `POST /api/providers` — add a new provider.
pub async fn add_provider(
    Json(config): Json<ProviderConfig>,
) -> Result<Json<ProviderConfig>, AppError> {
    let masked = provider::add_provider(config, "http").map_err(provider_write_error)?;
    Ok(Json(masked))
}

/// `PUT /api/providers/{id}` — update an existing provider.
pub async fn update_provider(
    Path(id): Path<String>,
    Json(mut config): Json<ProviderConfig>,
) -> Result<Json<Value>, AppError> {
    config.id = id;
    provider::update_provider(config, "http").map_err(provider_write_error)?;
    Ok(Json(json!({ "updated": true })))
}

/// `DELETE /api/providers/{id}` — delete a provider.
pub async fn delete_provider(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    provider::delete_provider(id, "http").map_err(provider_write_error)?;
    Ok(Json(json!({ "deleted": true })))
}

/// `POST /api/providers/test` — test provider connection.
pub async fn test_provider(Json(config): Json<ProviderConfig>) -> Result<Json<Value>, AppError> {
    let payload = ha_core::provider::test::test_provider(config)
        .await
        .unwrap_or_else(|e| e);
    let v: Value = serde_json::from_str(&payload).unwrap_or(Value::String(payload));
    Ok(Json(v))
}

/// `GET /api/providers/active-model` — get the currently active model.
pub async fn get_active_model() -> Result<Json<Value>, AppError> {
    let store = ha_core::config::cached_config();
    Ok(Json(json!({ "active_model": store.active_model })))
}

/// `GET /api/providers/available-models` — list all available models from enabled providers.
pub async fn get_available_models() -> Result<Json<Vec<AvailableModel>>, AppError> {
    let store = ha_core::config::cached_config();
    Ok(Json(provider::build_available_models(&store.providers)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderBody {
    pub provider_ids: Vec<String>,
}

/// `POST /api/providers/reorder` — reorder providers.
pub async fn reorder_providers(Json(body): Json<ReorderBody>) -> Result<Json<Value>, AppError> {
    provider::reorder_providers(body.provider_ids, "http").map_err(provider_write_error)?;
    Ok(Json(json!({ "reordered": true })))
}

/// Body wrapper: matches the Tauri command signature
/// `test_embedding(config: EmbeddingConfig)`. The frontend ships
/// `{ config: embeddingConfig }` (the param name is `config`), not the
/// EmbeddingConfig directly.
#[derive(Debug, Deserialize)]
pub struct TestEmbeddingBody {
    pub config: ha_core::memory::EmbeddingConfig,
}

/// `POST /api/providers/test-embedding` — ping an embedding provider.
///
/// Returns the JSON blob produced by `ha_core::provider::test::test_embedding`.
/// On error returns 200 with the failure payload (the frontend reads
/// `success: bool` from the body) so behaviour matches the Tauri command,
/// which always returns the JSON string.
pub async fn test_embedding(Json(body): Json<TestEmbeddingBody>) -> Result<Json<Value>, AppError> {
    let payload = ha_core::provider::test::test_embedding(body.config)
        .await
        .unwrap_or_else(|e| e);
    let v: Value = serde_json::from_str(&payload).unwrap_or(Value::String(payload));
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestImageBody {
    pub provider_id: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

/// `POST /api/providers/test-image` — ping an image-generation provider.
pub async fn test_image_generate(Json(body): Json<TestImageBody>) -> Result<Json<Value>, AppError> {
    let payload =
        ha_core::provider::test::test_image_generate(body.provider_id, body.api_key, body.base_url)
            .await
            .unwrap_or_else(|e| e);
    let v: Value = serde_json::from_str(&payload).unwrap_or(Value::String(payload));
    Ok(Json(v))
}

/// Body for [`test_model`]. Matches Tauri's `test_model(config, modelId)` signature.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestModelBody {
    pub config: ProviderConfig,
    pub model_id: String,
}

/// `POST /api/providers/test-model` — single-turn chat probe against a
/// specific model of the given provider. Response shape matches the Tauri
/// command; on failure returns 200 with the failure payload inlined (the
/// frontend reads `success: bool` from the body).
pub async fn test_model(Json(body): Json<TestModelBody>) -> Result<Json<Value>, AppError> {
    let payload = ha_core::provider::test::test_model(body.config, body.model_id)
        .await
        .unwrap_or_else(|e| e);
    let v: Value = serde_json::from_str(&payload).unwrap_or(Value::String(payload));
    Ok(Json(v))
}

/// `PUT /api/providers/active-model` — set the active model.
pub async fn set_active_model(
    Json(body): Json<SetActiveModelRequest>,
) -> Result<Json<Value>, AppError> {
    provider::set_active_model(body.provider_id, body.model_id, "http")
        .map_err(provider_write_error)?;
    Ok(Json(json!({ "updated": true })))
}
