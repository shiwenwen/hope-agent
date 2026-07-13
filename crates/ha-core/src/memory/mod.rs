pub mod audit;
pub mod backup;
pub mod claims;
pub(crate) mod context_manifest;
pub mod dreaming;
pub mod embedding;
pub mod episodes;
pub mod external_provider;
pub mod helpers;
pub mod import;
pub mod import_prompt;
pub mod mmr;
pub mod recall_summary;
pub mod reembed_job;
pub mod runtime_config;
pub(crate) mod selection;
pub mod sqlite;
pub mod traits;
pub mod types;

pub const EVENT_MEMORY_CHANGED: &str = "memory:changed";
pub const EVENT_MEMORY_CLAIM_CHANGED: &str = "memory:claim_changed";

pub fn emit_memory_changed(action: &str, memory_id: Option<i64>, count: Option<usize>) {
    if let Some(bus) = crate::get_event_bus() {
        let mut payload = serde_json::json!({ "action": action });
        if let Some(id) = memory_id {
            payload["memoryId"] = serde_json::json!(id);
        }
        if let Some(n) = count {
            payload["count"] = serde_json::json!(n);
        }
        bus.emit(EVENT_MEMORY_CHANGED, payload);
    }
    external_provider::schedule_external_memory_provider_sync();
}

pub fn emit_claim_changed(action: &str, claim_id: Option<&str>, count: Option<usize>) {
    if let Some(bus) = crate::get_event_bus() {
        let mut payload = serde_json::json!({ "action": action });
        if let Some(id) = claim_id {
            payload["claimId"] = serde_json::json!(id);
        }
        if let Some(n) = count {
            payload["count"] = serde_json::json!(n);
        }
        bus.emit(EVENT_MEMORY_CLAIM_CHANGED, payload);
    }
}

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original memory.rs is re-exported here
// so that `crate::memory::XXX` continues to work.

pub use audit::*;
pub use backup::*;
pub use embedding::*;
pub use episodes::*;
pub use external_provider::*;
pub use helpers::{
    apply_embedding_config_to_backend, delete_embedding_model_config, disable_memory_embedding,
    embedding_model_config_templates, get_external_memory_provider_preflight,
    get_memory_embedding_state, list_embedding_model_configs, load_dedup_config,
    load_extract_config, run_external_memory_provider_sync, save_embedding_model_config,
    save_legacy_embedding_config, set_memory_embedding_default,
};
pub use import::*;
pub use recall_summary::{maybe_summarize_recall, RecallSummaryConfig};
pub use reembed_job::{cancel_active_memory_reembed_jobs, start_memory_reembed_job, ReembedMode};
pub use runtime_config::*;
pub use sqlite::SqliteMemoryBackend;
pub use traits::*;
pub use types::*;
