use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Helpers ─────────────────────────────────────────────────────

fn load_config() -> Result<oc_core::config::AppConfig, AppError> {
    Ok(oc_core::config::load_config()?)
}

fn save_config(store: &oc_core::config::AppConfig) -> Result<(), AppError> {
    Ok(oc_core::config::save_config(store)?)
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
    let store = load_config()?;
    let mut config = store.web_search;
    oc_core::tools::web_search::backfill_providers(&mut config);
    Ok(Json(config))
}

/// `PUT /api/config/web-search` -- save web search config.
pub async fn save_web_search_config(
    Json(config): Json<oc_core::tools::web_search::WebSearchConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.web_search = config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Proxy Config ────────────────────────────────────────────────

/// `GET /api/config/proxy` -- get proxy config.
pub async fn get_proxy_config() -> Result<Json<oc_core::provider::ProxyConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.proxy))
}

/// `PUT /api/config/proxy` -- save proxy config.
pub async fn save_proxy_config(
    Json(config): Json<oc_core::provider::ProxyConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.proxy = config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Compact Config ──────────────────────────────────────────────

/// `GET /api/config/compact` -- get context compaction config.
pub async fn get_compact_config() -> Result<Json<oc_core::context_compact::CompactConfig>, AppError>
{
    let store = load_config()?;
    Ok(Json(store.compact))
}

/// `PUT /api/config/compact` -- save context compaction config.
pub async fn save_compact_config(
    Json(config): Json<oc_core::context_compact::CompactConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.compact = config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Notification Config ─────────────────────────────────────────

/// `GET /api/config/notification` -- get notification config.
pub async fn get_notification_config() -> Result<Json<oc_core::config::NotificationConfig>, AppError>
{
    let store = load_config()?;
    Ok(Json(store.notification))
}

/// `PUT /api/config/notification` -- save notification config.
pub async fn save_notification_config(
    Json(config): Json<oc_core::config::NotificationConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.notification = config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Tool Config ─────────────────────────────────────────────────

/// `GET /api/config/tool-timeout` -- get tool execution timeout (seconds).
pub async fn get_tool_timeout() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.tool_timeout)))
}

/// `POST /api/config/tool-timeout` -- set tool execution timeout (seconds).
pub async fn set_tool_timeout(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let seconds = body.get("seconds").and_then(|v| v.as_u64()).unwrap_or(300);
    let mut store = load_config()?;
    store.tool_timeout = seconds;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/approval-timeout` -- get tool approval wait timeout (seconds).
pub async fn get_approval_timeout() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.approval_timeout_secs)))
}

/// `POST /api/config/approval-timeout` -- set tool approval wait timeout (seconds).
pub async fn set_approval_timeout(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let seconds = body.get("seconds").and_then(|v| v.as_u64()).unwrap_or(300);
    let mut store = load_config()?;
    store.approval_timeout_secs = seconds;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/approval-timeout-action` -- get approval timeout action.
pub async fn get_approval_timeout_action() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.approval_timeout_action)))
}

/// `POST /api/config/approval-timeout-action` -- set approval timeout action.
pub async fn set_approval_timeout_action(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let action = match body.get("action").and_then(|v| v.as_str()) {
        Some("proceed") => oc_core::config::ApprovalTimeoutAction::Proceed,
        _ => oc_core::config::ApprovalTimeoutAction::Deny,
    };
    let mut store = load_config()?;
    store.approval_timeout_action = action;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/tool-result-threshold` -- get disk persistence threshold (bytes).
pub async fn get_tool_result_disk_threshold() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store
        .tool_result_disk_threshold
        .unwrap_or(50_000))))
}

/// `POST /api/config/tool-result-threshold` -- set disk persistence threshold (bytes).
pub async fn set_tool_result_disk_threshold(
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let bytes = body.get("bytes").and_then(|v| v.as_u64()).unwrap_or(50_000) as usize;
    let mut store = load_config()?;
    store.tool_result_disk_threshold = Some(bytes);
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/tool-limits` -- get tool image/pdf limits.
pub async fn get_tool_limits() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!({
        "maxImages": store.image.max_images,
        "maxPdfs": store.pdf.max_pdfs,
        "maxVisionPages": store.pdf.max_vision_pages,
    })))
}

/// `POST /api/config/tool-limits` -- set tool image/pdf limits.
pub async fn set_tool_limits(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let config = body.get("config").cloned().unwrap_or(Value::Null);
    let max_images = config
        .get("maxImages")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;
    let max_pdfs = config.get("maxPdfs").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let max_vision_pages = config
        .get("maxVisionPages")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    let mut store = load_config()?;
    store.image.max_images = max_images;
    store.pdf.max_pdfs = max_pdfs;
    store.pdf.max_vision_pages = max_vision_pages;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Plan Config ─────────────────────────────────────────────────

/// `GET /api/config/plan-subagent` -- get plan subagent toggle.
pub async fn get_plan_subagent() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.plan_subagent)))
}

/// `POST /api/config/plan-subagent` -- set plan subagent toggle.
pub async fn set_plan_subagent(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut store = load_config()?;
    store.plan_subagent = enabled;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/ask-user-question-timeout` -- get ask_user_question timeout (seconds).
pub async fn get_ask_user_question_timeout() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.ask_user_question_timeout_secs)))
}

/// `POST /api/config/ask-user-question-timeout` -- set ask_user_question timeout (seconds).
pub async fn set_ask_user_question_timeout(
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let secs = body.get("secs").and_then(|v| v.as_u64()).unwrap_or(1800);
    let mut store = load_config()?;
    store.ask_user_question_timeout_secs = secs;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Server Config ──────────────────────────────────────────────

/// `GET /api/config/server` -- get embedded server config (api_key masked).
pub async fn get_server_config() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
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
    Json(config): Json<oc_core::config::EmbeddedServerConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.server = config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true, "restartRequired": true })))
}
