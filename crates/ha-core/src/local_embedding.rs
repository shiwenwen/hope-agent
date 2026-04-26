//! Local embedding helper for Ollama-backed vector search.
//!
//! Ollama exposes `/v1/embeddings`, so local embedding models are stored as
//! reusable OpenAI-compatible embedding model configs.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::local_llm::{
    detect_ollama_version, list_ollama_model_names, pull_model_cancellable, PullProgress,
    OLLAMA_BASE_URL,
};
use crate::memory::{
    EmbeddingConfig, EmbeddingModelConfig, EmbeddingProviderType, MemoryEmbeddingSetDefaultResult,
};
use tokio_util::sync::CancellationToken;

const PROVIDER_SOURCE: &str = "local-embedding-wizard";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaEmbeddingModel {
    pub id: String,
    pub display_name: String,
    pub dimensions: u32,
    pub size_mb: u64,
    pub context_window: u32,
    pub languages: Vec<String>,
    pub min_ollama_version: Option<String>,
    pub installed: bool,
    pub recommended: bool,
}

/// Small, high-quality Ollama embedding models suitable for memory search.
pub fn embedding_model_catalog() -> Vec<OllamaEmbeddingModel> {
    vec![
        OllamaEmbeddingModel {
            id: "embeddinggemma:300m".into(),
            display_name: "EmbeddingGemma 300M".into(),
            dimensions: 768,
            size_mb: 622,
            context_window: 2_048,
            languages: vec!["100+ languages".into(), "code".into()],
            min_ollama_version: Some("0.11.10".into()),
            installed: false,
            recommended: true,
        },
        OllamaEmbeddingModel {
            id: "qwen3-embedding:0.6b".into(),
            display_name: "Qwen3 Embedding 0.6B".into(),
            dimensions: 1_024,
            size_mb: 639,
            context_window: 32_768,
            languages: vec!["100+ languages".into(), "code".into()],
            min_ollama_version: None,
            installed: false,
            recommended: false,
        },
        OllamaEmbeddingModel {
            id: "nomic-embed-text:v1.5".into(),
            display_name: "Nomic Embed Text v1.5".into(),
            dimensions: 768,
            size_mb: 274,
            context_window: 8_192,
            languages: vec!["en".into()],
            min_ollama_version: Some("0.1.26".into()),
            installed: false,
            recommended: false,
        },
        OllamaEmbeddingModel {
            id: "all-minilm:22m".into(),
            display_name: "All MiniLM 22M".into(),
            dimensions: 384,
            size_mb: 46,
            context_window: 512,
            languages: vec!["en".into()],
            min_ollama_version: Some("0.1.26".into()),
            installed: false,
            recommended: false,
        },
    ]
}

pub async fn list_models_with_status() -> Vec<OllamaEmbeddingModel> {
    let installed = list_ollama_model_names().await.unwrap_or_default();
    let installed: std::collections::HashSet<String> = installed.into_iter().collect();
    embedding_model_catalog()
        .into_iter()
        .map(|mut model| {
            model.installed = installed.contains(&model.id);
            model
        })
        .collect()
}

pub fn embedding_config_for_model(model: &OllamaEmbeddingModel) -> EmbeddingConfig {
    EmbeddingConfig {
        enabled: true,
        provider_type: EmbeddingProviderType::OpenaiCompatible,
        api_base_url: Some(OLLAMA_BASE_URL.to_string()),
        api_key: Some("ollama".to_string()),
        api_model: Some(model.id.clone()),
        api_dimensions: Some(model.dimensions),
        local_model_id: None,
        fallback_provider_type: None,
        fallback_api_base_url: None,
        fallback_api_key: None,
        fallback_api_model: None,
        fallback_api_dimensions: None,
    }
}

pub fn embedding_model_config_for_model(model: &OllamaEmbeddingModel) -> EmbeddingModelConfig {
    EmbeddingModelConfig {
        id: ollama_embedding_config_id(&model.id),
        name: model.display_name.clone(),
        provider_type: EmbeddingProviderType::OpenaiCompatible,
        api_base_url: Some(OLLAMA_BASE_URL.to_string()),
        api_key: Some("ollama".to_string()),
        api_model: Some(model.id.clone()),
        api_dimensions: Some(model.dimensions),
        source: Some("ollama".to_string()),
    }
}

pub fn ollama_embedding_config_id(model_id: &str) -> String {
    let sanitized: String = model_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("ollama-{sanitized}")
}

pub fn resolve_catalog_model(model_id: &str) -> Result<OllamaEmbeddingModel> {
    embedding_model_catalog()
        .into_iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| anyhow!("Unsupported Ollama embedding model: {model_id}"))
}

pub fn ollama_version_meets_min(version: &str, minimum: &str) -> bool {
    fn parse(v: &str) -> Vec<u32> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .take(3)
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    }

    let mut current = parse(version);
    let mut required = parse(minimum);
    current.resize(3, 0);
    required.resize(3, 0);
    current >= required
}

pub async fn ensure_version_compatible(model: &OllamaEmbeddingModel) -> Result<()> {
    let Some(minimum) = model.min_ollama_version.as_deref() else {
        return Ok(());
    };
    let version = match detect_ollama_version().await {
        Ok(Some(version)) => version,
        Ok(None) | Err(_) => return Ok(()),
    };
    if !ollama_version_meets_min(&version, minimum) {
        return Err(anyhow!(
            "{} requires Ollama {minimum}+; installed version is {version}",
            model.display_name
        ));
    }
    Ok(())
}

pub fn save_embedding_config_for_model(model: &OllamaEmbeddingModel) -> Result<EmbeddingConfig> {
    let model_config = save_embedding_model_config_for_model(model)?;
    let config = model_config.to_runtime_config(true);
    app_info!(
        "memory",
        "local_embedding",
        "Ollama embedding model config saved for model {} ({}d)",
        model.id,
        model.dimensions
    );
    Ok(config)
}

pub fn save_embedding_model_config_for_model(
    model: &OllamaEmbeddingModel,
) -> Result<EmbeddingModelConfig> {
    crate::memory::save_embedding_model_config(
        embedding_model_config_for_model(model),
        PROVIDER_SOURCE,
    )
}

pub fn save_and_set_default_for_model(
    model: &OllamaEmbeddingModel,
) -> Result<MemoryEmbeddingSetDefaultResult> {
    let config = save_embedding_model_config_for_model(model)?;
    crate::memory::set_memory_embedding_default(&config.id, true, PROVIDER_SOURCE)
}

pub async fn pull_and_activate_cancellable<F>(
    requested: OllamaEmbeddingModel,
    on_progress: F,
    cancel_token: CancellationToken,
) -> Result<MemoryEmbeddingSetDefaultResult>
where
    F: Fn(&PullProgress) + Send + Sync + 'static,
{
    let model = resolve_catalog_model(&requested.id)?;
    ensure_version_compatible(&model).await?;

    let on_progress = std::sync::Arc::new(on_progress);
    let cb = on_progress.clone();
    pull_model_cancellable(&model.id, move |p| cb(p), cancel_token).await?;

    on_progress(&PullProgress {
        model_id: model.id.clone(),
        phase: "configure-embedding".into(),
        percent: Some(99),
        bytes_completed: None,
        bytes_total: None,
    });
    let result = save_and_set_default_for_model(&model)?;
    on_progress(&PullProgress {
        model_id: model.id.clone(),
        phase: "done".into(),
        percent: Some(100),
        bytes_completed: None,
        bytes_total: None,
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keeps_recommended_model_first() {
        let catalog = embedding_model_catalog();
        assert_eq!(
            catalog.first().map(|m| m.id.as_str()),
            Some("embeddinggemma:300m")
        );
        assert!(catalog.first().map(|m| m.recommended).unwrap_or(false));
        assert!(catalog[1..]
            .windows(2)
            .all(|w| w[0].size_mb >= w[1].size_mb));
    }

    #[test]
    fn compares_ollama_versions() {
        assert!(ollama_version_meets_min("0.11.10", "0.11.10"));
        assert!(ollama_version_meets_min("0.12.6", "0.11.10"));
        assert!(ollama_version_meets_min("v0.11.10", "0.11.9"));
        assert!(!ollama_version_meets_min("0.11.9", "0.11.10"));
        assert!(!ollama_version_meets_min("0.1.25", "0.1.26"));
    }

    #[test]
    fn builds_openai_compatible_embedding_config() {
        let model = resolve_catalog_model("embeddinggemma:300m").expect("model");
        let config = embedding_config_for_model(&model);
        assert!(config.enabled);
        assert_eq!(
            config.provider_type,
            EmbeddingProviderType::OpenaiCompatible
        );
        assert_eq!(config.api_base_url.as_deref(), Some(OLLAMA_BASE_URL));
        assert_eq!(config.api_key.as_deref(), Some("ollama"));
        assert_eq!(config.api_model.as_deref(), Some("embeddinggemma:300m"));
        assert_eq!(config.api_dimensions, Some(768));
    }
}
