use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Embedding Config ────────────────────────────────────────────

/// Embedding provider type.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingProviderType {
    /// OpenAI /v1/embeddings compatible API (OpenAI, Jina, Cohere, SiliconFlow, etc.)
    #[default]
    OpenaiCompatible,
    /// Google Gemini Embedding API (different format)
    Google,
    /// Local ONNX model via fastembed-rs
    Local,
    /// Auto-select best available provider (local first, then reuse LLM API keys)
    Auto,
}

/// Embedding configuration, stored in AppConfig (config.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Reusable embedding model configuration managed from the model settings UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub provider_type: EmbeddingProviderType,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_model: Option<String>,
    #[serde(default)]
    pub api_dimensions: Option<u32>,
    #[serde(default)]
    pub source: Option<String>,
}

impl EmbeddingModelConfig {
    pub fn normalize_for_save(mut self) -> Self {
        if self.id.trim().is_empty() {
            self.id = format!("emb_{}", uuid::Uuid::new_v4().simple());
        }
        self.name = self.name.trim().to_string();
        self.api_base_url = self
            .api_base_url
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty());
        self.api_key = self
            .api_key
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        self.api_model = self
            .api_model
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        self.source = self
            .source
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if self.name.is_empty() {
            self.name = self.api_model.clone().unwrap_or_else(|| self.id.clone());
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(anyhow!("Embedding model config id is required"));
        }
        if self.name.trim().is_empty() {
            return Err(anyhow!("Embedding model config name is required"));
        }
        if self.api_base_url.as_deref().unwrap_or("").trim().is_empty() {
            return Err(anyhow!("Embedding API base URL is required"));
        }
        if self.api_model.as_deref().unwrap_or("").trim().is_empty() {
            return Err(anyhow!("Embedding model name is required"));
        }
        if matches!(
            self.provider_type,
            EmbeddingProviderType::Auto | EmbeddingProviderType::Local
        ) {
            return Err(anyhow!(
                "Auto/local embedding providers are no longer configurable"
            ));
        }
        Ok(())
    }

    pub fn to_runtime_config(&self, enabled: bool) -> EmbeddingConfig {
        EmbeddingConfig {
            enabled,
            provider_type: self.provider_type.clone(),
            api_base_url: self.api_base_url.clone(),
            api_key: self.api_key.clone(),
            api_model: self.api_model.clone(),
            api_dimensions: self.api_dimensions,
            local_model_id: None,
            fallback_provider_type: None,
            fallback_api_base_url: None,
            fallback_api_key: None,
            fallback_api_model: None,
            fallback_api_dimensions: None,
        }
    }

    pub fn signature(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", self.provider_type).to_ascii_lowercase());
        hasher.update(b"\n");
        hasher.update(
            self.api_base_url
                .as_deref()
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .to_ascii_lowercase(),
        );
        hasher.update(b"\n");
        hasher.update(self.api_model.as_deref().unwrap_or("").trim());
        hasher.update(b"\n");
        hasher.update(self.api_dimensions.unwrap_or_default().to_string());
        let digest = hasher.finalize();
        format!("{:x}", digest)
    }
}

/// Active memory embedding selection. The selected model config is resolved
/// into `EmbeddingConfig` only at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingSelection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model_config_id: Option<String>,
    #[serde(default)]
    pub active_signature: Option<String>,
    #[serde(default)]
    pub last_reembedded_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingState {
    pub selection: MemoryEmbeddingSelection,
    pub current_model: Option<EmbeddingModelConfig>,
    pub needs_reembed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingSetDefaultResult {
    pub state: MemoryEmbeddingState,
    pub reembedded: usize,
    pub reembed_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelTemplateModel {
    pub id: String,
    pub name: String,
    pub dimensions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelTemplate {
    pub name: String,
    pub provider_type: EmbeddingProviderType,
    pub base_url: String,
    pub default_model: String,
    pub default_dimensions: u32,
    #[serde(default)]
    pub models: Vec<EmbeddingModelTemplateModel>,
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

impl From<EmbeddingModelTemplate> for EmbeddingPreset {
    fn from(value: EmbeddingModelTemplate) -> Self {
        Self {
            name: value.name,
            provider_type: value.provider_type,
            base_url: value.base_url,
            default_model: value.default_model,
            default_dimensions: value.default_dimensions,
        }
    }
}

/// Return built-in API presets for the frontend.
pub fn embedding_presets() -> Vec<EmbeddingPreset> {
    embedding_model_templates()
        .into_iter()
        .map(EmbeddingPreset::from)
        .collect()
}

pub fn embedding_model_templates() -> Vec<EmbeddingModelTemplate> {
    fn model(id: &str, name: &str, dimensions: u32) -> EmbeddingModelTemplateModel {
        EmbeddingModelTemplateModel {
            id: id.to_string(),
            name: name.to_string(),
            dimensions,
        }
    }

    fn template(
        name: &str,
        provider_type: EmbeddingProviderType,
        base_url: &str,
        models: Vec<EmbeddingModelTemplateModel>,
    ) -> EmbeddingModelTemplate {
        let default = models
            .first()
            .expect("embedding model templates must include at least one model");
        EmbeddingModelTemplate {
            name: name.to_string(),
            provider_type,
            base_url: base_url.to_string(),
            default_model: default.id.clone(),
            default_dimensions: default.dimensions,
            models,
        }
    }

    vec![
        template(
            "OpenAI",
            EmbeddingProviderType::OpenaiCompatible,
            "https://api.openai.com",
            vec![
                model("text-embedding-3-small", "text-embedding-3-small", 1536),
                model("text-embedding-3-large", "text-embedding-3-large", 3072),
            ],
        ),
        template(
            "Google Gemini",
            EmbeddingProviderType::Google,
            "https://generativelanguage.googleapis.com",
            vec![
                model("gemini-embedding-2", "gemini-embedding-2", 3072),
                model("gemini-embedding-2", "gemini-embedding-2", 1536),
                model("gemini-embedding-2", "gemini-embedding-2", 768),
                model("gemini-embedding-001", "gemini-embedding-001", 768),
                model("gemini-embedding-001", "gemini-embedding-001", 1536),
                model("gemini-embedding-001", "gemini-embedding-001", 3072),
            ],
        ),
        template(
            "Jina AI",
            EmbeddingProviderType::OpenaiCompatible,
            "https://api.jina.ai",
            vec![
                model(
                    "jina-embeddings-v5-text-small",
                    "jina-embeddings-v5-text-small",
                    1024,
                ),
                model(
                    "jina-embeddings-v5-text-nano",
                    "jina-embeddings-v5-text-nano",
                    768,
                ),
                model("jina-embeddings-v4", "jina-embeddings-v4", 2048),
                model("jina-embeddings-v3", "jina-embeddings-v3", 1024),
            ],
        ),
        template(
            "Cohere",
            EmbeddingProviderType::OpenaiCompatible,
            "https://api.cohere.ai/compatibility",
            vec![
                model("embed-v4.0", "embed-v4.0", 1536),
                model("embed-multilingual-v3.0", "embed-multilingual-v3.0", 1024),
                model("embed-english-v3.0", "embed-english-v3.0", 1024),
            ],
        ),
        template(
            "SiliconFlow",
            EmbeddingProviderType::OpenaiCompatible,
            "https://api.siliconflow.cn",
            vec![
                model("BAAI/bge-m3", "BAAI/bge-m3", 1024),
                model(
                    "Qwen/Qwen3-Embedding-0.6B",
                    "Qwen/Qwen3-Embedding-0.6B",
                    1024,
                ),
                model("Qwen/Qwen3-Embedding-4B", "Qwen/Qwen3-Embedding-4B", 2560),
                model("Qwen/Qwen3-Embedding-8B", "Qwen/Qwen3-Embedding-8B", 4096),
            ],
        ),
        template(
            "Voyage AI",
            EmbeddingProviderType::OpenaiCompatible,
            "https://api.voyageai.com",
            vec![
                model("voyage-4-large", "voyage-4-large", 1024),
                model("voyage-4", "voyage-4", 1024),
                model("voyage-4-lite", "voyage-4-lite", 1024),
                model("voyage-code-3", "voyage-code-3", 1024),
                model("voyage-finance-2", "voyage-finance-2", 1024),
                model("voyage-law-2", "voyage-law-2", 1024),
            ],
        ),
        template(
            "Mistral",
            EmbeddingProviderType::OpenaiCompatible,
            "https://api.mistral.ai",
            vec![
                model("mistral-embed", "mistral-embed", 1024),
                model("codestral-embed", "codestral-embed", 3072),
            ],
        ),
        template(
            "Ollama",
            EmbeddingProviderType::OpenaiCompatible,
            "http://127.0.0.1:11434",
            vec![
                model("embeddinggemma:300m", "embeddinggemma:300m", 768),
                model("nomic-embed-text", "nomic-embed-text", 768),
                model("mxbai-embed-large", "mxbai-embed-large", 1024),
            ],
        ),
    ]
}

pub fn memory_embedding_state(
    selection: &MemoryEmbeddingSelection,
    models: &[EmbeddingModelConfig],
) -> MemoryEmbeddingState {
    let current_model = selection
        .model_config_id
        .as_ref()
        .and_then(|id| models.iter().find(|model| &model.id == id))
        .cloned();
    let active_signature = current_model.as_ref().map(EmbeddingModelConfig::signature);
    let needs_reembed = selection.enabled
        && active_signature.is_some()
        && active_signature != selection.last_reembedded_signature;
    MemoryEmbeddingState {
        selection: selection.clone(),
        current_model,
        needs_reembed,
    }
}

pub fn resolve_memory_embedding_config(
    selection: &MemoryEmbeddingSelection,
    models: &[EmbeddingModelConfig],
) -> Result<Option<(EmbeddingModelConfig, EmbeddingConfig, String)>> {
    if !selection.enabled {
        return Ok(None);
    }
    let Some(model_id) = selection.model_config_id.as_deref() else {
        return Ok(None);
    };
    let model = models
        .iter()
        .find(|model| model.id == model_id)
        .cloned()
        .ok_or_else(|| anyhow!("Embedding model config not found: {model_id}"))?;
    model.validate()?;
    let signature = model.signature();
    Ok(Some((
        model.clone(),
        model.to_runtime_config(true),
        signature,
    )))
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
