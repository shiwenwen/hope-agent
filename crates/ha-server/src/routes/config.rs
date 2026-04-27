use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Helpers ─────────────────────────────────────────────────────

fn load_config() -> Result<ha_core::config::AppConfig, AppError> {
    Ok((*ha_core::config::cached_config()).clone())
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
pub async fn get_user_config() -> Result<Json<ha_core::user_config::UserConfig>, AppError> {
    let config = ha_core::user_config::load_user_config()?;
    Ok(Json(config))
}

/// `PUT /api/config/user` -- save user config.
pub async fn save_user_config(
    Json(body): Json<ConfigBody<ha_core::user_config::UserConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::user_config::save_user_config_to_disk(&body.config)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Web Search Config ───────────────────────────────────────────

/// `GET /api/config/web-search` -- get web search config.
pub async fn get_web_search_config(
) -> Result<Json<ha_core::tools::web_search::WebSearchConfig>, AppError> {
    let store = load_config()?;
    let mut config = store.web_search;
    ha_core::tools::web_search::backfill_providers(&mut config);
    Ok(Json(config))
}

/// `PUT /api/config/web-search` -- save web search config.
pub async fn save_web_search_config(
    Json(body): Json<ConfigBody<ha_core::tools::web_search::WebSearchConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("web_search", "http"), |store| {
        store.web_search = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Proxy Config ────────────────────────────────────────────────

/// `GET /api/config/proxy` -- get proxy config.
pub async fn get_proxy_config() -> Result<Json<ha_core::provider::ProxyConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.proxy))
}

/// `PUT /api/config/proxy` -- save proxy config.
pub async fn save_proxy_config(
    Json(body): Json<ConfigBody<ha_core::provider::ProxyConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("proxy", "http"), |store| {
        store.proxy = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `POST /api/config/proxy/test` -- outbound proxy probe, mirror of the
/// Tauri `test_proxy` command. Returns the same human-readable status line
/// on success; body carries the error message on failure.
pub async fn test_proxy_config(
    Json(body): Json<ConfigBody<ha_core::provider::ProxyConfig>>,
) -> Result<Json<Value>, AppError> {
    match ha_core::provider::test::test_proxy(body.config).await {
        Ok(msg) => Ok(Json(json!({ "success": true, "message": msg }))),
        Err(msg) => Ok(Json(json!({ "success": false, "message": msg }))),
    }
}

// ── Compact Config ──────────────────────────────────────────────

/// `GET /api/config/compact` -- get context compaction config.
pub async fn get_compact_config() -> Result<Json<ha_core::context_compact::CompactConfig>, AppError>
{
    let store = load_config()?;
    Ok(Json(store.compact))
}

/// `PUT /api/config/compact` -- save context compaction config.
pub async fn save_compact_config(
    Json(body): Json<ConfigBody<ha_core::context_compact::CompactConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("compact", "http"), |store| {
        store.compact = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/session-title` -- get LLM session title config.
pub async fn get_session_title_config(
) -> Result<Json<ha_core::session_title::SessionTitleConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.session_title))
}

/// `PUT /api/config/session-title` -- save LLM session title config.
pub async fn save_session_title_config(
    Json(body): Json<ConfigBody<ha_core::session_title::SessionTitleConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("session_title", "http"), |store| {
        store.session_title = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Async Tools Config ──────────────────────────────────────────

/// `GET /api/config/async-tools` -- get async tool execution config.
pub async fn get_async_tools_config() -> Result<Json<ha_core::config::AsyncToolsConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.async_tools))
}

/// `PUT /api/config/async-tools` -- save async tool execution config.
pub async fn save_async_tools_config(
    Json(body): Json<ConfigBody<ha_core::config::AsyncToolsConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("async_tools", "http"), |store| {
        store.async_tools = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Deferred Tools Config ───────────────────────────────────────

/// `GET /api/config/deferred-tools` -- get deferred tool loading config.
pub async fn get_deferred_tools_config(
) -> Result<Json<ha_core::config::DeferredToolsConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.deferred_tools))
}

/// `PUT /api/config/deferred-tools` -- save deferred tool loading config.
pub async fn save_deferred_tools_config(
    Json(body): Json<ConfigBody<ha_core::config::DeferredToolsConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("deferred_tools", "http"), |store| {
        store.deferred_tools = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Memory Selection Config ─────────────────────────────────────

/// `GET /api/config/memory-selection` -- get LLM memory selection config.
pub async fn get_memory_selection_config(
) -> Result<Json<ha_core::memory::MemorySelectionConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.memory_selection))
}

/// `PUT /api/config/memory-selection` -- save LLM memory selection config.
pub async fn save_memory_selection_config(
    Json(body): Json<ConfigBody<ha_core::memory::MemorySelectionConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("memory_selection", "http"), |store| {
        store.memory_selection = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Memory Budget Config ────────────────────────────────────────

/// `GET /api/config/memory-budget` -- get the system-prompt memory budget.
pub async fn get_memory_budget_config(
) -> Result<Json<ha_core::memory::MemoryBudgetConfig>, AppError> {
    Ok(Json(ha_core::config::cached_config().memory_budget.clone()))
}

/// `PUT /api/config/memory-budget` -- save the memory budget.
pub async fn save_memory_budget_config(
    Json(body): Json<ConfigBody<ha_core::memory::MemoryBudgetConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("memory_budget", "http"), |store| {
        store.memory_budget = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Recap Config ────────────────────────────────────────────────

/// `GET /api/config/recap` -- get recap config.
pub async fn get_recap_config() -> Result<Json<ha_core::config::RecapConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.recap))
}

/// `PUT /api/config/recap` -- save recap config.
pub async fn save_recap_config(
    Json(body): Json<ConfigBody<ha_core::config::RecapConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("recap", "http"), |store| {
        store.recap = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Notification Config ─────────────────────────────────────────

/// `GET /api/config/notification` -- get notification config.
pub async fn get_notification_config() -> Result<Json<ha_core::config::NotificationConfig>, AppError>
{
    let store = load_config()?;
    Ok(Json(store.notification))
}

/// `PUT /api/config/notification` -- save notification config.
pub async fn save_notification_config(
    Json(body): Json<ConfigBody<ha_core::config::NotificationConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("notification", "http"), |store| {
        store.notification = body.config;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("tool_timeout", "http"), |store| {
        store.tool_timeout = seconds;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("approval_timeout", "http"), |store| {
        store.approval_timeout_secs = seconds;
        Ok(())
    })?;
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
        Some("proceed") => ha_core::config::ApprovalTimeoutAction::Proceed,
        _ => ha_core::config::ApprovalTimeoutAction::Deny,
    };
    ha_core::config::mutate_config(("approval_timeout_action", "http"), |store| {
        store.approval_timeout_action = action;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("tool_result_disk_threshold", "http"), |store| {
        store.tool_result_disk_threshold = Some(bytes);
        Ok(())
    })?;
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

    ha_core::config::mutate_config(("tool_limits", "http"), |store| {
        store.image.max_images = max_images;
        store.pdf.max_pdfs = max_pdfs;
        store.pdf.max_vision_pages = max_vision_pages;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("plan_subagent", "http"), |store| {
        store.plan_subagent = enabled;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("ask_user_question_timeout", "http"), |store| {
        store.ask_user_question_timeout_secs = secs;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Server Config ──────────────────────────────────────────────

/// `GET /api/config/server` -- get embedded server config (api_key masked).
pub async fn get_server_config() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    let server = &store.server;
    // Mask api_key for security — only reveal whether it's set
    let masked_key = server.api_key.as_ref().map(|k| {
        if k.is_empty() {
            "****".to_string()
        } else {
            ha_core::mask_secret_middle(k, 2, 2)
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
    Json(body): Json<ConfigBody<ha_core::config::EmbeddedServerConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("server", "http"), |store| {
        store.server = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true, "restartRequired": true })))
}

// ── Memory / Embedding Configs ──────────────────────────────────

/// `GET /api/config/embedding` -- get embedding provider config.
pub async fn get_embedding_config() -> Result<Json<ha_core::memory::EmbeddingConfig>, AppError> {
    let store = ha_core::config::cached_config();
    let resolved = ha_core::memory::resolve_memory_embedding_config(
        &store.memory_embedding,
        &store.embedding_models,
    )?;
    Ok(Json(
        resolved
            .map(|(_, config, _)| config)
            .unwrap_or_else(ha_core::memory::EmbeddingConfig::default),
    ))
}

/// `PUT /api/config/embedding` -- save embedding provider config.
pub async fn save_embedding_config(
    Json(body): Json<ConfigBody<ha_core::memory::EmbeddingConfig>>,
) -> Result<Json<Value>, AppError> {
    let state = ha_core::memory::save_legacy_embedding_config(body.config, "http")?;
    Ok(Json(json!({ "saved": true, "state": state })))
}

/// `GET /api/config/embedding/presets` -- list built-in embedding presets.
pub async fn get_embedding_presets() -> Result<Json<Vec<ha_core::memory::EmbeddingPreset>>, AppError>
{
    Ok(Json(ha_core::memory::embedding_presets()))
}

pub async fn embedding_model_config_list(
) -> Result<Json<Vec<ha_core::memory::EmbeddingModelConfig>>, AppError> {
    Ok(Json(ha_core::memory::list_embedding_model_configs()))
}

pub async fn embedding_model_config_templates(
) -> Result<Json<Vec<ha_core::memory::EmbeddingModelTemplate>>, AppError> {
    Ok(Json(ha_core::memory::embedding_model_config_templates()))
}

pub async fn embedding_model_config_save(
    Json(body): Json<ConfigBody<ha_core::memory::EmbeddingModelConfig>>,
) -> Result<Json<ha_core::memory::EmbeddingModelConfig>, AppError> {
    Ok(Json(ha_core::memory::save_embedding_model_config(
        body.config,
        "http",
    )?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelConfigIdBody {
    pub id: String,
}

pub async fn embedding_model_config_delete(
    Json(body): Json<EmbeddingModelConfigIdBody>,
) -> Result<Json<Value>, AppError> {
    ha_core::memory::delete_embedding_model_config(&body.id, "http")?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn embedding_model_config_test(
    Json(body): Json<ConfigBody<ha_core::memory::EmbeddingModelConfig>>,
) -> Result<Json<Value>, AppError> {
    let config = body.config.normalize_for_save();
    config.validate()?;
    let payload = ha_core::provider::test::test_embedding(config.to_runtime_config(true))
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(
        serde_json::from_str(&payload).unwrap_or_else(|_| json!({ "message": payload })),
    ))
}

pub async fn memory_embedding_get() -> Result<Json<ha_core::memory::MemoryEmbeddingState>, AppError>
{
    Ok(Json(ha_core::memory::get_memory_embedding_state()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingSetDefaultBody {
    pub model_config_id: String,
    pub mode: ha_core::memory::ReembedMode,
}

pub async fn memory_embedding_set_default(
    Json(body): Json<MemoryEmbeddingSetDefaultBody>,
) -> Result<Json<ha_core::memory::MemoryEmbeddingSetDefaultResult>, AppError> {
    Ok(Json(ha_core::memory::set_memory_embedding_default(
        &body.model_config_id,
        body.mode,
        "http",
    )?))
}

pub async fn memory_embedding_disable(
) -> Result<Json<ha_core::memory::MemoryEmbeddingState>, AppError> {
    Ok(Json(ha_core::memory::disable_memory_embedding("http")?))
}

/// `GET /api/config/embedding-cache` -- get embedding cache config.
pub async fn get_embedding_cache_config(
) -> Result<Json<ha_core::memory::EmbeddingCacheConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.embedding_cache))
}

/// `PUT /api/config/embedding-cache` -- save embedding cache config.
pub async fn save_embedding_cache_config(
    Json(body): Json<ConfigBody<ha_core::memory::EmbeddingCacheConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("embedding_cache", "http"), |store| {
        store.embedding_cache = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/dedup` -- get memory deduplication config.
pub async fn get_dedup_config() -> Result<Json<ha_core::memory::DedupConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.dedup))
}

/// `PUT /api/config/dedup` -- save memory deduplication config.
pub async fn save_dedup_config(
    Json(body): Json<ConfigBody<ha_core::memory::DedupConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("memory_dedup", "http"), |store| {
        store.dedup = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/hybrid-search` -- get hybrid search weights.
pub async fn get_hybrid_search_config(
) -> Result<Json<ha_core::memory::HybridSearchConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.hybrid_search))
}

/// `PUT /api/config/hybrid-search` -- save hybrid search weights.
pub async fn save_hybrid_search_config(
    Json(body): Json<ConfigBody<ha_core::memory::HybridSearchConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("hybrid_search", "http"), |store| {
        store.hybrid_search = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/mmr` -- get MMR reranking config.
pub async fn get_mmr_config() -> Result<Json<ha_core::memory::MmrConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.mmr))
}

/// `PUT /api/config/mmr` -- save MMR reranking config.
pub async fn save_mmr_config(
    Json(body): Json<ConfigBody<ha_core::memory::MmrConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("memory_mmr", "http"), |store| {
        store.mmr = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/multimodal` -- get multimodal embedding config.
pub async fn get_multimodal_config() -> Result<Json<ha_core::memory::MultimodalConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.multimodal))
}

/// `PUT /api/config/multimodal` -- save multimodal embedding config.
pub async fn save_multimodal_config(
    Json(body): Json<ConfigBody<ha_core::memory::MultimodalConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("multimodal", "http"), |store| {
        store.multimodal = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/temporal-decay` -- get temporal decay config.
pub async fn get_temporal_decay_config(
) -> Result<Json<ha_core::memory::TemporalDecayConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.temporal_decay))
}

/// `PUT /api/config/temporal-decay` -- save temporal decay config.
pub async fn save_temporal_decay_config(
    Json(body): Json<ConfigBody<ha_core::memory::TemporalDecayConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("temporal_decay", "http"), |store| {
        store.temporal_decay = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/extract` -- get memory auto-extract config.
pub async fn get_extract_config() -> Result<Json<ha_core::memory::MemoryExtractConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.memory_extract))
}

/// `PUT /api/config/extract` -- save memory auto-extract config.
pub async fn save_extract_config(
    Json(body): Json<ConfigBody<ha_core::memory::MemoryExtractConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("memory_extract", "http"), |store| {
        store.memory_extract = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Web Fetch / Image Generate / Canvas ────────────────────────

/// `GET /api/config/web-fetch` -- get web fetch tool config.
pub async fn get_web_fetch_config(
) -> Result<Json<ha_core::tools::web_fetch::WebFetchConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.web_fetch))
}

/// `PUT /api/config/web-fetch` -- save web fetch tool config.
pub async fn save_web_fetch_config(
    Json(body): Json<ConfigBody<ha_core::tools::web_fetch::WebFetchConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("web_fetch", "http"), |store| {
        store.web_fetch = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/ssrf` -- get SSRF policy config.
pub async fn get_ssrf_config() -> Result<Json<ha_core::security::ssrf::SsrfConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.ssrf))
}

/// `PUT /api/config/ssrf` -- save SSRF policy config.
pub async fn save_ssrf_config(
    Json(body): Json<ConfigBody<ha_core::security::ssrf::SsrfConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("security.ssrf", "http"), |store| {
        store.ssrf = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/image-generate` -- get image generation config.
pub async fn get_image_generate_config(
) -> Result<Json<ha_core::tools::image_generate::ImageGenConfig>, AppError> {
    let store = load_config()?;
    let mut config = store.image_generate;
    ha_core::tools::image_generate::backfill_providers(&mut config);
    Ok(Json(config))
}

/// `PUT /api/config/image-generate` -- save image generation config.
pub async fn save_image_generate_config(
    Json(body): Json<ConfigBody<ha_core::tools::image_generate::ImageGenConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("image_generate", "http"), |store| {
        store.image_generate = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/canvas` -- get canvas tool config.
pub async fn get_canvas_config() -> Result<Json<ha_core::tools::canvas::CanvasConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.canvas))
}

/// `PUT /api/config/canvas` -- save canvas tool config.
pub async fn save_canvas_config(
    Json(body): Json<ConfigBody<ha_core::tools::canvas::CanvasConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("canvas", "http"), |store| {
        store.canvas = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

// ── Shortcuts ───────────────────────────────────────────────────

/// `GET /api/config/shortcuts` -- get global keyboard shortcut config.
pub async fn get_shortcut_config() -> Result<Json<ha_core::config::ShortcutConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.shortcuts))
}

/// `PUT /api/config/shortcuts` -- save global keyboard shortcut config.
///
/// Only persists the config — the actual OS-level shortcut registration is
/// performed by the Tauri desktop shell. In headless server mode this is a
/// no-op beyond saving the value.
pub async fn save_shortcut_config(
    Json(body): Json<ConfigBody<ha_core::config::ShortcutConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("shortcuts", "http"), |store| {
        store.shortcuts = body.config;
        Ok(())
    })?;
    Ok(Json(
        json!({ "saved": true, "note": "desktop-only registration" }),
    ))
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
    ha_core::config::mutate_config(("theme", "http"), |store| {
        store.theme = theme;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("language", "http"), |store| {
        store.language = language;
        Ok(())
    })?;
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
    ha_core::config::mutate_config(("ui_effects", "http"), |store| {
        store.ui_effects_enabled = enabled;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/config/tool-call-narration` -- get tool-call narration guidance toggle.
pub async fn get_tool_call_narration_enabled() -> Result<Json<Value>, AppError> {
    let store = load_config()?;
    Ok(Json(json!(store.tool_call_narration_enabled)))
}

/// `POST /api/config/tool-call-narration` -- set tool-call narration guidance toggle.
pub async fn set_tool_call_narration_enabled(
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    ha_core::config::mutate_config(("tool_call_narration", "http"), |store| {
        store.tool_call_narration_enabled = enabled;
        Ok(())
    })?;
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
pub async fn get_sandbox_config() -> Result<Json<ha_core::sandbox::SandboxConfig>, AppError> {
    Ok(Json(ha_core::sandbox::load_sandbox_config()?))
}

/// `PUT /api/config/sandbox` -- save Docker sandbox config.
pub async fn set_sandbox_config(
    Json(body): Json<ConfigBody<ha_core::sandbox::SandboxConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::sandbox::save_sandbox_config(&body.config)?;
    Ok(Json(json!({ "saved": true })))
}

// ── Behavior Awareness ──────────────────────────────────────────

/// `GET /api/config/awareness` -- global behavior awareness config.
pub async fn get_awareness_config() -> Result<Json<ha_core::awareness::AwarenessConfig>, AppError> {
    let store = load_config()?;
    Ok(Json(store.awareness))
}

/// `PUT /api/config/awareness` -- save global behavior awareness config.
pub async fn save_awareness_config(
    Json(body): Json<ConfigBody<ha_core::awareness::AwarenessConfig>>,
) -> Result<Json<Value>, AppError> {
    ha_core::config::mutate_config(("awareness", "http"), |store| {
        store.awareness = body.config;
        Ok(())
    })?;
    Ok(Json(json!({ "saved": true })))
}
