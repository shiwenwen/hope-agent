//! Model-management routes.
//!
//! These wrap the same config store / provider helpers used by the
//! `/api/providers/*` endpoints, but live under `/api/models/*` to match the
//! frontend `COMMAND_MAP` expectations (see `src/lib/transport-http.ts`).

use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use oc_core::provider::{self, ActiveModel, AvailableModel};

use crate::error::AppError;

// ── Request / Response types ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetActiveModelBody {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SetFallbackBody {
    pub models: Vec<ActiveModel>,
}

#[derive(Debug, Deserialize)]
pub struct SetReasoningEffortBody {
    pub effort: String,
}

#[derive(Debug, Deserialize)]
pub struct SetTemperatureBody {
    pub temperature: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct CurrentSettings {
    pub model: String,
    pub reasoning_effort: String,
    pub temperature: Option<f64>,
    pub fallback_models: Vec<ActiveModel>,
    pub active_model: Option<ActiveModel>,
}

// ── Handlers ───────────────────────────────────────────────────

/// `GET /api/models` — list every model across enabled providers.
pub async fn list_available_models() -> Result<Json<Vec<AvailableModel>>, AppError> {
    let store = oc_core::config::cached_config();
    Ok(Json(provider::build_available_models(&store.providers)))
}

/// `GET /api/models/active` — currently active model, if any.
pub async fn get_active_model() -> Result<Json<Value>, AppError> {
    let store = oc_core::config::cached_config();
    Ok(Json(json!({ "active_model": store.active_model })))
}

/// `POST /api/models/active` — set the active model.
pub async fn set_active_model(
    Json(body): Json<SetActiveModelBody>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;

    let provider_cfg = store
        .providers
        .iter()
        .find(|p| p.id == body.provider_id)
        .ok_or_else(|| AppError::not_found(format!("Provider not found: {}", body.provider_id)))?;

    if !provider_cfg.models.iter().any(|m| m.id == body.model_id) {
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

/// `GET /api/models/fallback` — ordered fallback model chain.
pub async fn get_fallback_models() -> Result<Json<Vec<ActiveModel>>, AppError> {
    let store = oc_core::config::cached_config();
    Ok(Json(store.fallback_models.clone()))
}

/// `POST /api/models/fallback` — overwrite the fallback model chain.
pub async fn set_fallback_models(
    Json(body): Json<SetFallbackBody>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;
    store.fallback_models = body.models;
    oc_core::config::save_config(&store)?;
    Ok(Json(json!({ "updated": true })))
}

/// `POST /api/models/reasoning-effort` — validate + accept a reasoning
/// effort value. Server mode is stateless (each `/api/chat` request carries
/// its own `reasoning_effort` field) so this is a validate-only no-op
/// mirroring the Tauri command's `set_reasoning_effort_core` gate.
pub async fn set_reasoning_effort(
    Json(body): Json<SetReasoningEffortBody>,
) -> Result<Json<Value>, AppError> {
    let valid = ["none", "low", "medium", "high", "xhigh"];
    if !valid.contains(&body.effort.as_str()) {
        return Err(AppError::bad_request(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            body.effort, valid
        )));
    }
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/models/settings` — snapshot of the current model + defaults.
pub async fn get_current_settings() -> Result<Json<CurrentSettings>, AppError> {
    let store = oc_core::config::cached_config();
    let model = store
        .active_model
        .as_ref()
        .map(|am| am.model_id.clone())
        .unwrap_or_else(|| "unknown".to_string());
    Ok(Json(CurrentSettings {
        model,
        reasoning_effort: "medium".to_string(),
        temperature: store.temperature,
        fallback_models: store.fallback_models.clone(),
        active_model: store.active_model.clone(),
    }))
}

/// `POST /api/models/temperature` — set the global default LLM temperature.
pub async fn set_global_temperature(
    Json(body): Json<SetTemperatureBody>,
) -> Result<Json<Value>, AppError> {
    if let Some(t) = body.temperature {
        if !(0.0..=2.0).contains(&t) {
            return Err(AppError::bad_request(format!(
                "temperature must be in 0.0..=2.0 (got {})",
                t
            )));
        }
    }
    let mut store = oc_core::config::load_config()?;
    store.temperature = body.temperature;
    oc_core::config::save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/models/temperature` — get the global default temperature.
pub async fn get_global_temperature() -> Result<Json<Value>, AppError> {
    let store = oc_core::config::cached_config();
    Ok(Json(json!({ "temperature": store.temperature })))
}
