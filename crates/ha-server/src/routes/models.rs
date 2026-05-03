//! Model-management routes.
//!
//! These wrap the same config store / provider helpers used by the
//! `/api/providers/*` endpoints, but live under `/api/models/*` to match the
//! frontend `COMMAND_MAP` expectations (see `src/lib/transport-http.ts`).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::provider::{self, ActiveModel, AvailableModel, ProviderWriteError};

use crate::error::AppError;
use crate::AppContext;

fn provider_write_error(err: ProviderWriteError) -> AppError {
    match err {
        ProviderWriteError::NotFound(_) | ProviderWriteError::ModelNotFound { .. } => {
            AppError::not_found(err.to_string())
        }
        ProviderWriteError::UnknownLocalBackend(_) => AppError::bad_request(err.to_string()),
        ProviderWriteError::Config(err) => AppError::internal(err.to_string()),
    }
}

// ── Request / Response types ───────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveModelBody {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SetFallbackBody {
    pub models: Vec<ActiveModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetReasoningEffortBody {
    pub effort: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetTemperatureBody {
    pub temperature: Option<f64>,
}

/// Mirror of the Tauri `CurrentSettings` struct. Field names stay
/// snake_case (no `rename_all`) to match what the desktop returns — frontend
/// reads `result.reasoning_effort` directly. See `ChatScreen.tsx`:
///   `getTransport().call<{ model: string; reasoning_effort: string }>("get_current_settings")`
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
    let store = ha_core::config::cached_config();
    Ok(Json(provider::build_available_models(&store.providers)))
}

/// `GET /api/models/active` — currently active model, if any.
pub async fn get_active_model() -> Result<Json<Value>, AppError> {
    let store = ha_core::config::cached_config();
    Ok(Json(json!({ "active_model": store.active_model })))
}

/// `POST /api/models/active` — set the active model.
pub async fn set_active_model(
    Json(body): Json<SetActiveModelBody>,
) -> Result<Json<Value>, AppError> {
    provider::set_active_model(body.provider_id, body.model_id, "http")
        .map_err(provider_write_error)?;
    Ok(Json(json!({ "updated": true })))
}

/// `GET /api/models/fallback` — ordered fallback model chain.
pub async fn get_fallback_models() -> Result<Json<Vec<ActiveModel>>, AppError> {
    let store = ha_core::config::cached_config();
    Ok(Json(store.fallback_models.clone()))
}

/// `POST /api/models/fallback` — overwrite the fallback model chain.
pub async fn set_fallback_models(
    Json(body): Json<SetFallbackBody>,
) -> Result<Json<Value>, AppError> {
    let mut store = ha_core::config::load_config()?;
    store.fallback_models = body.models;
    ha_core::config::save_config(&store)?;
    Ok(Json(json!({ "updated": true })))
}

/// `POST /api/models/reasoning-effort` — validate and persist the current
/// Think / reasoning effort. When `sessionId` is supplied, it is also stored
/// as a session-scoped override so switching conversations restores it.
pub async fn set_reasoning_effort(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<SetReasoningEffortBody>,
) -> Result<Json<Value>, AppError> {
    if !ha_core::agent::is_valid_reasoning_effort(&body.effort) {
        return Err(AppError::bad_request(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            body.effort,
            ha_core::agent::VALID_REASONING_EFFORTS
        )));
    }
    if let Some(cell) = ha_core::get_reasoning_effort_cell() {
        *cell.lock().await = body.effort.clone();
    }
    if let Some(session_id) = body
        .session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
    {
        ctx.session_db
            .update_session_reasoning_effort(session_id, Some(&body.effort))?;
    }
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/models/settings` — snapshot of the current model + defaults.
pub async fn get_current_settings() -> Result<Json<CurrentSettings>, AppError> {
    let store = ha_core::config::cached_config();
    let model = store
        .active_model
        .as_ref()
        .map(|am| am.model_id.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let reasoning_effort = if let Some(cell) = ha_core::get_reasoning_effort_cell() {
        cell.lock().await.clone()
    } else {
        "medium".to_string()
    };
    Ok(Json(CurrentSettings {
        model,
        reasoning_effort,
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
    let mut store = ha_core::config::load_config()?;
    store.temperature = body.temperature;
    ha_core::config::save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/models/temperature` — get the global default temperature.
pub async fn get_global_temperature() -> Result<Json<Value>, AppError> {
    let store = ha_core::config::cached_config();
    Ok(Json(json!({ "temperature": store.temperature })))
}
