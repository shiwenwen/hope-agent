use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::provider::{self, ActiveModel, AvailableModel, ProviderConfig};

use crate::error::AppError;

// ── Request / Response Types ───────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveModelRequest {
    pub provider_id: String,
    pub model_id: String,
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
    let mut store = ha_core::config::load_config()?;
    let mut new_provider = ProviderConfig::new(
        config.name,
        config.api_type,
        config.base_url,
        config.api_key,
    );
    new_provider.models = config.models;
    new_provider.auth_profiles = config.auth_profiles;
    new_provider.thinking_style = config.thinking_style;
    new_provider.allow_private_network = config.allow_private_network;

    let masked = new_provider.masked();
    store.providers.push(new_provider);
    ha_core::config::save_config(&store)?;
    Ok(Json(masked))
}

/// `PUT /api/providers/{id}` — update an existing provider.
pub async fn update_provider(
    Path(id): Path<String>,
    Json(config): Json<ProviderConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = ha_core::config::load_config()?;
    if let Some(existing) = store.providers.iter_mut().find(|p| p.id == id) {
        existing.name = config.name;
        existing.api_type = config.api_type;
        existing.base_url = config.base_url;
        // Only update API key if a real key is provided (not the masked version)
        if !provider::is_masked_key(&config.api_key) {
            existing.api_key = config.api_key;
        }
        // Merge auth profile keys: preserve real keys when incoming is masked
        existing.auth_profiles =
            provider::merge_profile_keys(&existing.auth_profiles, &config.auth_profiles);
        existing.models = config.models;
        existing.enabled = config.enabled;
        existing.user_agent = config.user_agent;
        existing.thinking_style = config.thinking_style;
        existing.allow_private_network = config.allow_private_network;
        ha_core::config::save_config(&store)?;
        Ok(Json(json!({ "updated": true })))
    } else {
        Err(AppError::not_found(format!("Provider not found: {}", id)))
    }
}

/// `DELETE /api/providers/{id}` — delete a provider.
pub async fn delete_provider(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let mut store = ha_core::config::load_config()?;
    let len_before = store.providers.len();
    store.providers.retain(|p| p.id != id);
    if store.providers.len() == len_before {
        return Err(AppError::not_found(format!("Provider not found: {}", id)));
    }
    // Clear active model if it was from the deleted provider
    if let Some(ref active) = store.active_model {
        if active.provider_id == id {
            store.active_model = None;
        }
    }
    ha_core::config::save_config(&store)?;
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
    let mut store = ha_core::config::load_config()?;
    let mut reordered = Vec::with_capacity(body.provider_ids.len());
    for id in &body.provider_ids {
        if let Some(p) = store.providers.iter().find(|p| &p.id == id) {
            reordered.push(p.clone());
        }
    }
    for p in &store.providers {
        if !body.provider_ids.contains(&p.id) {
            reordered.push(p.clone());
        }
    }
    store.providers = reordered;
    ha_core::config::save_config(&store)?;
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
    let mut store = ha_core::config::load_config()?;

    // Verify provider and model exist
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == body.provider_id)
        .ok_or_else(|| AppError::not_found(format!("Provider not found: {}", body.provider_id)))?;

    if !provider.models.iter().any(|m| m.id == body.model_id) {
        return Err(AppError::not_found(format!(
            "Model not found: {}",
            body.model_id
        )));
    }

    store.active_model = Some(ActiveModel {
        provider_id: body.provider_id,
        model_id: body.model_id,
    });
    ha_core::config::save_config(&store)?;
    Ok(Json(json!({ "updated": true })))
}
