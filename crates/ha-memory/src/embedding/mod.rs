mod api_provider;
mod utils;

use std::sync::Arc;

use anyhow::Result;

use ha_core::memory::{EmbeddingConfig, EmbeddingProvider};

pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    Ok(Arc::new(api_provider::ApiEmbeddingProvider::new(config)?))
}
