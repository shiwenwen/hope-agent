use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use super::traits::EmbeddingProvider;

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

        Ok(Self {
            client: reqwest::blocking::Client::new(),
            base_url,
            api_key,
            model,
            dimensions,
            provider_type: config.provider_type.clone(),
        })
    }

    fn call_openai_compatible(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        // Some APIs support specifying dimensions
        if self.dimensions > 0 {
            body["dimensions"] = serde_json::json!(self.dimensions);
        }

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call embedding API")?;

        let status = resp.status();
        let resp_text = resp.text()?;

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

    fn call_google(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::new();
        for text in texts {
            let url = format!(
                "{}/v1beta/models/{}:embedContent?key={}",
                self.base_url.trim_end_matches('/'),
                self.model,
                self.api_key,
            );

            let mut body = serde_json::json!({
                "content": {
                    "parts": [{"text": text}]
                }
            });

            if self.dimensions > 0 {
                body["outputDimensionality"] = serde_json::json!(self.dimensions);
            }

            let resp = self.client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .context("Failed to call Google embedding API")?;

            let status = resp.status();
            let resp_text = resp.text()?;

            if !status.is_success() {
                anyhow::bail!("Google Embedding API error {}: {}", status, resp_text);
            }

            let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
            let values = resp_json["embedding"]["values"].as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid Google embedding response"))?;

            let embedding: Vec<f32> = values.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }
        Ok(results)
    }
}

impl EmbeddingProvider for ApiEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(&[text.to_string()])?,
            _ => self.call_openai_compatible(&[text.to_string()])?,
        };
        results.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(texts),
            _ => self.call_openai_compatible(texts),
        }
    }

    fn dimensions(&self) -> u32 {
        self.dimensions
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
        results.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        model.embed(texts.to_vec(), None)
            .map_err(|e| anyhow::anyhow!("Local batch embedding failed: {}", e))
    }

    fn dimensions(&self) -> u32 {
        self.dims
    }
}

// ── Create provider from config ─────────────────────────────────

/// Create an EmbeddingProvider from EmbeddingConfig.
pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.provider_type {
        EmbeddingProviderType::Local => {
            let model_id = config.local_model_id.as_deref().unwrap_or("bge-small-en-v1.5");
            Ok(Arc::new(LocalEmbeddingProvider::new(model_id)?))
        }
        _ => {
            Ok(Arc::new(ApiEmbeddingProvider::new(config)?))
        }
    }
}
