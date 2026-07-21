use anyhow::Result;
use std::sync::Arc;

use super::api_provider::ApiEmbeddingProvider;
use super::config::EmbeddingConfig;
use crate::memory::traits::EmbeddingProvider;

// ── Create provider from config ─────────────────────────────────

/// Create an EmbeddingProvider from EmbeddingConfig.
/// Safe to call from any thread; tokio-context panic regression is guarded
/// inside [`ApiEmbeddingProvider::new`].
pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    Ok(Arc::new(ApiEmbeddingProvider::new(config)?))
}
