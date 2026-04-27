use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::stream::{self, StreamExt};
use reqwest::StatusCode;
use rusqlite::{params, Connection, OptionalExtension};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::memory::{EmbeddingModelConfig, EmbeddingProviderType};
use crate::provider::{self, ModelConfig};
use crate::security::ssrf::{check_url, SsrfPolicy};

use super::{ensure_ollama_provider_with_model_config, start_ollama, OLLAMA_BASE_URL};

const OLLAMA_LIBRARY_ORIGIN: &str = "https://www.ollama.com";
const CACHE_TTL_SECS: i64 = 24 * 60 * 60;
const PROVIDER_SOURCE: &str = "local-llm-manager";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelDetails {
    #[serde(default)]
    pub parent_model: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub families: Option<Vec<String>>,
    #[serde(default)]
    pub parameter_size: Option<String>,
    #[serde(default)]
    pub quantization_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelUsage {
    pub active_model: bool,
    pub fallback_model: bool,
    pub provider_model: bool,
    pub embedding_config: bool,
    pub embedding_model: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_config_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOllamaModel {
    pub id: String,
    pub name: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub digest: Option<String>,
    pub details: Option<OllamaModelDetails>,
    pub context_window: Option<u32>,
    pub capabilities: Vec<String>,
    pub input_types: Vec<String>,
    pub running: bool,
    pub expires_at: Option<String>,
    pub size_vram_bytes: Option<u64>,
    pub usage: LocalModelUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaLibraryModel {
    pub name: String,
    pub href: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub sizes: Vec<String>,
    pub pull_count: Option<String>,
    pub tag_count: Option<u32>,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaLibraryTag {
    pub id: String,
    pub size_label: Option<String>,
    pub size_bytes: Option<u64>,
    pub context_label: Option<String>,
    pub context_window: Option<u32>,
    pub input_types: Vec<String>,
    pub digest: Option<String>,
    pub updated: Option<String>,
    pub cloud_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaLibrarySearchResponse {
    pub query: String,
    pub models: Vec<OllamaLibraryModel>,
    pub from_cache: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaLibraryModelDetail {
    pub model: OllamaLibraryModel,
    pub summary: String,
    pub downloads: Option<String>,
    pub updated: Option<String>,
    pub tags: Vec<OllamaLibraryTag>,
    pub from_cache: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelActionResult {
    pub ok: bool,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelRegistration {
    pub provider_id: Option<String>,
    pub model_id: String,
    pub registered_provider: bool,
    pub active_model: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelDeleteResult {
    pub deleted: bool,
    pub model_id: String,
    pub removed_provider_model: bool,
    pub removed_provider: bool,
    pub removed_active_model: bool,
    pub removed_fallback_models: usize,
    pub removed_embedding_model: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct TagModel {
    name: Option<String>,
    model: Option<String>,
    modified_at: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
    details: Option<OllamaModelDetails>,
}

#[derive(Debug, Clone, Deserialize)]
struct PsResponse {
    #[serde(default)]
    models: Vec<RunningModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct RunningModel {
    name: Option<String>,
    model: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
    details: Option<OllamaModelDetails>,
    expires_at: Option<String>,
    size_vram: Option<u64>,
    context_length: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct ShowModelResponse {
    #[serde(default)]
    details: Option<OllamaModelDetails>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    model_info: HashMap<String, Value>,
}

fn now_secs() -> i64 {
    Utc::now().timestamp()
}

fn ollama_client() -> Result<reqwest::Client> {
    crate::provider::apply_proxy_for_url(
        reqwest::Client::builder().timeout(Duration::from_secs(15)),
        OLLAMA_BASE_URL,
    )
    .build()
    .context("build Ollama client")
}

fn log_ollama_request_error(method: &str, path: &str, error: &reqwest::Error) {
    crate::app_warn!(
        "local_llm",
        "ollama_api",
        "Ollama {} {} request failed: {}",
        method,
        path,
        error
    );
}

fn log_ollama_status_error(method: &str, path: &str, status: StatusCode, body: &str) {
    crate::app_warn!(
        "local_llm",
        "ollama_api",
        "Ollama {} {} returned {}: {}",
        method,
        path,
        status,
        crate::truncate_utf8(body, 2048)
    );
}

fn log_ollama_parse_error(method: &str, path: &str, error: &reqwest::Error) {
    crate::app_warn!(
        "local_llm",
        "ollama_api",
        "Ollama {} {} response parse failed: {}",
        method,
        path,
        error
    );
}

async fn fetch_ollama_json<T>(path: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let resp = match ollama_client()?
        .get(format!("{OLLAMA_BASE_URL}{path}"))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            log_ollama_request_error("GET", path, &e);
            return Err(e).with_context(|| format!("GET {path}"));
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log_ollama_status_error("GET", path, status, &body);
        return Err(anyhow!("Ollama {path} returned {status}: {body}"));
    }
    match resp.json::<T>().await {
        Ok(value) => Ok(value),
        Err(e) => {
            log_ollama_parse_error("GET", path, &e);
            Err(e).with_context(|| format!("parse Ollama {path}"))
        }
    }
}

async fn post_ollama_json<T>(path: &str, body: Value) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let resp = match ollama_client()?
        .post(format!("{OLLAMA_BASE_URL}{path}"))
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            log_ollama_request_error("POST", path, &e);
            return Err(e).with_context(|| format!("POST {path}"));
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log_ollama_status_error("POST", path, status, &body);
        return Err(anyhow!("Ollama {path} returned {status}: {body}"));
    }
    match resp.json::<T>().await {
        Ok(value) => Ok(value),
        Err(e) => {
            log_ollama_parse_error("POST", path, &e);
            Err(e).with_context(|| format!("parse Ollama {path}"))
        }
    }
}

async fn show_ollama_model(model_id: &str) -> Result<ShowModelResponse> {
    post_ollama_json(
        "/api/show",
        serde_json::json!({ "model": model_id, "verbose": false }),
    )
    .await
}

fn context_window_from_show(show: &ShowModelResponse) -> Option<u32> {
    show.model_info.iter().find_map(|(key, value)| {
        if !key.ends_with(".context_length") && key != "context_length" {
            return None;
        }
        value.as_u64().and_then(|v| u32::try_from(v).ok())
    })
}

fn embedding_dimensions_from_show(show: &ShowModelResponse) -> Option<u32> {
    show.model_info.iter().find_map(|(key, value)| {
        if !key.ends_with(".embedding_length") && key != "embedding_length" {
            return None;
        }
        value.as_u64().and_then(|v| u32::try_from(v).ok())
    })
}

fn input_types_from_capabilities(capabilities: &[String]) -> Vec<String> {
    let lower: Vec<String> = capabilities
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let mut input = vec!["text".to_string()];
    if lower.iter().any(|c| c == "vision" || c == "image") {
        input.push("image".to_string());
    }
    input
}

fn completion_capable(capabilities: &[String]) -> bool {
    if capabilities.is_empty() {
        return true;
    }
    let has_completion = capabilities.iter().any(|c| {
        let c = c.to_ascii_lowercase();
        c == "completion" || c == "chat" || c == "tools" || c == "thinking" || c == "vision"
    });
    let embedding_only = capabilities
        .iter()
        .all(|c| c.eq_ignore_ascii_case("embedding"));
    has_completion && !embedding_only
}

fn embedding_capable(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|c| c.eq_ignore_ascii_case("embedding"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OllamaKeepAliveEndpoint {
    Generate,
    Embed,
}

impl OllamaKeepAliveEndpoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::Generate => "/api/generate",
            Self::Embed => "/api/embed",
        }
    }
}

fn keep_alive_endpoint_for_capabilities(capabilities: &[String]) -> OllamaKeepAliveEndpoint {
    if embedding_capable(capabilities) && !completion_capable(capabilities) {
        OllamaKeepAliveEndpoint::Embed
    } else {
        OllamaKeepAliveEndpoint::Generate
    }
}

fn keep_alive_request_body(
    endpoint: OllamaKeepAliveEndpoint,
    model_id: &str,
    keep_alive: i64,
) -> Value {
    match endpoint {
        OllamaKeepAliveEndpoint::Generate => serde_json::json!({
            "model": model_id,
            "prompt": "",
            "stream": false,
            "keep_alive": keep_alive
        }),
        OllamaKeepAliveEndpoint::Embed => serde_json::json!({
            "model": model_id,
            "input": "warmup",
            "keep_alive": keep_alive
        }),
    }
}

fn model_config_from_show(
    model_id: &str,
    display_name: Option<String>,
    show: &ShowModelResponse,
) -> ModelConfig {
    let context_window = context_window_from_show(show).unwrap_or(32_768);
    let reasoning = show
        .capabilities
        .iter()
        .any(|c| c.eq_ignore_ascii_case("thinking"))
        || model_id.contains("qwen3")
        || model_id.contains("deepseek-r1");
    ModelConfig {
        id: model_id.to_string(),
        name: display_name.unwrap_or_else(|| model_id.to_string()),
        input_types: input_types_from_capabilities(&show.capabilities),
        context_window,
        max_tokens: 8192,
        reasoning,
        thinking_style: None,
        cost_input: 0.0,
        cost_output: 0.0,
    }
}

pub async fn register_ollama_model_as_provider(
    model_id: &str,
    display_name: Option<String>,
    activate: bool,
) -> Result<OllamaModelRegistration> {
    crate::app_info!(
        "local_llm",
        "register_provider",
        "Register Ollama model as provider model: model={} activate={}",
        model_id,
        activate
    );
    let show = show_ollama_model(model_id).await?;
    if !completion_capable(&show.capabilities) {
        crate::app_warn!(
            "local_llm",
            "register_provider",
            "Skip Ollama provider registration for non-completion model: {} capabilities={:?}",
            model_id,
            show.capabilities
        );
        return Ok(OllamaModelRegistration {
            provider_id: None,
            model_id: model_id.to_string(),
            registered_provider: false,
            active_model: false,
        });
    }
    let model_cfg = model_config_from_show(model_id, display_name, &show);
    let (provider_id, model_id) = ensure_ollama_provider_with_model_config(model_cfg, activate)?;
    crate::app_info!(
        "local_llm",
        "register_provider",
        "Ollama provider model registered: provider={} model={} active={}",
        provider_id,
        model_id,
        activate
    );
    Ok(OllamaModelRegistration {
        provider_id: Some(provider_id),
        model_id,
        registered_provider: true,
        active_model: activate,
    })
}

pub async fn list_local_ollama_models() -> Result<Vec<LocalOllamaModel>> {
    if !super::ping_ollama().await {
        return Ok(Vec::new());
    }

    let tags: TagsResponse = fetch_ollama_json("/api/tags").await?;
    let ps: PsResponse = fetch_ollama_json("/api/ps")
        .await
        .unwrap_or(PsResponse { models: Vec::new() });
    let running: HashMap<String, RunningModel> = ps
        .models
        .into_iter()
        .filter_map(|m| m.model.clone().or(m.name.clone()).map(|id| (id, m)))
        .collect();
    let config = crate::config::cached_config();
    let usage_index = UsageIndex::build(&config);

    // Concurrent /api/show fan-out — Ollama serves these from a local index,
    // so the bottleneck is HTTP round-trips, not server CPU. Cap concurrency
    // at 8 to avoid pinning all reqwest connections to a single endpoint.
    let resolved: Vec<(TagModel, String, Option<ShowModelResponse>)> =
        stream::iter(tags.models.into_iter().filter_map(|tag| {
            tag.model
                .clone()
                .or_else(|| tag.name.clone())
                .map(|id| (tag, id))
        }))
        .map(|(tag, id)| async move {
            let show = show_ollama_model(&id).await.ok();
            (tag, id, show)
        })
        .buffered(8)
        .collect()
        .await;

    let mut models = Vec::with_capacity(resolved.len());
    for (tag, id, show) in resolved {
        let run = running.get(&id);
        let capabilities = show
            .as_ref()
            .map(|s| s.capabilities.clone())
            .unwrap_or_default();
        let details = show
            .as_ref()
            .and_then(|s| s.details.clone())
            .or_else(|| tag.details.clone())
            .or_else(|| run.and_then(|r| r.details.clone()));
        let context_window = show
            .as_ref()
            .and_then(context_window_from_show)
            .or_else(|| run.and_then(|r| r.context_length));
        let usage = usage_index.usage_for(&id, run.is_some());
        models.push(LocalOllamaModel {
            id: id.clone(),
            name: tag.name.unwrap_or(id),
            size_bytes: tag.size.or_else(|| run.and_then(|r| r.size)),
            modified_at: tag.modified_at,
            digest: tag.digest.or_else(|| run.and_then(|r| r.digest.clone())),
            details,
            context_window,
            input_types: input_types_from_capabilities(&capabilities),
            capabilities,
            running: run.is_some(),
            expires_at: run.and_then(|r| r.expires_at.clone()),
            size_vram_bytes: run.and_then(|r| r.size_vram),
            usage,
        });
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

struct UsageIndex<'a> {
    provider_id: Option<String>,
    provider_model_ids: HashSet<&'a str>,
    active_model_id: Option<&'a str>,
    fallback_model_ids: HashSet<&'a str>,
    embedding_models: HashMap<&'a str, &'a EmbeddingModelConfig>,
    memory_embedding_enabled: bool,
    active_embedding_model_id: Option<&'a str>,
}

impl<'a> UsageIndex<'a> {
    fn build(config: &'a crate::config::AppConfig) -> Self {
        let provider = config
            .providers
            .iter()
            .find(|p| provider::provider_matches_known_local_backend(p, "ollama"));
        let provider_id = provider.map(|p| p.id.clone());
        let provider_model_ids: HashSet<&str> = provider
            .map(|p| p.models.iter().map(|m| m.id.as_str()).collect())
            .unwrap_or_default();
        let active_model_id = config.active_model.as_ref().and_then(|m| {
            (provider_id.as_deref() == Some(m.provider_id.as_str())).then_some(m.model_id.as_str())
        });
        let fallback_model_ids: HashSet<&str> = config
            .fallback_models
            .iter()
            .filter(|m| provider_id.as_deref() == Some(m.provider_id.as_str()))
            .map(|m| m.model_id.as_str())
            .collect();
        let embedding_models: HashMap<&str, &EmbeddingModelConfig> = config
            .embedding_models
            .iter()
            .filter(|model| model.api_base_url.as_deref() == Some(OLLAMA_BASE_URL))
            .filter_map(|model| model.api_model.as_deref().map(|id| (id, model)))
            .collect();
        let active_embedding_model_id = config
            .memory_embedding
            .model_config_id
            .as_deref()
            .and_then(|id| config.embedding_models.iter().find(|m| m.id == id))
            .filter(|model| model.api_base_url.as_deref() == Some(OLLAMA_BASE_URL))
            .and_then(|model| model.api_model.as_deref());
        Self {
            provider_id,
            provider_model_ids,
            active_model_id,
            fallback_model_ids,
            embedding_models,
            memory_embedding_enabled: config.memory_embedding.enabled,
            active_embedding_model_id,
        }
    }

    fn usage_for(&self, model_id: &str, running: bool) -> LocalModelUsage {
        let matching_embedding_config = self.embedding_models.get(model_id).copied();
        LocalModelUsage {
            active_model: self.active_model_id == Some(model_id),
            fallback_model: self.fallback_model_ids.contains(model_id),
            provider_model: self.provider_model_ids.contains(model_id),
            embedding_config: matching_embedding_config.is_some(),
            embedding_model: self.memory_embedding_enabled
                && self.active_embedding_model_id == Some(model_id),
            running,
            provider_id: self.provider_id.clone(),
            embedding_config_id: matching_embedding_config.map(|m| m.id.clone()),
        }
    }
}

pub async fn preload_ollama_model(model_id: &str) -> Result<OllamaModelActionResult> {
    crate::app_info!(
        "local_llm",
        "preload",
        "Preload Ollama model requested: {}",
        model_id
    );
    start_ollama().await?;
    keep_alive_ollama_model(model_id, -1).await?;
    crate::app_info!(
        "local_llm",
        "preload",
        "Ollama model preloaded: {}",
        model_id
    );
    Ok(OllamaModelActionResult {
        ok: true,
        model_id: model_id.to_string(),
    })
}

pub async fn stop_ollama_model(model_id: &str) -> Result<OllamaModelActionResult> {
    crate::app_info!(
        "local_llm",
        "stop_model",
        "Stop Ollama model requested: {}",
        model_id
    );
    start_ollama().await?;
    keep_alive_ollama_model(model_id, 0).await?;
    crate::app_info!(
        "local_llm",
        "stop_model",
        "Ollama model stopped: {}",
        model_id
    );
    Ok(OllamaModelActionResult {
        ok: true,
        model_id: model_id.to_string(),
    })
}

async fn keep_alive_ollama_model(model_id: &str, keep_alive: i64) -> Result<()> {
    let show = show_ollama_model(model_id).await?;
    let endpoint = keep_alive_endpoint_for_capabilities(&show.capabilities);
    crate::app_info!(
        "local_llm",
        "keep_alive",
        "Apply Ollama keep_alive: model={} endpoint={} keep_alive={} capabilities={:?}",
        model_id,
        endpoint.as_str(),
        keep_alive,
        show.capabilities
    );
    match endpoint {
        OllamaKeepAliveEndpoint::Generate => {
            let _: Value = post_ollama_json(
                "/api/generate",
                keep_alive_request_body(OllamaKeepAliveEndpoint::Generate, model_id, keep_alive),
            )
            .await?;
        }
        OllamaKeepAliveEndpoint::Embed => {
            let _: Value = post_ollama_json(
                "/api/embed",
                keep_alive_request_body(OllamaKeepAliveEndpoint::Embed, model_id, keep_alive),
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn add_ollama_model_as_embedding_config(model_id: &str) -> Result<EmbeddingModelConfig> {
    crate::app_info!(
        "local_llm",
        "add_embedding_config",
        "Add Ollama model as embedding config requested: {}",
        model_id
    );
    start_ollama().await?;
    let show = show_ollama_model(model_id).await?;
    let config = EmbeddingModelConfig {
        id: crate::local_embedding::ollama_embedding_config_id(model_id),
        name: model_id.to_string(),
        provider_type: EmbeddingProviderType::OpenaiCompatible,
        api_base_url: Some(OLLAMA_BASE_URL.to_string()),
        api_key: Some("ollama".to_string()),
        api_model: Some(model_id.to_string()),
        api_dimensions: embedding_dimensions_from_show(&show),
        source: Some("ollama".to_string()),
    };
    let config = crate::memory::save_embedding_model_config(config, PROVIDER_SOURCE)?;
    crate::app_info!(
        "local_llm",
        "add_embedding_config",
        "Ollama embedding model config saved: model={} dimensions={:?}",
        model_id,
        config.api_dimensions
    );
    Ok(config)
}

pub async fn delete_ollama_model(model_id: &str) -> Result<LocalModelDeleteResult> {
    crate::app_info!(
        "local_llm",
        "delete_model",
        "Delete Ollama model requested: {}",
        model_id
    );
    start_ollama().await?;
    match keep_alive_ollama_model(model_id, 0).await {
        Ok(()) => {
            crate::app_info!(
                "local_llm",
                "delete_model",
                "Ollama model unloaded before delete: {}",
                model_id
            );
        }
        Err(e) => {
            crate::app_warn!(
                "local_llm",
                "delete_model",
                "Failed to unload Ollama model before delete, continuing with delete: model={} error={}",
                model_id,
                e
            );
        }
    }
    let resp = match ollama_client()?
        .delete(format!("{OLLAMA_BASE_URL}/api/delete"))
        .json(&serde_json::json!({ "model": model_id }))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            log_ollama_request_error("DELETE", "/api/delete", &e);
            return Err(e).context("DELETE /api/delete");
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status != StatusCode::NOT_FOUND {
            log_ollama_status_error("DELETE", "/api/delete", status, &body);
            return Err(anyhow!("Ollama /api/delete returned {status}: {body}"));
        }
        crate::app_info!(
            "local_llm",
            "delete_model",
            "Ollama model was already absent during delete: {}",
            model_id
        );
    }

    let removal = provider::remove_known_local_provider_model("ollama", model_id, PROVIDER_SOURCE)?;
    let removed_embedding_model = clear_embedding_model_if_matches(model_id)?;
    let result = LocalModelDeleteResult {
        deleted: true,
        model_id: model_id.to_string(),
        removed_provider_model: removal.removed_provider_model,
        removed_provider: removal.removed_provider,
        removed_active_model: removal.removed_active_model,
        removed_fallback_models: removal.removed_fallback_models,
        removed_embedding_model,
    };
    crate::app_info!(
        "local_llm",
        "delete_model",
        "Ollama model deleted: model={} removed_provider_model={} removed_provider={} removed_active_model={} removed_fallback_models={} removed_embedding_model={}",
        result.model_id,
        result.removed_provider_model,
        result.removed_provider,
        result.removed_active_model,
        result.removed_fallback_models,
        result.removed_embedding_model
    );
    Ok(result)
}

fn clear_embedding_model_if_matches(model_id: &str) -> Result<bool> {
    let mut removed_current = false;
    let model_id = model_id.to_string();
    crate::config::mutate_config(
        ("embedding_models.remove_ollama", PROVIDER_SOURCE),
        |store| {
            let removed_ids: std::collections::HashSet<String> = store
                .embedding_models
                .iter()
                .filter(|model| {
                    model.api_base_url.as_deref() == Some(OLLAMA_BASE_URL)
                        && model.api_model.as_deref() == Some(model_id.as_str())
                })
                .map(|model| model.id.clone())
                .collect();
            if removed_ids.is_empty() {
                return Ok(());
            }
            if store
                .memory_embedding
                .model_config_id
                .as_ref()
                .map(|id| removed_ids.contains(id))
                .unwrap_or(false)
            {
                removed_current = true;
                store.memory_embedding = crate::memory::MemoryEmbeddingSelection::default();
            }
            store
                .embedding_models
                .retain(|model| !removed_ids.contains(&model.id));
            Ok(())
        },
    )?;
    if removed_current {
        if let Some(backend) = crate::get_memory_backend() {
            backend.clear_embedder();
        }
    }
    Ok(removed_current)
}

fn library_cache_conn() -> Result<Connection> {
    let path = crate::paths::local_llm_library_cache_db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("open Ollama library cache at {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS ollama_library_cache (
             cache_key TEXT PRIMARY KEY,
             value_json TEXT NOT NULL,
             fetched_at INTEGER NOT NULL
         );",
    )?;
    Ok(conn)
}

fn read_cache<T>(key: &str) -> Result<Option<(T, i64)>>
where
    T: serde::de::DeserializeOwned,
{
    let conn = library_cache_conn()?;
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT value_json, fetched_at FROM ollama_library_cache WHERE cache_key=?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((json, fetched_at)) = row else {
        return Ok(None);
    };
    Ok(Some((serde_json::from_str(&json)?, fetched_at)))
}

fn write_cache<T>(key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let conn = library_cache_conn()?;
    conn.execute(
        "INSERT INTO ollama_library_cache(cache_key, value_json, fetched_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(cache_key) DO UPDATE SET
             value_json=excluded.value_json,
             fetched_at=excluded.fetched_at",
        params![key, serde_json::to_string(value)?, now_secs()],
    )?;
    Ok(())
}

async fn fetch_library_html(path_and_query: &str) -> Result<String> {
    let url = format!("{OLLAMA_LIBRARY_ORIGIN}{path_and_query}");
    let trusted = crate::config::cached_config().ssrf.trusted_hosts.clone();
    check_url(&url, SsrfPolicy::Default, &trusted)
        .await
        .with_context(|| format!("SSRF blocked {url}"))?;
    let client = crate::provider::apply_proxy_for_url(
        reqwest::Client::builder().timeout(Duration::from_secs(20)),
        &url,
    )
    .build()
    .context("build Ollama library client")?;
    client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()?
        .text()
        .await
        .context("read Ollama library HTML")
}

pub async fn search_ollama_library(query: &str) -> Result<OllamaLibrarySearchResponse> {
    let query = query.trim();
    let cache_key = format!("search:{query}");
    if let Some((models, fetched_at)) = read_cache::<Vec<OllamaLibraryModel>>(&cache_key)? {
        if now_secs() - fetched_at <= CACHE_TTL_SECS {
            return Ok(OllamaLibrarySearchResponse {
                query: query.to_string(),
                models,
                from_cache: true,
                stale: false,
            });
        }
    }

    let path = if query.is_empty() {
        "/search".to_string()
    } else {
        format!("/search?q={}", urlencoding::encode(query))
    };
    match fetch_library_html(&path).await {
        Ok(html) => {
            let models = parse_library_search_html(&html);
            write_cache(&cache_key, &models)?;
            Ok(OllamaLibrarySearchResponse {
                query: query.to_string(),
                models,
                from_cache: false,
                stale: false,
            })
        }
        Err(err) => {
            if let Some((models, _)) = read_cache::<Vec<OllamaLibraryModel>>(&cache_key)? {
                Ok(OllamaLibrarySearchResponse {
                    query: query.to_string(),
                    models,
                    from_cache: true,
                    stale: true,
                })
            } else {
                Err(err)
            }
        }
    }
}

pub async fn get_ollama_library_model(model: &str) -> Result<OllamaLibraryModelDetail> {
    let model = sanitize_library_model_name(model)?;
    let cache_key = format!("model:{model}");
    if let Some((detail, fetched_at)) = read_cache::<OllamaLibraryModelDetail>(&cache_key)? {
        if now_secs() - fetched_at <= CACHE_TTL_SECS {
            return Ok(OllamaLibraryModelDetail {
                from_cache: true,
                stale: false,
                ..detail
            });
        }
    }

    match fetch_library_html(&format!("/library/{model}/tags")).await {
        Ok(html) => {
            let mut detail = parse_library_model_html(&model, &html);
            detail.from_cache = false;
            detail.stale = false;
            write_cache(&cache_key, &detail)?;
            Ok(detail)
        }
        Err(err) => {
            if let Some((detail, _)) = read_cache::<OllamaLibraryModelDetail>(&cache_key)? {
                Ok(OllamaLibraryModelDetail {
                    from_cache: true,
                    stale: true,
                    ..detail
                })
            } else {
                Err(err)
            }
        }
    }
}

fn sanitize_library_model_name(model: &str) -> Result<String> {
    let trimmed = model.trim().trim_start_matches("/library/");
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(anyhow!("Invalid Ollama library model name: {model}"));
    }
    Ok(trimmed.to_string())
}

fn selector(s: &str) -> Selector {
    Selector::parse(s).unwrap_or_else(|_| panic!("invalid selector: {s}"))
}

fn element_text(el: ElementRef<'_>) -> String {
    el.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_u32_text(text: &str) -> Option<u32> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn parse_size_bytes(label: &str) -> Option<u64> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let number = lower
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .parse::<f64>()
        .ok()?;
    let multiplier = if lower.contains("tb") {
        1024_f64.powi(4)
    } else if lower.contains("gb") {
        1024_f64.powi(3)
    } else if lower.contains("mb") {
        1024_f64.powi(2)
    } else {
        1.0
    };
    Some((number * multiplier) as u64)
}

fn parse_context_window(label: &str) -> Option<u32> {
    let lower = label.to_ascii_lowercase();
    let number = lower
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .parse::<f64>()
        .ok()?;
    let multiplier = if lower.contains('k') { 1_000.0 } else { 1.0 };
    u32::try_from((number * multiplier) as u64).ok()
}

fn parse_library_search_html(html: &str) -> Vec<OllamaLibraryModel> {
    let doc = Html::parse_document(html);
    let li_sel = selector("li[x-test-model]");
    let title_sel = selector("[x-test-search-response-title]");
    let desc_sel = selector("p.max-w-lg");
    let cap_sel = selector("[x-test-capability]");
    let size_sel = selector("[x-test-size]");
    let pull_sel = selector("[x-test-pull-count]");
    let tag_sel = selector("[x-test-tag-count]");
    let updated_sel = selector("[x-test-updated]");
    let link_sel = selector("a[href^='/library/']");

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for li in doc.select(&li_sel) {
        let Some(title_el) = li.select(&title_sel).next() else {
            continue;
        };
        let name = element_text(title_el);
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        let href = li
            .select(&link_sel)
            .next()
            .and_then(|a| a.value().attr("href"))
            .unwrap_or("")
            .to_string();
        models.push(OllamaLibraryModel {
            name,
            href,
            description: li
                .select(&desc_sel)
                .next()
                .map(element_text)
                .unwrap_or_default(),
            capabilities: li.select(&cap_sel).map(element_text).collect(),
            sizes: li.select(&size_sel).map(element_text).collect(),
            pull_count: li.select(&pull_sel).next().map(element_text),
            tag_count: li
                .select(&tag_sel)
                .next()
                .and_then(|el| parse_u32_text(&element_text(el))),
            updated: li.select(&updated_sel).next().map(element_text),
        });
    }
    models
}

fn parse_library_model_html(model_name: &str, html: &str) -> OllamaLibraryModelDetail {
    let doc = Html::parse_document(html);
    let summary_sel = selector("#summary-content");
    let cap_sel = selector("span.inline-flex");
    let pull_sel = selector("[x-test-pull-count]");
    let updated_sel = selector("[x-test-updated]");
    let row_sel = selector("div.group.px-4.py-3");
    let link_sel = selector("a[href^='/library/']");
    let col_sel = selector("p.col-span-2");
    let input_sel = selector("div.col-span-2");
    let digest_sel = selector("span.font-mono");

    let summary = doc
        .select(&summary_sel)
        .next()
        .map(element_text)
        .unwrap_or_default();
    let capabilities: Vec<String> = doc
        .select(&cap_sel)
        .map(element_text)
        .filter(|t| {
            matches!(
                t.to_ascii_lowercase().as_str(),
                "cloud" | "embedding" | "vision" | "tools" | "thinking"
            )
        })
        .collect();
    let downloads = doc.select(&pull_sel).next().map(element_text);
    let updated = doc.select(&updated_sel).next().map(element_text);

    let mut tags = Vec::new();
    let mut seen = HashSet::new();
    for row in doc.select(&row_sel) {
        let tag_id = row.select(&link_sel).find_map(|a| {
            let text = element_text(a);
            if text.contains(':') {
                Some(text)
            } else {
                None
            }
        });
        let Some(tag_id) = tag_id else {
            continue;
        };
        if !seen.insert(tag_id.clone()) {
            continue;
        }
        let columns: Vec<String> = row.select(&col_sel).map(element_text).collect();
        let size_label = columns.first().cloned().filter(|v| v.trim() != "-");
        let context_label = columns.get(1).cloned();
        let input_text = row
            .select(&input_sel)
            .next()
            .map(element_text)
            .unwrap_or_default();
        let lower_input = input_text.to_ascii_lowercase();
        let mut input_types = Vec::new();
        if lower_input.contains("text") {
            input_types.push("text".to_string());
        }
        if lower_input.contains("image") {
            input_types.push("image".to_string());
        }
        let cloud_only = tag_id.ends_with("-cloud") || size_label.is_none();
        tags.push(OllamaLibraryTag {
            id: tag_id,
            size_bytes: size_label.as_deref().and_then(parse_size_bytes),
            size_label,
            context_window: context_label.as_deref().and_then(parse_context_window),
            context_label,
            input_types,
            digest: row.select(&digest_sel).next().map(element_text),
            updated: None,
            cloud_only,
        });
    }

    let sizes = tags
        .iter()
        .filter_map(|tag| tag.size_label.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    OllamaLibraryModelDetail {
        model: OllamaLibraryModel {
            name: model_name.to_string(),
            href: format!("/library/{model_name}"),
            description: summary.clone(),
            capabilities,
            sizes,
            pull_count: downloads.clone(),
            tag_count: u32::try_from(tags.len()).ok(),
            updated: updated.clone(),
        },
        summary,
        downloads,
        updated,
        tags,
        from_cache: false,
        stale: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_results() {
        let html = r#"
        <li x-test-model>
          <a href="/library/gemma3"><span x-test-search-response-title>gemma3</span></a>
          <p class="max-w-lg">A useful model.</p>
          <span x-test-capability>vision</span>
          <span x-test-size>4b</span>
          <span x-test-pull-count>1.2M</span>
          <span x-test-tag-count>29</span>
          <span x-test-updated>1 week ago</span>
        </li>"#;
        let models = parse_library_search_html(html);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "gemma3");
        assert_eq!(models[0].capabilities, vec!["vision"]);
        assert_eq!(models[0].tag_count, Some(29));
    }

    #[test]
    fn parses_library_tags_and_cloud_only_rows() {
        let html = r#"
        <span id="summary-content">Current Gemma.</span>
        <span class="inline-flex">vision</span>
        <span x-test-pull-count>36M</span>
        <div class="group px-4 py-3">
          <a href="/library/gemma3:27b">gemma3:27b</a>
          <p class="col-span-2 text-neutral-500">17GB</p>
          <p class="col-span-2 text-neutral-500">128K</p>
          <div class="col-span-2 text-neutral-500">Text, Image</div>
          <span class="font-mono">abc123</span>
        </div>
        <div class="group px-4 py-3">
          <a href="/library/gemma3:27b-cloud">gemma3:27b-cloud</a>
          <p class="col-span-2 text-neutral-500">-</p>
          <p class="col-span-2 text-neutral-500">128K</p>
          <div class="col-span-2 text-neutral-500">Text, Image</div>
          <span class="font-mono">def456</span>
        </div>"#;
        let detail = parse_library_model_html("gemma3", html);
        assert_eq!(detail.tags.len(), 2);
        assert_eq!(detail.tags[0].size_bytes, Some(17 * 1024 * 1024 * 1024));
        assert_eq!(detail.tags[0].context_window, Some(128_000));
        assert!(!detail.tags[0].cloud_only);
        assert!(detail.tags[1].cloud_only);
    }

    #[test]
    fn embedding_only_models_use_embed_keep_alive_endpoint() {
        assert_eq!(
            keep_alive_endpoint_for_capabilities(&["embedding".into()]),
            OllamaKeepAliveEndpoint::Embed
        );
        assert_eq!(
            keep_alive_endpoint_for_capabilities(&["completion".into()]),
            OllamaKeepAliveEndpoint::Generate
        );
        assert_eq!(
            keep_alive_endpoint_for_capabilities(&["embedding".into(), "completion".into()]),
            OllamaKeepAliveEndpoint::Generate
        );
        assert_eq!(
            keep_alive_endpoint_for_capabilities(&[]),
            OllamaKeepAliveEndpoint::Generate
        );
    }

    #[test]
    fn keep_alive_body_uses_numeric_duration() {
        let body =
            keep_alive_request_body(OllamaKeepAliveEndpoint::Embed, "embeddinggemma:300m", -1);
        assert_eq!(body["keep_alive"].as_i64(), Some(-1));
        assert!(body["keep_alive"].as_str().is_none());

        let body = keep_alive_request_body(OllamaKeepAliveEndpoint::Generate, "qwen3.6:27b", 0);
        assert_eq!(body["keep_alive"].as_i64(), Some(0));
        assert!(body["keep_alive"].as_str().is_none());
    }
}
