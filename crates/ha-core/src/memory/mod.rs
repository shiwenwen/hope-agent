pub mod dreaming;
pub mod embedding;
pub mod helpers;
pub mod import;
pub mod import_prompt;
pub mod mmr;
pub mod recall_summary;
pub mod reembed_job;
pub(crate) mod selection;
pub mod sqlite;
pub mod traits;
pub mod types;

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original memory.rs is re-exported here
// so that `crate::memory::XXX` continues to work.

pub use embedding::*;
pub use helpers::{
    apply_embedding_config_to_backend, delete_embedding_model_config, disable_memory_embedding,
    embedding_model_config_templates, get_memory_embedding_state, list_embedding_model_configs,
    load_dedup_config, load_extract_config, save_embedding_model_config,
    save_legacy_embedding_config, set_memory_embedding_default,
};
pub use import::*;
pub use recall_summary::{maybe_summarize_recall, RecallSummaryConfig};
pub use reembed_job::{cancel_active_memory_reembed_jobs, start_memory_reembed_job, ReembedMode};
pub use sqlite::SqliteMemoryBackend;
pub use traits::*;
pub use types::*;
