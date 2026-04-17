use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Helpers ─────────────────────────────────────────────────────

fn load_config() -> Result<oc_core::config::AppConfig, AppError> {
    Ok(oc_core::config::load_config()?)
}

fn save_config(store: &oc_core::config::AppConfig) -> Result<(), AppError> {
    Ok(oc_core::config::save_config(store)?)
}

/// Generic body wrapper used by every `save_*_config` handler.
///
/// All Tauri `save_*_config(config: T)` commands take a single struct
/// parameter named `config`. The frontend HTTP transport mirrors that by
/// shipping `{ config: <T> }` rather than `<T>` directly. Without this
/// wrapper, axum's `Json<T>` extractor would fail because it would look
/// for top-level fields of `T` directly in the body.
#[derive(Debug, Deserialize)]
pub struct ConfigBody<T> {
    pub config: T,
}

// ── User Config ─────────────────────────────────────────────────

/// `GET /api/config/user` -- get user config.
pub async fn get_user_config() -> Result<Json<oc_core::user_config::UserConfig>, AppError> {
    let config = oc_core::user_config::load_user_config()?;
    Ok(Json(config))
}

/// `PUT /api/config/user` -- save user config.
pub async fn save_user_config(
    Json(body): Json<ConfigBody<oc_core::user_config::UserConfig>>,
) -> Result<Json<Value>, AppError> {
    oc_core::user_config::save_user_config_to_disk(&body.config)?;
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
    Json(body): Json<ConfigBody<oc_core::tools::web_search::WebSearchConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.web_search = body.config;
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
    Json(body): Json<ConfigBody<oc_core::provider::ProxyConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.proxy = body.config;
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
    Json(body): Json<ConfigBody<oc_core::context_compact::CompactConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.compact = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Async Tools Config ──────────────────────────────────────────

/// `GET /api/config/async-tools` -- get async tool execution config.
pub async fn get_async_tools_config(
) -> Result<Json<oc_core::config::AsyncToolsConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.async_tools))
}

/// `PUT /api/config/async-tools` -- save async tool execution config.
pub async fn save_async_tools_config(
    Json(body): Json<ConfigBody<oc_core::config::AsyncToolsConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.async_tools = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Deferred Tools Config ───────────────────────────────────────

/// `GET /api/config/deferred-tools` -- get deferred tool loading config.
pub async fn get_deferred_tools_config(
) -> Result<Json<oc_core::config::DeferredToolsConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.deferred_tools))
}

/// `PUT /api/config/deferred-tools` -- save deferred tool loading config.
pub async fn save_deferred_tools_config(
    Json(body): Json<ConfigBody<oc_core::config::DeferredToolsConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.deferred_tools = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Memory Selection Config ─────────────────────────────────────

/// `GET /api/config/memory-selection` -- get LLM memory selection config.
pub async fn get_memory_selection_config(
) -> Result<Json<oc_core::memory::MemorySelectionConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.memory_selection))
}

/// `PUT /api/config/memory-selection` -- save LLM memory selection config.
pub async fn save_memory_selection_config(
    Json(body): Json<ConfigBody<oc_core::memory::MemorySelectionConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.memory_selection = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Recap Config ────────────────────────────────────────────────

/// `GET /api/config/recap` -- get recap config.
pub async fn get_recap_config() -> Result<Json<oc_core::config::RecapConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.recap))
}

/// `PUT /api/config/recap` -- save recap config.
pub async fn save_recap_config(
    Json(body): Json<ConfigBody<oc_core::config::RecapConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.recap = body.config;
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
    Json(body): Json<ConfigBody<oc_core::config::NotificationConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.notification = body.config;
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
    Json(body): Json<ConfigBody<oc_core::config::EmbeddedServerConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.server = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true, "restartRequired": true })))
}

// ── Memory / Embedding Configs ──────────────────────────────────

/// `GET /api/config/embedding` -- get embedding provider config.
pub async fn get_embedding_config(
) -> Result<Json<oc_core::memory::EmbeddingConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.embedding))
}

/// `PUT /api/config/embedding` -- save embedding provider config.
pub async fn save_embedding_config(
    Json(body): Json<ConfigBody<oc_core::memory::EmbeddingConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.embedding = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/embedding/presets` -- list built-in embedding presets.
pub async fn get_embedding_presets(
) -> Result<Json<Vec<oc_core::memory::EmbeddingPreset>>, AppError> {
    Ok(Json(oc_core::memory::embedding_presets()))
}

/// `GET /api/config/embedding-cache` -- get embedding cache config.
pub async fn get_embedding_cache_config(
) -> Result<Json<oc_core::memory::EmbeddingCacheConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.embedding_cache))
}

/// `PUT /api/config/embedding-cache` -- save embedding cache config.
pub async fn save_embedding_cache_config(
    Json(body): Json<ConfigBody<oc_core::memory::EmbeddingCacheConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.embedding_cache = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/dedup` -- get memory deduplication config.
pub async fn get_dedup_config() -> Result<Json<oc_core::memory::DedupConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.dedup))
}

/// `PUT /api/config/dedup` -- save memory deduplication config.
pub async fn save_dedup_config(
    Json(body): Json<ConfigBody<oc_core::memory::DedupConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.dedup = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/hybrid-search` -- get hybrid search weights.
pub async fn get_hybrid_search_config(
) -> Result<Json<oc_core::memory::HybridSearchConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.hybrid_search))
}

/// `PUT /api/config/hybrid-search` -- save hybrid search weights.
pub async fn save_hybrid_search_config(
    Json(body): Json<ConfigBody<oc_core::memory::HybridSearchConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.hybrid_search = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/mmr` -- get MMR reranking config.
pub async fn get_mmr_config() -> Result<Json<oc_core::memory::MmrConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.mmr))
}

/// `PUT /api/config/mmr` -- save MMR reranking config.
pub async fn save_mmr_config(
    Json(body): Json<ConfigBody<oc_core::memory::MmrConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.mmr = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/multimodal` -- get multimodal embedding config.
pub async fn get_multimodal_config(
) -> Result<Json<oc_core::memory::MultimodalConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.multimodal))
}

/// `PUT /api/config/multimodal` -- save multimodal embedding config.
pub async fn save_multimodal_config(
    Json(body): Json<ConfigBody<oc_core::memory::MultimodalConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.multimodal = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/temporal-decay` -- get temporal decay config.
pub async fn get_temporal_decay_config(
) -> Result<Json<oc_core::memory::TemporalDecayConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.temporal_decay))
}

/// `PUT /api/config/temporal-decay` -- save temporal decay config.
pub async fn save_temporal_decay_config(
    Json(body): Json<ConfigBody<oc_core::memory::TemporalDecayConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.temporal_decay = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/extract` -- get memory auto-extract config.
pub async fn get_extract_config(
) -> Result<Json<oc_core::memory::MemoryExtractConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.memory_extract))
}

/// `PUT /api/config/extract` -- save memory auto-extract config.
pub async fn save_extract_config(
    Json(body): Json<ConfigBody<oc_core::memory::MemoryExtractConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.memory_extract = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Web Fetch / Image Generate / Canvas ────────────────────────

/// `GET /api/config/web-fetch` -- get web fetch tool config.
pub async fn get_web_fetch_config(
) -> Result<Json<oc_core::tools::web_fetch::WebFetchConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.web_fetch))
}

/// `PUT /api/config/web-fetch` -- save web fetch tool config.
pub async fn save_web_fetch_config(
    Json(body): Json<ConfigBody<oc_core::tools::web_fetch::WebFetchConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.web_fetch = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/ssrf` -- get SSRF policy config.
pub async fn get_ssrf_config(
) -> Result<Json<oc_core::security::ssrf::SsrfConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.ssrf))
}

/// `PUT /api/config/ssrf` -- save SSRF policy config.
pub async fn save_ssrf_config(
    Json(body): Json<ConfigBody<oc_core::security::ssrf::SsrfConfig>>,
) -> Result<Json<Value>, AppError> {
    let _guard = oc_core::backup::scope_save_reason("security.ssrf", "http-api");
    let mut store = load_config()?;
    store.ssrf = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/image-generate` -- get image generation config.
pub async fn get_image_generate_config(
) -> Result<Json<oc_core::tools::image_generate::ImageGenConfig>, AppError> {
    let store = load_config()?;
    let mut config = store.image_generate;
    oc_core::tools::image_generate::backfill_providers(&mut config);
    Ok(Json(config))
}

/// `PUT /api/config/image-generate` -- save image generation config.
pub async fn save_image_generate_config(
    Json(body): Json<ConfigBody<oc_core::tools::image_generate::ImageGenConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.image_generate = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/canvas` -- get canvas tool config.
pub async fn get_canvas_config(
) -> Result<Json<oc_core::tools::canvas::CanvasConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.canvas))
}

/// `PUT /api/config/canvas` -- save canvas tool config.
pub async fn save_canvas_config(
    Json(body): Json<ConfigBody<oc_core::tools::canvas::CanvasConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.canvas = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Shortcuts ───────────────────────────────────────────────────

/// `GET /api/config/shortcuts` -- get global keyboard shortcut config.
pub async fn get_shortcut_config() -> Result<Json<oc_core::config::ShortcutConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.shortcuts))
}

/// `PUT /api/config/shortcuts` -- save global keyboard shortcut config.
///
/// Only persists the config — the actual OS-level shortcut registration is
/// performed by the Tauri desktop shell. In headless server mode this is a
/// no-op beyond saving the value.
pub async fn save_shortcut_config(
    Json(body): Json<ConfigBody<oc_core::config::ShortcutConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.shortcuts = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true, "note": "desktop-only registration" })))
}

/// `POST /api/config/shortcuts/pause` -- temporarily pause shortcut capture.
///
/// Desktop-only: in headless mode this is a no-op. Returns 200 regardless.
pub async fn set_shortcuts_paused(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "ok": true, "note": "desktop-only" })))
}

// ── Theme / Language / UI ──────────────────────────────────────

/// `GET /api/config/theme` -- get UI theme ("auto" | "light" | "dark").
pub async fn get_theme() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.theme)))
}

/// `POST /api/config/theme` -- set UI theme.
pub async fn set_theme(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let theme = body
        .get("theme")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    let mut store = load_config()?;
    store.theme = theme;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `POST /api/config/window-theme` -- desktop-only, no-op in server mode.
pub async fn set_window_theme(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "ok": true, "note": "desktop-only" })))
}

/// `GET /api/config/language` -- get UI language code.
pub async fn get_language() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.language)))
}

/// `POST /api/config/language` -- set UI language code.
pub async fn set_language(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let language = body
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    let mut store = load_config()?;
    store.language = language;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/ui-effects` -- get UI background effects toggle.
pub async fn get_ui_effects_enabled() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.ui_effects_enabled)))
}

/// `POST /api/config/ui-effects` -- set UI background effects toggle.
pub async fn set_ui_effects_enabled(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut store = load_config()?;
    store.ui_effects_enabled = enabled;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/autostart` -- desktop-only, always reports false in server mode.
pub async fn get_autostart_enabled() -> Result<Json<Value>, AppError> {
    Ok(Json(json!(false)))
}

/// `POST /api/config/autostart` -- desktop-only, no-op in server mode.
pub async fn set_autostart_enabled(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "ok": true, "note": "desktop-only" })))
}

// ── Sandbox ────────────────────────────────────────────────────

/// `GET /api/config/sandbox` -- get Docker sandbox config.
pub async fn get_sandbox_config() -> Result<Json<oc_core::sandbox::SandboxConfig>, AppError> {
    Ok(Json(oc_core::sandbox::load_sandbox_config()?))
}

/// `PUT /api/config/sandbox` -- save Docker sandbox config.
pub async fn set_sandbox_config(
    Json(body): Json<ConfigBody<oc_core::sandbox::SandboxConfig>>,
) -> Result<Json<Value>, AppError> {
    oc_core::sandbox::save_sandbox_config(&body.config)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Behavior Awareness ──────────────────────────────────────────

/// `GET /api/config/awareness` -- global behavior awareness config.
pub async fn get_cross_session_config(
) -> Result<Json<oc_core::cross_session::CrossSessionConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.cross_session))
}

/// `PUT /api/config/awareness` -- save global behavior awareness config.
pub async fn save_cross_session_config(
    Json(body): Json<ConfigBody<oc_core::cross_session::CrossSessionConfig>>,
) -> Result<Json<Value>, AppError> {
    let mut store = load_config()?;
    store.cross_session = body.config;
    save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}
