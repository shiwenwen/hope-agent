use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Helpers ─────────────────────────────────────────────────────

fn load_store() -> Result<oc_core::provider::ProviderStore, AppError> {
    Ok(oc_core::provider::load_store()?)
}

fn save_store(store: &oc_core::provider::ProviderStore) -> Result<(), AppError> {
    Ok(oc_core::provider::save_store(store)?)
}

// ── User Config ─────────────────────────────────────────────────

/// `GET /api/config/user` -- get user config.
pub async fn get_user_config() -> Result<Json<oc_core::user_config::UserConfig>, AppError> {
    let config = oc_core::user_config::load_user_config()?;
    Ok(Json(config))
}

/// `PUT /api/config/user` -- save user config.
pub async fn save_user_config(
    Json(config): Json<oc_core::user_config::UserConfig>,
) -> Result<Json<Value>, AppError> {
    oc_core::user_config::save_user_config_to_disk(&config)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Web Search Config ───────────────────────────────────────────

/// `GET /api/config/web-search` -- get web search config.
pub async fn get_web_search_config(
) -> Result<Json<oc_core::tools::web_search::WebSearchConfig>, AppError> {
    let store = load_store()?;
    let mut config = store.web_search;
    oc_core::tools::web_search::backfill_providers(&mut config);
    Ok(Json(config))
}

/// `PUT /api/config/web-search` -- save web search config.
pub async fn save_web_search_config(
    Json(config): Json<oc_core::tools::web_search::WebSearchConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_store()?;
    store.web_search = config;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Proxy Config ────────────────────────────────────────────────

/// `GET /api/config/proxy` -- get proxy config.
pub async fn get_proxy_config() -> Result<Json<oc_core::provider::ProxyConfig>, AppError> {
    let store = load_store()?;
    Ok(Json(store.proxy))
}

/// `PUT /api/config/proxy` -- save proxy config.
pub async fn save_proxy_config(
    Json(config): Json<oc_core::provider::ProxyConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_store()?;
    store.proxy = config;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Compact Config ──────────────────────────────────────────────

/// `GET /api/config/compact` -- get context compaction config.
pub async fn get_compact_config(
) -> Result<Json<oc_core::context_compact::CompactConfig>, AppError> {
    let store = load_store()?;
    Ok(Json(store.compact))
}

/// `PUT /api/config/compact` -- save context compaction config.
pub async fn save_compact_config(
    Json(config): Json<oc_core::context_compact::CompactConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_store()?;
    store.compact = config;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Notification Config ─────────────────────────────────────────

/// `GET /api/config/notification` -- get notification config.
pub async fn get_notification_config(
) -> Result<Json<oc_core::provider::NotificationConfig>, AppError> {
    let store = load_store()?;
    Ok(Json(store.notification))
}

/// `PUT /api/config/notification` -- save notification config.
pub async fn save_notification_config(
    Json(config): Json<oc_core::provider::NotificationConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_store()?;
    store.notification = config;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Plan Config ─────────────────────────────────────────────────

/// `GET /api/config/plan-subagent` -- get plan subagent toggle.
pub async fn get_plan_subagent() -> Result<Json<Value>, AppError> {
    let store = load_store()?;
    Ok(Json(json!(store.plan_subagent)))
}

/// `POST /api/config/plan-subagent` -- set plan subagent toggle.
pub async fn set_plan_subagent(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut store = load_store()?;
    store.plan_subagent = enabled;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/plan-question-timeout` -- get plan question timeout (seconds).
pub async fn get_plan_question_timeout() -> Result<Json<Value>, AppError> {
    let store = load_store()?;
    Ok(Json(json!(store.plan_question_timeout_secs)))
}

/// `POST /api/config/plan-question-timeout` -- set plan question timeout (seconds).
pub async fn set_plan_question_timeout(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let secs = body
        .get("secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(1800);
    let mut store = load_store()?;
    store.plan_question_timeout_secs = secs;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Server Config ──────────────────────────────────────────────

/// `GET /api/config/server` -- get embedded server config (api_key masked).
pub async fn get_server_config() -> Result<Json<Value>, AppError> {
    let store = load_store()?;
    let server = &store.server;
    // Mask api_key for security — only reveal whether it's set
    let masked_key = server.api_key.as_ref().map(|k| {
        if k.len() <= 4 {
            "****".to_string()
        } else {
            format!("{}...{}", &k[..2], &k[k.len() - 2..])
        }
    });
    Ok(Json(json!({
        "bindAddr": server.bind_addr,
        "apiKey": masked_key,
        "hasApiKey": server.api_key.is_some(),
    })))
}

/// `PUT /api/config/server` -- save embedded server config.
pub async fn save_server_config(
    Json(config): Json<oc_core::provider::EmbeddedServerConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_store()?;
    store.server = config;
    save_store(&store)?;
    Ok(Json(json!({ "saved": true, "restartRequired": true })))
}
