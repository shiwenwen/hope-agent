use anyhow::Result;
use std::sync::Arc;
use std::sync::OnceLock;

use super::config::EmbeddingConfig;
use crate::memory::traits::EmbeddingProvider;

pub type EmbeddingFactory = fn(&EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>>;

static FACTORY: OnceLock<EmbeddingFactory> = OnceLock::new();

pub fn register_embedding_factory(
    factory: EmbeddingFactory,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    FACTORY
        .set(factory)
        .map_err(|_| crate::AlreadyRegistered("memory embedding factory"))
}

/// Create an EmbeddingProvider from EmbeddingConfig.
/// The HTTP implementation lives in `ha-memory`; missing feature wiring fails
/// explicitly because returning an empty embedder would corrupt retrieval.
pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    let factory = FACTORY
        .get()
        .ok_or_else(|| anyhow::anyhow!("memory embedding runtime is not wired"))?;
    factory(config)
}
