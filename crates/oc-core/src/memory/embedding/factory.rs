use anyhow::Result;
use std::sync::Arc;

use super::api_provider::ApiEmbeddingProvider;
use super::config::{EmbeddingConfig, EmbeddingProviderType};
use super::fallback_provider::FallbackEmbeddingProvider;
use super::local_provider::LocalEmbeddingProvider;
use crate::memory::traits::EmbeddingProvider;

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
            logger.log(
                "info",
                "memory",
                "embedding::auto",
                "Auto-selected local embedding provider (multilingual-e5-small)",
                None,
                None,
                None,
            );
        }
        return Ok(Arc::new(provider));
    }

    // Priority 20-50: Try API providers by reusing configured LLM API keys
    let store = crate::provider::load_store()
        .map_err(|e| anyhow::anyhow!("Failed to load provider store for auto-selection: {}", e))?;

    for candidate in AUTO_CANDIDATES {
        // Find a configured LLM provider whose base_url matches
        let matching_provider = store.providers.iter().find(|p| {
            p.enabled
                && !p.api_key.is_empty()
                && candidate
                    .url_patterns
                    .iter()
                    .any(|pat| p.base_url.contains(pat))
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
                        logger.log(
                            "info",
                            "memory",
                            "embedding::auto",
                            &format!(
                                "Auto-selected {} embedding provider (model={})",
                                candidate.base_url, candidate.model
                            ),
                            None,
                            None,
                            None,
                        );
                    }
                    return Ok(Arc::new(api_provider));
                }
                Err(e) => {
                    if let Some(logger) = crate::get_logger() {
                        logger.log(
                            "debug",
                            "memory",
                            "embedding::auto",
                            &format!("Skipping {} for auto-selection: {}", candidate.base_url, e),
                            None,
                            None,
                            None,
                        );
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
            let model_id = config
                .local_model_id
                .as_deref()
                .unwrap_or("bge-small-en-v1.5");
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
                        fallback.dimensions(),
                        primary.dimensions()
                    );
                }
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "info",
                        "memory",
                        "embedding::fallback",
                        "Fallback embedding provider configured",
                        None,
                        None,
                        None,
                    );
                }
                return Ok(Arc::new(FallbackEmbeddingProvider { primary, fallback }));
            }
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::fallback",
                        &format!(
                            "Failed to create fallback provider, continuing without: {}",
                            e
                        ),
                        None,
                        None,
                        None,
                    );
                }
            }
        }
    }

    Ok(primary)
}
