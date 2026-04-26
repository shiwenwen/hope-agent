mod api_provider;
pub mod config;
pub mod factory;
mod fallback_provider;
mod local_provider;
pub(crate) mod utils;

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original embedding.rs is re-exported here
// so that `crate::memory::embedding::XXX` and `crate::memory::XXX` continue to work.

pub use config::{
    embedding_model_templates, embedding_presets, list_local_models_with_status,
    local_embedding_models, memory_embedding_state, resolve_memory_embedding_config,
    EmbeddingConfig, EmbeddingModelConfig, EmbeddingModelTemplate, EmbeddingPreset,
    EmbeddingProviderType, LocalEmbeddingModel, MemoryEmbeddingSelection,
    MemoryEmbeddingSetDefaultResult, MemoryEmbeddingState,
};
pub use factory::create_embedding_provider;

// Also re-export provider structs that were public
pub use api_provider::ApiEmbeddingProvider;
pub use fallback_provider::FallbackEmbeddingProvider;
pub use local_provider::LocalEmbeddingProvider;
