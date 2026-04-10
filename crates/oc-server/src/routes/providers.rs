use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use oc_core::provider::{self, ActiveModel, AvailableModel, ProviderConfig};

use crate::error::AppError;

// ── Request / Response Types ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetActiveModelRequest {
    pub provider_id: String,
    pub model_id: String,
}

// ── Handlers ───────────────────────────────────────────────────

/// `GET /api/providers` — list all providers (API keys masked).
pub async fn list_providers() -> Result<Json<Vec<ProviderConfig>>, AppError> {
    let store = oc_core::config::cached_config();
    let masked: Vec<ProviderConfig> = store.providers.iter().map(|p| p.masked()).collect();
    Ok(Json(masked))
}

/// `POST /api/providers` — add a new provider.
pub async fn add_provider(
    Json(config): Json<ProviderConfig>,
) -> Result<Json<ProviderConfig>, AppError> {
    let mut store = oc_core::config::load_config()?;
    let mut new_provider = ProviderConfig::new(
        config.name,
        config.api_type,
        config.base_url,
        config.api_key,
    );
    new_provider.models = config.models;

    let masked = new_provider.masked();
    store.providers.push(new_provider);
    oc_core::config::save_config(&store)?;
    Ok(Json(masked))
}

/// `PUT /api/providers/{id}` — update an existing provider.
pub async fn update_provider(
    Path(id): Path<String>,
    Json(config): Json<ProviderConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;
    if let Some(existing) = store.providers.iter_mut().find(|p| p.id == id) {
        existing.name = config.name;
        existing.api_type = config.api_type;
        existing.base_url = config.base_url;
        // Only update API key if a real key is provided (not the masked version)
        if !config.api_key.contains("...") && config.api_key != "****" {
            existing.api_key = config.api_key;
        }
        existing.models = config.models;
        existing.enabled = config.enabled;
        existing.user_agent = config.user_agent;
        existing.thinking_style = config.thinking_style;
        oc_core::config::save_config(&store)?;
        Ok(Json(json!({ "updated": true })))
    } else {
        Err(AppError::not_found(format!("Provider not found: {}", id)))
    }
}

/// `DELETE /api/providers/{id}` — delete a provider.
pub async fn delete_provider(
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;
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
    oc_core::config::save_config(&store)?;
    Ok(Json(json!({ "deleted": true })))
}

/// `POST /api/providers/test` — test provider connection.
pub async fn test_provider(
    Json(config): Json<ProviderConfig>,
) -> Result<Json<Value>, AppError> {
    // Delegate to the same test logic used by the Tauri command.
    // We reimplement a lightweight version here — ping the models endpoint.
    use std::time::{Duration, Instant};

    let client = oc_core::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(&config.user_agent),
    )
    .build()
    .map_err(|e| AppError::internal(format!("Client error: {}", e)))?;

    let base = config.base_url.trim_end_matches('/');
    let has_version_suffix =
        base.ends_with("/v1") || base.ends_with("/v2") || base.ends_with("/v3");
    let start = Instant::now();

    match config.api_type {
        oc_core::provider::ApiType::Anthropic => {
            let url = if has_version_suffix {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            };
            let body = serde_json::json!({
                "model": "test-model",
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "Hi" }]
            });
            let resp = client
                .post(&url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| AppError::internal(format!("Connection failed: {}", e)))?;

            let status = resp.status().as_u16();
            let latency = start.elapsed().as_millis() as u64;
            let success = resp.status().is_success() || status == 400 || status == 404;
            Ok(Json(json!({
                "success": success,
                "status": status,
                "latencyMs": latency,
                "url": url,
            })))
        }
        oc_core::provider::ApiType::OpenaiChat | oc_core::provider::ApiType::OpenaiResponses => {
            let url = if has_version_suffix {
                format!("{}/models", base)
            } else {
                format!("{}/v1/models", base)
            };
            let mut req = client.get(&url);
            if !config.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", config.api_key));
            }
            let resp = req
                .send()
                .await
                .map_err(|e| AppError::internal(format!("Connection failed: {}", e)))?;

            let status = resp.status().as_u16();
            let latency = start.elapsed().as_millis() as u64;
            let success = resp.status().is_success();
            Ok(Json(json!({
                "success": success,
                "status": status,
                "latencyMs": latency,
                "url": url,
            })))
        }
        oc_core::provider::ApiType::Codex => {
            Ok(Json(json!({
                "success": true,
                "message": "Codex uses OAuth, no test needed",
                "latencyMs": 0,
            })))
        }
    }
}

/// `GET /api/providers/active-model` — get the currently active model.
pub async fn get_active_model() -> Result<Json<Value>, AppError> {
    let store = oc_core::config::cached_config();
    Ok(Json(json!({ "active_model": store.active_model })))
}

/// `GET /api/providers/available-models` — list all available models from enabled providers.
pub async fn get_available_models() -> Result<Json<Vec<AvailableModel>>, AppError> {
    let store = oc_core::config::cached_config();
    Ok(Json(provider::build_available_models(&store.providers)))
}

#[derive(Debug, Deserialize)]
pub struct ReorderBody {
    pub provider_ids: Vec<String>,
}

/// `POST /api/providers/reorder` — reorder providers.
pub async fn reorder_providers(
    Json(body): Json<ReorderBody>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;
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
    oc_core::config::save_config(&store)?;
    Ok(Json(json!({ "reordered": true })))
}

/// `PUT /api/providers/active-model` — set the active model.
pub async fn set_active_model(
    Json(body): Json<SetActiveModelRequest>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;

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
    oc_core::config::save_config(&store)?;
    Ok(Json(json!({ "updated": true })))
}
