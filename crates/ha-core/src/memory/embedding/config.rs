use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

// 类型已下沉 ha-config-schema（`EmbeddingConfig` 随 `EmbeddingModelConfig` 的
// inherent impl 一并下沉）；模板 / 状态投影与解析函数留在本文件。
pub use ha_config_schema::memory::embedding::{
    embedding_endpoint_family, EmbeddingConfig, EmbeddingEndpointFamily, EmbeddingModelConfig,
    EmbeddingProviderType, EmbeddingPurpose, EmbeddingSelection,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSelectionState {
    pub selection: EmbeddingSelection,
    pub current_model: Option<EmbeddingModelConfig>,
    pub needs_reembed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSetDefaultResult {
    pub state: EmbeddingSelectionState,
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

/// Derive the UI-facing state (selected model + `needsReembed`) for an embedding
/// selection. Subsystem-agnostic — the historical `memory_` name predates the
/// knowledge split; pass either `memory_embedding` or `knowledge_embedding`.
pub fn memory_embedding_state(
    selection: &EmbeddingSelection,
    models: &[EmbeddingModelConfig],
) -> EmbeddingSelectionState {
    // selection.model_config_id 在 disabled 状态下被 disable_memory_embedding
    // 保留（pause 语义），但 current_model 字段必须是 None——否则 UI 会显示
    // 「memoryActive」徽标，与已 clear 的 embedder 实际状态冲突。
    let current_model = if selection.enabled {
        selection
            .model_config_id
            .as_ref()
            .and_then(|id| models.iter().find(|model| &model.id == id))
            .cloned()
    } else {
        None
    };
    let active_signature = current_model.as_ref().map(EmbeddingModelConfig::signature);
    let needs_reembed = selection.enabled
        && active_signature.is_some()
        && active_signature != selection.last_reembedded_signature;
    EmbeddingSelectionState {
        selection: selection.clone(),
        current_model,
        needs_reembed,
    }
}

/// Resolve a selection into `(model, runtime config, signature)`. Subsystem-
/// agnostic despite the `memory_` name (predates the knowledge split); pass
/// either `memory_embedding` or `knowledge_embedding`.
pub fn resolve_memory_embedding_config(
    selection: &EmbeddingSelection,
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

/// Active document signature for an embedding selection. Always recompute from
/// the live model config so a signature/provider-semantics upgrade immediately
/// makes old vectors ineligible; a persisted v1 signature must never reinterpret
/// them as v2. `None` when disabled or unresolved. Shared single source of truth for memory
/// (`active_embedding_signature`) and knowledge
/// (`knowledge_active_embedding_signature`) — pass the respective selection.
pub fn active_signature_for(
    selection: &EmbeddingSelection,
    models: &[EmbeddingModelConfig],
) -> Option<String> {
    if !selection.enabled {
        return None;
    }
    resolve_memory_embedding_config(selection, models)
        .ok()
        .flatten()
        .map(|(_, _, signature)| signature)
}
