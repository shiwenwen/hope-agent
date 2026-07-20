mod api_provider;
pub mod config;
pub mod factory;
pub(crate) mod utils;

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original embedding.rs is re-exported here
// so that `crate::memory::embedding::XXX` and `crate::memory::XXX` continue to work.

pub use config::{
    active_signature_for, embedding_model_templates, embedding_presets, memory_embedding_state,
    resolve_memory_embedding_config, EmbeddingConfig, EmbeddingModelConfig, EmbeddingModelTemplate,
    EmbeddingModelTemplateModel, EmbeddingPreset, EmbeddingProviderType, EmbeddingSelection,
    EmbeddingSelectionState, EmbeddingSetDefaultResult,
};
pub use factory::create_embedding_provider;

// Also re-export provider structs that were public
pub use api_provider::ApiEmbeddingProvider;
