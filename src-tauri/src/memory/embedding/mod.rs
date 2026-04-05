mod api_provider;
pub mod config;
mod fallback_provider;
pub mod factory;
mod local_provider;
pub(crate) mod utils;

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original embedding.rs is re-exported here
// so that `crate::memory::embedding::XXX` and `crate::memory::XXX` continue to work.

pub use config::{
    EmbeddingConfig, EmbeddingPreset, EmbeddingProviderType, LocalEmbeddingModel,
    embedding_presets, list_local_models_with_status, local_embedding_models,
};
pub use factory::create_embedding_provider;

// Also re-export provider structs that were public
pub use api_provider::ApiEmbeddingProvider;
pub use local_provider::LocalEmbeddingProvider;
pub use fallback_provider::FallbackEmbeddingProvider;
