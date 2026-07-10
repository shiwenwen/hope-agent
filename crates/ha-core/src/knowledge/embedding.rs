//! Knowledge-base embedding selection (D7).
//!
//! The knowledge base owns its embedding selection (`knowledge_embedding`),
//! fully independent of memory's (`memory_embedding`): its own enable switch,
//! model choice, active signature, and reembed lifecycle. It draws from the
//! **same shared `embedding_models` library** (configure a provider/API key
//! once, both subsystems can pick it) and reuses memory's provider factory +
//! signature algorithm. (Note: the knowledge index calls the raw
//! `EmbeddingProvider` directly via `IndexDb`, so it does NOT go through memory's
//! `embedding_cache` — that table is internal to the memory SQLite backend.)
//!
//! Crucially, knowledge embedding being **off does not fall back to memory** —
//! note vector search just degrades to FTS-only. This is the whole point of the
//! split: memory may be unconfigured, knowledge retrieval should not depend on
//! that.

use anyhow::{anyhow, Result};

use crate::memory::{
    active_signature_for, create_embedding_provider, memory_embedding_state,
    EmbeddingSelectionState,
};

use super::index::{apply_embedding_to_index, get_index_db};
use super::reembed::{cancel_active_knowledge_reembed_jobs, start_knowledge_reembed_job};

/// Current knowledge-embedding selection state: selected model + `needsReembed`.
/// Drives the knowledge settings UI (mirrors `get_memory_embedding_state`).
pub fn get_knowledge_embedding_state() -> EmbeddingSelectionState {
    let store = crate::config::cached_config();
    memory_embedding_state(&store.knowledge_embedding, &store.embedding_models)
}

/// Active knowledge-embedding signature (persisted SHA256 of the active model).
/// `None` when knowledge embedding is disabled / unresolved — note chunks index
/// FTS-only and `search_notes` skips the vector leg. This is the knowledge
/// analogue of [`crate::memory::helpers::active_embedding_signature`], read from
/// `knowledge_embedding` rather than `memory_embedding`.
pub fn knowledge_active_embedding_signature() -> Option<String> {
    let store = crate::config::cached_config();
    active_signature_for(&store.knowledge_embedding, &store.embedding_models)
}

/// Persist the user's choice of knowledge embedding model, swap the index DB's
/// embedder immediately, and kick off a background reembed (a full reindex of
/// every KB, which re-embeds each note chunk under the new model).
///
/// Mirrors [`crate::memory::set_memory_embedding_default`] but writes
/// `knowledge_embedding` and rebuilds the note index instead of the memory
/// store. The provider AND the index DB are resolved up front so a bad config or
/// an uninitialized index fails *before* anything is persisted — never leaving a
/// selection that says "enabled" with no embedder installed.
pub fn set_knowledge_embedding_default(
    model_config_id: &str,
    source: &str,
) -> Result<EmbeddingSelectionState> {
    let store = crate::config::cached_config();
    let model = store
        .embedding_models
        .iter()
        .find(|item| item.id == model_config_id)
        .cloned()
        .ok_or_else(|| anyhow!("Embedding model config not found: {model_config_id}"))?;
    model.validate()?;
    let runtime_config = model.to_runtime_config(true);
    // Build the provider and require the index DB up front: either failing here
    // returns Err before we mutate persisted state, so config never ends up
    // enabled-but-embedder-less.
    let provider = create_embedding_provider(&runtime_config)?;
    let db = get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let signature = model.signature();

    // Same-signature short-circuit: re-selecting the already-active, already
    // re-embedded model (equivalently `!needsReembed` for the same model) is a
    // no-op — skip the full re-embed and the note_vec churn. memory has the
    // analogous `already_reembedded_for_signature` guard.
    let already_active_same = store.knowledge_embedding.enabled
        && store.knowledge_embedding.active_signature.as_deref() == Some(signature.as_str())
        && store
            .knowledge_embedding
            .last_reembedded_signature
            .as_deref()
            == Some(signature.as_str());

    crate::app_info!(
        "knowledge",
        "embedding",
        "Switch knowledge embedding model requested: id={} name={} same={} source={}",
        model.id,
        model.name,
        already_active_same,
        source
    );

    if already_active_same {
        // Nothing changed; just ensure the runtime embedder is installed.
        db.set_embedder(provider);
        return Ok(get_knowledge_embedding_state());
    }

    // Cancel any in-flight reembed BEFORE swapping the embedder (which may DROP +
    // recreate note_vec on a dimension change), so the old job can't write
    // old-dimension vectors into the freshly recreated table.
    cancel_active_knowledge_reembed_jobs(None);

    crate::config::mutate_config(("knowledge_embedding.set_default", source), |store| {
        store.knowledge_embedding.enabled = true;
        store.knowledge_embedding.model_config_id = Some(model_config_id.to_string());
        store.knowledge_embedding.active_signature = Some(signature.clone());
        // Invalidate so `needsReembed` reports true until the reindex finishes;
        // the reembed job writes `last_reembedded_signature` on success only.
        store.knowledge_embedding.last_reembedded_signature = None;
        Ok(())
    })?;

    // Swap the runtime embedder on the index DB so subsequent note writes embed
    // with the new model + dimension (note_vec recreated on a dimension change).
    db.set_embedder(provider);

    // Rebuild every KB's note vectors under the new model (background job).
    if let Err(e) = start_knowledge_reembed_job(None, source) {
        crate::app_warn!(
            "knowledge",
            "embedding",
            "failed to spawn knowledge reembed job: {}",
            e
        );
    }

    Ok(get_knowledge_embedding_state())
}

/// Disable knowledge vector search. Keeps `model_config_id` /
/// `last_reembedded_signature` (pause semantics — re-enabling the same model can
/// skip the reembed) and clears the index embedder so note writes go FTS-only.
pub fn disable_knowledge_embedding(source: &str) -> Result<EmbeddingSelectionState> {
    // Cancel any in-flight reembed first: an orphan job would otherwise keep
    // running against the about-to-be-cleared embedder (embedding nothing) and
    // still stamp `last_reembedded_signature` on completion.
    cancel_active_knowledge_reembed_jobs(None);
    crate::config::mutate_config(("knowledge_embedding.disable", source), |store| {
        store.knowledge_embedding.enabled = false;
        Ok(())
    })?;
    if let Some(db) = get_index_db() {
        db.clear_embedder();
    }
    crate::app_info!(
        "knowledge",
        "embedding",
        "Knowledge embedding disabled (source={})",
        source
    );
    Ok(get_knowledge_embedding_state())
}

/// Reload the index embedder from the persisted `knowledge_embedding` selection.
/// Called by [`crate::memory::save_embedding_model_config`] after an edit to the
/// active knowledge model. NOTE: there is currently **no** `config:changed`
/// subscriber wired to this, so config mutations that bypass the dedicated
/// set/disable/save paths (e.g. backup restore, config rollback) do not
/// hot-reload the embedder — it picks up the change on next restart. (memory's
/// `apply_memory_embedding_from_config` has the same limitation.)
pub fn apply_knowledge_embedding_from_config(_source: &str) {
    if let Some(db) = get_index_db() {
        apply_embedding_to_index(&db);
    }
}
