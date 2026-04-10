use serde::{Deserialize, Serialize};

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

/// Embedding configuration, stored in AppConfig (config.json).
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
