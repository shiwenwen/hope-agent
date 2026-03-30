use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use super::traits::{EmbeddingProvider, MultimodalInput};

// ── Embedding Config ────────────────────────────────────────────

/// Embedding provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingProviderType {
    /// OpenAI /v1/embeddings compatible API (OpenAI, Jina, Cohere, SiliconFlow, etc.)
    OpenaiCompatible,
    /// Google Gemini Embedding API (different format)
    Google,
    /// Local ONNX model via fastembed-rs
    Local,
    /// Auto-select best available provider (local first, then reuse LLM API keys)
    Auto,
}

impl Default for EmbeddingProviderType {
    fn default() -> Self {
        EmbeddingProviderType::OpenaiCompatible
    }
}

/// Embedding configuration, stored in ProviderStore (config.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// Whether embedding (vector search) is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Provider type
    #[serde(default)]
    pub provider_type: EmbeddingProviderType,

    // ── API mode fields ──

    /// API Base URL (e.g. "https://api.openai.com")
    #[serde(default)]
    pub api_base_url: Option<String>,

    /// API Key
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model name (e.g. "text-embedding-3-small")
    #[serde(default)]
    pub api_model: Option<String>,

    /// Output dimensions (some APIs support specifying this)
    #[serde(default)]
    pub api_dimensions: Option<u32>,

    // ── Local mode fields ──

    /// Local model ID (e.g. "bge-small-en-v1.5")
    #[serde(default)]
    pub local_model_id: Option<String>,

    // ── Fallback provider fields ──

    /// Fallback provider type (used when primary fails)
    #[serde(default)]
    pub fallback_provider_type: Option<EmbeddingProviderType>,

    /// Fallback API Base URL
    #[serde(default)]
    pub fallback_api_base_url: Option<String>,

    /// Fallback API Key
    #[serde(default)]
    pub fallback_api_key: Option<String>,

    /// Fallback Model name
    #[serde(default)]
    pub fallback_api_model: Option<String>,

    /// Fallback Output dimensions
    #[serde(default)]
    pub fallback_api_dimensions: Option<u32>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_type: EmbeddingProviderType::default(),
            api_base_url: None,
            api_key: None,
            api_model: None,
            api_dimensions: None,
            local_model_id: None,
            fallback_provider_type: None,
            fallback_api_base_url: None,
            fallback_api_key: None,
            fallback_api_model: None,
            fallback_api_dimensions: None,
        }
    }
}

/// Local embedding model definition (built-in presets).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEmbeddingModel {
    pub id: String,
    pub name: String,
    pub dimensions: u32,
    pub size_mb: u32,
    pub min_ram_gb: u32,
    pub languages: Vec<String>,
    pub downloaded: bool,
}

/// API preset template for frontend dropdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingPreset {
    pub name: String,
    pub provider_type: EmbeddingProviderType,
    pub base_url: String,
    pub default_model: String,
    pub default_dimensions: u32,
}

/// Return built-in API presets for the frontend.
pub fn embedding_presets() -> Vec<EmbeddingPreset> {
    vec![
        EmbeddingPreset {
            name: "OpenAI".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.openai.com".to_string(),
            default_model: "text-embedding-3-small".to_string(),
            default_dimensions: 1536,
        },
        EmbeddingPreset {
            name: "Google Gemini".to_string(),
            provider_type: EmbeddingProviderType::Google,
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            default_model: "gemini-embedding-001".to_string(),
            default_dimensions: 768,
        },
        EmbeddingPreset {
            name: "Jina AI".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.jina.ai".to_string(),
            default_model: "jina-embeddings-v3".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "Cohere".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.cohere.com".to_string(),
            default_model: "embed-multilingual-v3.0".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "SiliconFlow".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.siliconflow.cn".to_string(),
            default_model: "BAAI/bge-m3".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "Voyage AI".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.voyageai.com".to_string(),
            default_model: "voyage-3".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "Mistral".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "https://api.mistral.ai".to_string(),
            default_model: "mistral-embed".to_string(),
            default_dimensions: 1024,
        },
        EmbeddingPreset {
            name: "Ollama".to_string(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            base_url: "http://localhost:11434".to_string(),
            default_model: "nomic-embed-text".to_string(),
            default_dimensions: 768,
        },
    ]
}

/// Return built-in local model presets.
pub fn local_embedding_models() -> Vec<LocalEmbeddingModel> {
    vec![
        LocalEmbeddingModel {
            id: "bge-small-en-v1.5".to_string(),
            name: "BGE Small English v1.5".to_string(),
            dimensions: 384,
            size_mb: 33,
            min_ram_gb: 4,
            languages: vec!["en".to_string()],
            downloaded: false, // filled at runtime
        },
        LocalEmbeddingModel {
            id: "bge-small-zh-v1.5".to_string(),
            name: "BGE Small Chinese v1.5".to_string(),
            dimensions: 384,
            size_mb: 33,
            min_ram_gb: 4,
            languages: vec!["zh".to_string()],
            downloaded: false,
        },
        LocalEmbeddingModel {
            id: "multilingual-e5-small".to_string(),
            name: "Multilingual E5 Small".to_string(),
            dimensions: 384,
            size_mb: 90,
            min_ram_gb: 8,
            languages: vec!["multilingual".to_string()],
            downloaded: false,
        },
        LocalEmbeddingModel {
            id: "bge-large-en-v1.5".to_string(),
            name: "BGE Large English v1.5".to_string(),
            dimensions: 1024,
            size_mb: 335,
            min_ram_gb: 16,
            languages: vec!["en".to_string()],
            downloaded: false,
        },
    ]
}

/// Check which local models are downloaded.
pub fn list_local_models_with_status() -> Vec<LocalEmbeddingModel> {
    let cache_dir = crate::paths::models_cache_dir().unwrap_or_default();
    let mut models = local_embedding_models();
    for model in &mut models {
        let model_dir = cache_dir.join(&model.id);
        model.downloaded = model_dir.exists() && model_dir.is_dir();
    }
    models
}

// ── Token Limit Management ──────────────────────────────────────

/// Maximum input tokens for known embedding models.
fn max_input_tokens(model: &str) -> usize {
    match model {
        "text-embedding-3-small" | "text-embedding-3-large" | "text-embedding-ada-002" => 8191,
        "gemini-embedding-001" | "text-embedding-004" => 2048,
        "gemini-embedding-2-preview" => 8192,
        "voyage-3" | "voyage-code-3" | "voyage-4-large" => 32000,
        "voyage-3-lite" => 16000,
        "mistral-embed" => 8192,
        "jina-embeddings-v3" => 8192,
        "embed-multilingual-v3.0" => 512,
        "nomic-embed-text" => 8192,
        m if m.starts_with("BAAI/bge") => 512,
        _ => 8192, // safe default
    }
}

/// Truncate texts that exceed the model's token limit (conservative: ~4 bytes/token).
fn truncate_for_model(texts: &[String], model: &str) -> Vec<String> {
    let max_bytes = max_input_tokens(model) * 4;
    texts.iter().map(|t| {
        if t.len() > max_bytes {
            if let Some(logger) = crate::get_logger() {
                logger.log("warn", "memory", "embedding::truncate",
                    &format!("Truncating text from {} to {} bytes for model {}", t.len(), max_bytes, model),
                    None, None, None);
            }
            crate::truncate_utf8(t, max_bytes).to_string()
        } else {
            t.clone()
        }
    }).collect()
}

// ── L2 Vector Normalization ─────────────────────────────────────

/// L2-normalize an embedding vector in place for consistent cosine similarity.
fn l2_normalize(vec: &mut Vec<f32>) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

// ── API Embedding Provider ───────────────────────────────────────

/// OpenAI-compatible /v1/embeddings API provider.
pub struct ApiEmbeddingProvider {
    client: reqwest::blocking::Client,
    base_url: String,
    api_key: String,
    model: String,
    dimensions: u32,
    provider_type: EmbeddingProviderType,
}

impl ApiEmbeddingProvider {
    pub fn new(config: &EmbeddingConfig) -> Result<Self> {
        let base_url = config.api_base_url.as_deref().unwrap_or("https://api.openai.com").to_string();
        let api_key = config.api_key.as_deref().unwrap_or("").to_string();
        let model = config.api_model.as_deref().unwrap_or("text-embedding-3-small").to_string();
        let dimensions = config.api_dimensions.unwrap_or(1536);

        let client = crate::provider::apply_proxy_blocking(
            reqwest::blocking::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
        )
            .build()
            .context("Failed to build embedding HTTP client")?;

        Ok(Self {
            client,
            base_url,
            api_key,
            model,
            dimensions,
            provider_type: config.provider_type.clone(),
        })
    }

    fn call_openai_compatible(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let texts = truncate_for_model(texts, &self.model);
        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": self.model,
            "input": &texts,
        });

        // Some APIs support specifying dimensions
        if self.dimensions > 0 {
            body["dimensions"] = serde_json::json!(self.dimensions);
        }

        // Voyage AI asymmetric embedding: query (single text search) vs document (batch indexing)
        if self.base_url.contains("voyageai.com") {
            body["input_type"] = serde_json::json!(if texts.len() == 1 { "query" } else { "document" });
        }

        // Log embedding API request
        if let Some(logger) = crate::get_logger() {
            let body_str = serde_json::to_string(&body).unwrap_or_default();
            let body_size = body_str.len();
            let body_preview = if body_size > 4096 {
                format!("{}...(truncated, total {}B)", crate::truncate_utf8(&body_str, 4096), body_size)
            } else {
                body_str
            };
            logger.log("debug", "memory", "embedding::openai_compatible::request",
                &format!("Embedding API request: {} texts, model={}, url={}, body {}B", texts.len(), self.model, url, body_size),
                Some(serde_json::json!({
                    "api_url": &url,
                    "model": &self.model,
                    "text_count": texts.len(),
                    "dimensions": self.dimensions,
                    "body_size_bytes": body_size,
                    "request_body": body_preview,
                }).to_string()),
                None, None);
        }

        let request_start = std::time::Instant::now();
        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        // Log embedding API response
        if let Some(logger) = crate::get_logger() {
            let resp_preview = if resp_text.len() > 2048 {
                format!("{}...(truncated, total {}B)", crate::truncate_utf8(&resp_text, 2048), resp_text.len())
            } else {
                resp_text.clone()
            };
            let level = if status.is_success() { "debug" } else { "error" };
            logger.log(level, "memory", "embedding::openai_compatible::response",
                &format!("Embedding API response: status={}, ttfb={}ms, body {}B", status.as_u16(), ttfb_ms, resp_text.len()),
                Some(serde_json::json!({
                    "status": status.as_u16(),
                    "ttfb_ms": ttfb_ms,
                    "response_size_bytes": resp_text.len(),
                    "response_body": resp_preview,
                }).to_string()),
                None, None);
        }

        if !status.is_success() {
            anyhow::bail!("Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let data = resp_json["data"].as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding API response"))?;

        let mut results = Vec::new();
        for item in data {
            let embedding = item["embedding"].as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing embedding in response"))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }

        Ok(results)
    }

    /// Batch embed via Google Gemini `batchEmbedContents` API (up to 100 texts per request).
    /// Falls back to single `embedContent` if batch fails.
    fn call_google(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let texts = truncate_for_model(texts, &self.model);
        const BATCH_SIZE: usize = 100; // Gemini batch limit

        let mut all_results = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(BATCH_SIZE) {
            match self.call_google_batch(chunk) {
                Ok(mut batch_results) => {
                    all_results.append(&mut batch_results);
                }
                Err(batch_err) => {
                    // Fallback: single embedContent per text
                    if let Some(logger) = crate::get_logger() {
                        logger.log("warn", "memory", "embedding::google::batch_fallback",
                            &format!("Batch embedContent failed, falling back to single requests: {}", batch_err),
                            None, None, None);
                    }
                    for text in chunk {
                        let result = self.call_google_single(text)?;
                        all_results.push(result);
                    }
                }
            }
        }

        Ok(all_results)
    }

    /// Batch embed via `batchEmbedContents` endpoint.
    fn call_google_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!(
            "{}/v1beta/models/{}:batchEmbedContents?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let model_path = format!("models/{}", self.model);
        let requests: Vec<serde_json::Value> = texts.iter().map(|text| {
            let mut req = serde_json::json!({
                "model": &model_path,
                "content": { "parts": [{"text": text}] }
            });
            if self.dimensions > 0 {
                req["outputDimensionality"] = serde_json::json!(self.dimensions);
            }
            req
        }).collect();

        let body = serde_json::json!({ "requests": requests });

        // Log batch request
        if let Some(logger) = crate::get_logger() {
            let safe_url = format!(
                "{}/v1beta/models/{}:batchEmbedContents?key=[REDACTED]",
                self.base_url.trim_end_matches('/'),
                self.model,
            );
            logger.log("debug", "memory", "embedding::google::batch_request",
                &format!("Google Batch Embedding API: {} texts, model={}", texts.len(), self.model),
                Some(serde_json::json!({
                    "api_url": safe_url,
                    "model": &self.model,
                    "text_count": texts.len(),
                    "dimensions": self.dimensions,
                }).to_string()),
                None, None);
        }

        let request_start = std::time::Instant::now();
        let resp = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call Google batch embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        // Log batch response
        if let Some(logger) = crate::get_logger() {
            let resp_preview = if resp_text.len() > 2048 {
                format!("{}...(truncated, total {}B)", crate::truncate_utf8(&resp_text, 2048), resp_text.len())
            } else {
                resp_text.clone()
            };
            let level = if status.is_success() { "debug" } else { "error" };
            logger.log(level, "memory", "embedding::google::batch_response",
                &format!("Google Batch Embedding API response: status={}, ttfb={}ms, body {}B", status.as_u16(), ttfb_ms, resp_text.len()),
                Some(serde_json::json!({
                    "status": status.as_u16(),
                    "ttfb_ms": ttfb_ms,
                    "text_count": texts.len(),
                    "response_size_bytes": resp_text.len(),
                    "response_body": resp_preview,
                }).to_string()),
                None, None);
        }

        if !status.is_success() {
            anyhow::bail!("Google Batch Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let embeddings = resp_json["embeddings"].as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google batch embedding response: missing 'embeddings' array"))?;

        let mut results = Vec::with_capacity(embeddings.len());
        for emb in embeddings {
            let values = emb["values"].as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid Google batch embedding response: missing 'values'"))?;
            let embedding: Vec<f32> = values.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }

        Ok(results)
    }

    /// Single text embed via `embedContent` endpoint (fallback).
    fn call_google_single(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let mut body = serde_json::json!({
            "content": { "parts": [{"text": text}] }
        });
        if self.dimensions > 0 {
            body["outputDimensionality"] = serde_json::json!(self.dimensions);
        }

        if let Some(logger) = crate::get_logger() {
            let text_preview = if text.len() > 200 {
                format!("{}...", crate::truncate_utf8(text, 200))
            } else {
                text.to_string()
            };
            let safe_url = format!(
                "{}/v1beta/models/{}:embedContent?key=[REDACTED]",
                self.base_url.trim_end_matches('/'),
                self.model,
            );
            logger.log("debug", "memory", "embedding::google::single_request",
                &format!("Google Embedding API single request: model={}, text_len={}", self.model, text.len()),
                Some(serde_json::json!({
                    "api_url": safe_url,
                    "model": &self.model,
                    "text_length": text.len(),
                    "text_preview": text_preview,
                    "dimensions": self.dimensions,
                }).to_string()),
                None, None);
        }

        let request_start = std::time::Instant::now();
        let resp = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call Google embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        if let Some(logger) = crate::get_logger() {
            let resp_preview = if resp_text.len() > 2048 {
                format!("{}...(truncated, total {}B)", crate::truncate_utf8(&resp_text, 2048), resp_text.len())
            } else {
                resp_text.clone()
            };
            let level = if status.is_success() { "debug" } else { "error" };
            logger.log(level, "memory", "embedding::google::single_response",
                &format!("Google Embedding API single response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
                Some(serde_json::json!({
                    "status": status.as_u16(),
                    "ttfb_ms": ttfb_ms,
                    "response_size_bytes": resp_text.len(),
                    "response_body": resp_preview,
                }).to_string()),
                None, None);
        }

        if !status.is_success() {
            anyhow::bail!("Google Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let values = resp_json["embedding"]["values"].as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google embedding response"))?;

        Ok(values.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
    }

    /// Multimodal embed via Gemini `embedContent` with inline data (image/audio).
    /// Only works with gemini-embedding-2-preview.
    fn call_google_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        use base64::Engine;

        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let b64_data = base64::engine::general_purpose::STANDARD.encode(&input.file_data);

        let mut body = serde_json::json!({
            "content": {
                "parts": [
                    { "text": &input.label },
                    { "inlineData": {
                        "mimeType": &input.mime_type,
                        "data": &b64_data,
                    }}
                ]
            }
        });
        if self.dimensions > 0 {
            body["outputDimensionality"] = serde_json::json!(self.dimensions);
        }

        if let Some(logger) = crate::get_logger() {
            let safe_url = format!(
                "{}/v1beta/models/{}:embedContent?key=[REDACTED]",
                self.base_url.trim_end_matches('/'),
                self.model,
            );
            logger.log("info", "memory", "embedding::google::multimodal_request",
                &format!("Multimodal embedding: model={}, mime={}, file_size={}B, label={}", self.model, input.mime_type, input.file_data.len(), crate::truncate_utf8(&input.label, 100)),
                Some(serde_json::json!({
                    "api_url": safe_url,
                    "model": &self.model,
                    "mime_type": &input.mime_type,
                    "file_size_bytes": input.file_data.len(),
                    "base64_size_bytes": b64_data.len(),
                }).to_string()),
                None, None);
        }

        let request_start = std::time::Instant::now();
        let resp = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call Google multimodal embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        if let Some(logger) = crate::get_logger() {
            let level = if status.is_success() { "info" } else { "error" };
            logger.log(level, "memory", "embedding::google::multimodal_response",
                &format!("Multimodal embedding response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
                None, None, None);
        }

        if !status.is_success() {
            anyhow::bail!("Google Multimodal Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let values = resp_json["embedding"]["values"].as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google multimodal embedding response"))?;

        Ok(values.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
    }

    // ── Async Batch API (OpenAI / Voyage compatible) ──

    /// Check if this provider supports the async Batch API.
    fn batch_api_supported(&self) -> bool {
        match self.provider_type {
            EmbeddingProviderType::OpenaiCompatible => {
                // OpenAI and Voyage support Batch API
                self.base_url.contains("openai.com") || self.base_url.contains("voyageai.com")
            }
            _ => false, // Gemini uses batchEmbedContents (already synchronous batch)
        }
    }

    /// Upload a JSONL file for batch processing.
    fn batch_upload_jsonl(&self, jsonl_content: &str) -> Result<String> {
        let url = format!("{}/v1/files", self.base_url.trim_end_matches('/'));

        let boundary = format!("----BatchBoundary{}", chrono::Utc::now().timestamp_millis());
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nbatch\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"memory-embeddings.jsonl\"\r\nContent-Type: application/jsonl\r\n\r\n{jsonl_content}\r\n--{boundary}--\r\n",
        );

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", format!("multipart/form-data; boundary={}", boundary))
            .body(body)
            .send()
            .context("Failed to upload batch JSONL file")?;

        let status = resp.status();
        let resp_text = resp.text()?;
        if !status.is_success() {
            anyhow::bail!("Batch file upload error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        resp_json["id"].as_str()
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Missing file id in upload response"))
    }

    /// Create a batch job.
    fn batch_create(&self, input_file_id: &str) -> Result<String> {
        let url = format!("{}/v1/batches", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "input_file_id": input_file_id,
            "endpoint": "/v1/embeddings",
            "completion_window": "24h",
        });

        // Voyage needs request_params
        if self.base_url.contains("voyageai.com") {
            body["completion_window"] = serde_json::json!("12h");
            body["request_params"] = serde_json::json!({
                "model": &self.model,
                "input_type": "document",
            });
        }

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to create batch job")?;

        let status = resp.status();
        let resp_text = resp.text()?;
        if !status.is_success() {
            anyhow::bail!("Batch create error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        resp_json["id"].as_str()
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Missing batch id in create response"))
    }

    /// Poll batch status until completion or failure.
    fn batch_poll(&self, batch_id: &str, timeout_ms: u64, poll_interval_ms: u64) -> Result<String> {
        let url = format!("{}/v1/batches/{}", self.base_url.trim_end_matches('/'), batch_id);
        let start = std::time::Instant::now();

        loop {
            let resp = self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .send()
                .context("Failed to poll batch status")?;

            let resp_text = resp.text()?;
            let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
            let state = resp_json["status"].as_str().unwrap_or("unknown");

            match state {
                "completed" => {
                    return resp_json["output_file_id"].as_str()
                        .map(String::from)
                        .ok_or_else(|| anyhow::anyhow!("Batch completed but no output_file_id"));
                }
                "failed" | "expired" | "cancelled" | "canceled" => {
                    anyhow::bail!("Batch {} {}: {}", batch_id, state,
                        resp_json["error"].as_str().unwrap_or("unknown error"));
                }
                _ => {
                    if start.elapsed().as_millis() as u64 > timeout_ms {
                        anyhow::bail!("Batch {} timed out after {}ms (state: {})", batch_id, timeout_ms, state);
                    }
                    if let Some(logger) = crate::get_logger() {
                        logger.log("debug", "memory", "embedding::batch_poll",
                            &format!("Batch {} state={}, waiting {}ms", batch_id, state, poll_interval_ms),
                            None, None, None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(poll_interval_ms));
                }
            }
        }
    }

    /// Download batch output file content (JSONL).
    fn batch_download_output(&self, file_id: &str) -> Result<String> {
        let url = format!("{}/v1/files/{}/content", self.base_url.trim_end_matches('/'), file_id);

        let resp = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .context("Failed to download batch output")?;

        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            anyhow::bail!("Batch output download error {}: {}", status, text);
        }
        Ok(text)
    }

    /// Run the complete async Batch API flow: upload JSONL → create batch → poll → download → parse.
    fn run_batch_api(&self, items: &[(String, String)]) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        use std::collections::HashMap;
        const MAX_BATCH_SIZE: usize = 50_000;
        const POLL_INTERVAL_MS: u64 = 5_000;
        const TIMEOUT_MS: u64 = 60 * 60 * 1_000; // 60 minutes

        if let Some(logger) = crate::get_logger() {
            logger.log("info", "memory", "embedding::batch_api",
                &format!("Starting async Batch API: {} items, model={}", items.len(), self.model),
                None, None, None);
        }

        let mut all_results: HashMap<String, Vec<f32>> = HashMap::new();

        for chunk in items.chunks(MAX_BATCH_SIZE) {
            // Build JSONL
            let jsonl: String = chunk.iter().map(|(id, text)| {
                let mut body = serde_json::json!({
                    "model": &self.model,
                    "input": text,
                });
                if self.dimensions > 0 {
                    body["dimensions"] = serde_json::json!(self.dimensions);
                }
                serde_json::json!({
                    "custom_id": id,
                    "method": "POST",
                    "url": "/v1/embeddings",
                    "body": body,
                }).to_string()
            }).collect::<Vec<_>>().join("\n");

            // Upload → Create → Poll → Download
            let file_id = self.batch_upload_jsonl(&jsonl)?;
            if let Some(logger) = crate::get_logger() {
                logger.log("info", "memory", "embedding::batch_api",
                    &format!("Batch JSONL uploaded: file_id={}, {} items", file_id, chunk.len()),
                    None, None, None);
            }

            let batch_id = self.batch_create(&file_id)?;
            if let Some(logger) = crate::get_logger() {
                logger.log("info", "memory", "embedding::batch_api",
                    &format!("Batch created: batch_id={}", batch_id),
                    None, None, None);
            }

            let output_file_id = self.batch_poll(&batch_id, TIMEOUT_MS, POLL_INTERVAL_MS)?;
            let output = self.batch_download_output(&output_file_id)?;

            // Parse JSONL output
            for line in output.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
                let parsed: serde_json::Value = serde_json::from_str(line)
                    .with_context(|| format!("Invalid batch output line: {}", crate::truncate_utf8(line, 200)))?;

                let custom_id = parsed["custom_id"].as_str().unwrap_or("").to_string();
                if custom_id.is_empty() { continue; }

                let status_code = parsed["response"]["status_code"].as_u64().unwrap_or(0);
                if status_code >= 400 {
                    let err_msg = parsed["response"]["body"]["error"]["message"]
                        .as_str().unwrap_or("unknown error");
                    if let Some(logger) = crate::get_logger() {
                        logger.log("warn", "memory", "embedding::batch_api",
                            &format!("Batch item {} failed: {}", custom_id, err_msg),
                            None, None, None);
                    }
                    continue;
                }

                if let Some(data) = parsed["response"]["body"]["data"].as_array() {
                    if let Some(first) = data.first() {
                        if let Some(emb_arr) = first["embedding"].as_array() {
                            let mut emb: Vec<f32> = emb_arr.iter()
                                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                                .collect();
                            l2_normalize(&mut emb);
                            all_results.insert(custom_id, emb);
                        }
                    }
                }
            }
        }

        if let Some(logger) = crate::get_logger() {
            logger.log("info", "memory", "embedding::batch_api",
                &format!("Batch API completed: {}/{} embeddings generated", all_results.len(), items.len()),
                None, None, None);
        }

        Ok(all_results)
    }
}

impl EmbeddingProvider for ApiEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(&[text.to_string()])?,
            _ => self.call_openai_compatible(&[text.to_string()])?,
        };
        let mut vec = results.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(texts)?,
            _ => self.call_openai_compatible(texts)?,
        };
        for vec in &mut results {
            l2_normalize(vec);
        }
        Ok(results)
    }

    fn dimensions(&self) -> u32 {
        self.dimensions
    }

    fn supports_multimodal(&self) -> bool {
        matches!(self.provider_type, EmbeddingProviderType::Google)
            && self.model.contains("embedding-2")
    }

    fn embed_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        if !self.supports_multimodal() {
            return self.embed(&input.label);
        }
        let mut vec = self.call_google_multimodal(input)?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn supports_batch_api(&self) -> bool {
        self.batch_api_supported()
    }

    fn embed_batch_async(&self, texts: &[(String, String)]) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        if !self.batch_api_supported() {
            // Fallback to synchronous
            let text_strs: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
            let results = self.embed_batch(&text_strs)?;
            let mut map = std::collections::HashMap::new();
            for ((id, _), emb) in texts.iter().zip(results) {
                map.insert(id.clone(), emb);
            }
            return Ok(map);
        }
        self.run_batch_api(texts)
    }
}

// ── Local Embedding Provider ────────────────────────────────────

/// Local ONNX-based embedding provider using fastembed-rs.
pub struct LocalEmbeddingProvider {
    model: Mutex<fastembed::TextEmbedding>,
    dims: u32,
}

impl LocalEmbeddingProvider {
    /// Initialize with a model ID from the built-in presets.
    pub fn new(model_id: &str) -> Result<Self> {
        let (fe_model, dims) = match model_id {
            "bge-small-zh-v1.5" => (fastembed::EmbeddingModel::BGESmallZHV15, 384),
            "multilingual-e5-small" => (fastembed::EmbeddingModel::MultilingualE5Small, 384),
            "bge-large-en-v1.5" => (fastembed::EmbeddingModel::BGELargeENV15, 1024),
            _ => (fastembed::EmbeddingModel::BGESmallENV15, 384), // default
        };

        let cache_dir = crate::paths::models_cache_dir()?;

        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(fe_model)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(false),
        ).context("Failed to initialize local embedding model")?;

        Ok(Self { model: Mutex::new(model), dims })
    }
}

impl EmbeddingProvider for LocalEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let results = model.embed(vec![text.to_string()], None)
            .map_err(|e| anyhow::anyhow!("Local embedding failed: {}", e))?;
        let mut vec = results.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut results = model.embed(texts.to_vec(), None)
            .map_err(|e| anyhow::anyhow!("Local batch embedding failed: {}", e))?;
        for vec in &mut results {
            l2_normalize(vec);
        }
        Ok(results)
    }

    fn dimensions(&self) -> u32 {
        self.dims
    }
}

// ── Fallback Embedding Provider ─────────────────────────────────

/// Provider wrapper that falls back to a secondary provider on error.
pub struct FallbackEmbeddingProvider {
    primary: Arc<dyn EmbeddingProvider>,
    fallback: Arc<dyn EmbeddingProvider>,
}

impl EmbeddingProvider for FallbackEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        match self.primary.embed(text) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log("warn", "memory", "embedding::fallback",
                        &format!("Primary embed failed, trying fallback: {}", e),
                        None, None, None);
                }
                self.fallback.embed(text)
            }
        }
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self.primary.embed_batch(texts) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log("warn", "memory", "embedding::fallback",
                        &format!("Primary embed_batch failed, trying fallback: {}", e),
                        None, None, None);
                }
                self.fallback.embed_batch(texts)
            }
        }
    }

    fn dimensions(&self) -> u32 {
        self.primary.dimensions()
    }

    fn supports_multimodal(&self) -> bool {
        self.primary.supports_multimodal()
    }

    fn embed_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        match self.primary.embed_multimodal(input) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log("warn", "memory", "embedding::fallback",
                        &format!("Primary embed_multimodal failed, trying fallback: {}", e),
                        None, None, None);
                }
                self.fallback.embed_multimodal(input)
            }
        }
    }

    fn supports_batch_api(&self) -> bool {
        self.primary.supports_batch_api()
    }

    fn embed_batch_async(&self, texts: &[(String, String)]) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        match self.primary.embed_batch_async(texts) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log("warn", "memory", "embedding::fallback",
                        &format!("Primary embed_batch_async failed, trying fallback: {}", e),
                        None, None, None);
                }
                self.fallback.embed_batch_async(texts)
            }
        }
    }
}

// ── Auto-selection Logic ────────────────────────────────────────

/// Auto-selection provider priority definitions.
struct AutoCandidate {
    provider_type: EmbeddingProviderType,
    base_url: &'static str,
    model: &'static str,
    dimensions: u32,
    /// URL patterns to match against configured LLM provider base_url
    url_patterns: &'static [&'static str],
}

const AUTO_CANDIDATES: &[AutoCandidate] = &[
    // Priority 20: OpenAI
    AutoCandidate {
        provider_type: EmbeddingProviderType::OpenaiCompatible,
        base_url: "https://api.openai.com",
        model: "text-embedding-3-small",
        dimensions: 1536,
        url_patterns: &["openai.com"],
    },
    // Priority 30: Google Gemini
    AutoCandidate {
        provider_type: EmbeddingProviderType::Google,
        base_url: "https://generativelanguage.googleapis.com",
        model: "gemini-embedding-001",
        dimensions: 768,
        url_patterns: &["googleapis.com", "generativelanguage"],
    },
    // Priority 40: Voyage AI
    AutoCandidate {
        provider_type: EmbeddingProviderType::OpenaiCompatible,
        base_url: "https://api.voyageai.com",
        model: "voyage-3",
        dimensions: 1024,
        url_patterns: &["voyageai.com"],
    },
    // Priority 50: Mistral
    AutoCandidate {
        provider_type: EmbeddingProviderType::OpenaiCompatible,
        base_url: "https://api.mistral.ai",
        model: "mistral-embed",
        dimensions: 1024,
        url_patterns: &["mistral.ai"],
    },
];

/// Try to auto-select an embedding provider by checking available API keys.
fn create_auto_provider() -> Result<Arc<dyn EmbeddingProvider>> {
    // Priority 10: Try local model first (no API key needed)
    if let Ok(provider) = LocalEmbeddingProvider::new("multilingual-e5-small") {
        if let Some(logger) = crate::get_logger() {
            logger.log("info", "memory", "embedding::auto",
                "Auto-selected local embedding provider (multilingual-e5-small)",
                None, None, None);
        }
        return Ok(Arc::new(provider));
    }

    // Priority 20-50: Try API providers by reusing configured LLM API keys
    let store = crate::provider::load_store()
        .map_err(|e| anyhow::anyhow!("Failed to load provider store for auto-selection: {}", e))?;

    for candidate in AUTO_CANDIDATES {
        // Find a configured LLM provider whose base_url matches
        let matching_provider = store.providers.iter().find(|p| {
            p.enabled && !p.api_key.is_empty() &&
            candidate.url_patterns.iter().any(|pat| p.base_url.contains(pat))
        });

        if let Some(provider) = matching_provider {
            let config = EmbeddingConfig {
                enabled: true,
                provider_type: candidate.provider_type.clone(),
                api_base_url: Some(candidate.base_url.to_string()),
                api_key: Some(provider.api_key.clone()),
                api_model: Some(candidate.model.to_string()),
                api_dimensions: Some(candidate.dimensions),
                local_model_id: None,
                fallback_provider_type: None,
                fallback_api_base_url: None,
                fallback_api_key: None,
                fallback_api_model: None,
                fallback_api_dimensions: None,
            };
            match ApiEmbeddingProvider::new(&config) {
                Ok(api_provider) => {
                    if let Some(logger) = crate::get_logger() {
                        logger.log("info", "memory", "embedding::auto",
                            &format!("Auto-selected {} embedding provider (model={})", candidate.base_url, candidate.model),
                            None, None, None);
                    }
                    return Ok(Arc::new(api_provider));
                }
                Err(e) => {
                    if let Some(logger) = crate::get_logger() {
                        logger.log("debug", "memory", "embedding::auto",
                            &format!("Skipping {} for auto-selection: {}", candidate.base_url, e),
                            None, None, None);
                    }
                }
            }
        }
    }

    anyhow::bail!("No embedding provider available for auto-selection (no local model or matching API keys found)")
}

// ── Create provider from config ─────────────────────────────────

/// Create a single EmbeddingProvider (without fallback wrapping).
fn create_single_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.provider_type {
        EmbeddingProviderType::Auto => create_auto_provider(),
        EmbeddingProviderType::Local => {
            let model_id = config.local_model_id.as_deref().unwrap_or("bge-small-en-v1.5");
            Ok(Arc::new(LocalEmbeddingProvider::new(model_id)?))
        }
        _ => Ok(Arc::new(ApiEmbeddingProvider::new(config)?)),
    }
}

/// Create an EmbeddingProvider from EmbeddingConfig, with optional fallback.
pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    let primary = create_single_provider(config)?;

    // Wrap with fallback if configured
    if let Some(ref fb_type) = config.fallback_provider_type {
        let fb_config = EmbeddingConfig {
            enabled: true,
            provider_type: fb_type.clone(),
            api_base_url: config.fallback_api_base_url.clone(),
            api_key: config.fallback_api_key.clone(),
            api_model: config.fallback_api_model.clone(),
            api_dimensions: config.fallback_api_dimensions,
            local_model_id: config.local_model_id.clone(),
            fallback_provider_type: None,
            fallback_api_base_url: None,
            fallback_api_key: None,
            fallback_api_model: None,
            fallback_api_dimensions: None,
        };
        match create_single_provider(&fb_config) {
            Ok(fallback) => {
                if fallback.dimensions() != primary.dimensions() {
                    anyhow::bail!(
                        "Fallback embedding dimensions ({}) != primary ({}). Both must match.",
                        fallback.dimensions(), primary.dimensions()
                    );
                }
                if let Some(logger) = crate::get_logger() {
                    logger.log("info", "memory", "embedding::fallback",
                        "Fallback embedding provider configured",
                        None, None, None);
                }
                return Ok(Arc::new(FallbackEmbeddingProvider { primary, fallback }));
            }
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log("warn", "memory", "embedding::fallback",
                        &format!("Failed to create fallback provider, continuing without: {}", e),
                        None, None, None);
                }
            }
        }
    }

    Ok(primary)
}
