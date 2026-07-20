use anyhow::Result;
use std::sync::Arc;

use super::api_provider::ApiEmbeddingProvider;
use super::config::EmbeddingConfig;
use super::fallback_provider::FallbackEmbeddingProvider;
use crate::memory::traits::EmbeddingProvider;

// ── Create provider from config ─────────────────────────────────

/// Create a single EmbeddingProvider (without fallback wrapping).
fn create_single_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    Ok(Arc::new(ApiEmbeddingProvider::new(config)?))
}

/// Create an EmbeddingProvider from EmbeddingConfig, with optional fallback.
/// Safe to call from any thread; tokio-context panic regression is guarded
/// inside [`ApiEmbeddingProvider::new`].
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
