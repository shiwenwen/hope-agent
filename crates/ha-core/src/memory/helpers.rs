use super::types::*;
use super::{
    memory_embedding_state, resolve_memory_embedding_config, start_memory_reembed_job,
    EmbeddingConfig, EmbeddingModelConfig, EmbeddingModelTemplate, MemoryEmbeddingSelection,
    MemoryEmbeddingSetDefaultResult, MemoryEmbeddingState, ReembedMode,
};
use anyhow::{anyhow, Result};

/// Clean each word (keep alphanumeric / `_` / `-`), wrap non-empty results in
/// double quotes for FTS5 MATCH literal matching, and OR-join them. Returns
/// `None` when no usable term remains — callers short-circuit to an empty
/// result set instead of running an unbounded full-index scan.
fn format_fts_terms<'a, I: Iterator<Item = &'a str>>(words: I) -> Option<String> {
    let terms: Vec<String> = words
        .filter_map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            (!clean.is_empty()).then(|| format!("\"{}\"", clean))
        })
        .collect();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Sanitize a user query for FTS5 MATCH syntax (no stopword filtering).
pub(crate) fn sanitize_fts_query(query: &str) -> Option<String> {
    format_fts_terms(query.split_whitespace())
}

/// Load dedup thresholds from config.json, falling back to defaults.
pub fn load_dedup_config() -> DedupConfig {
    crate::config::cached_config().dedup.clone()
}

/// Load LLM memory selection config from config.json.
pub fn load_memory_selection_config() -> MemorySelectionConfig {
    crate::config::cached_config().memory_selection.clone()
}

/// Load global extract config from config.json.
pub fn load_extract_config() -> MemoryExtractConfig {
    crate::config::cached_config().memory_extract.clone()
}

/// Load hybrid search config from config.json.
pub fn load_hybrid_search_config() -> HybridSearchConfig {
    crate::config::cached_config().hybrid_search.clone()
}

/// Load temporal decay config from config.json.
pub fn load_temporal_decay_config() -> TemporalDecayConfig {
    crate::config::cached_config().temporal_decay.clone()
}

/// Load MMR config from config.json.
pub fn load_mmr_config() -> MmrConfig {
    crate::config::cached_config().mmr.clone()
}

/// Load multimodal config from config.json.
pub fn load_multimodal_config() -> MultimodalConfig {
    crate::config::cached_config().multimodal.clone()
}

/// Load embedding cache config from config.json.
pub fn load_embedding_cache_config() -> EmbeddingCacheConfig {
    crate::config::cached_config().embedding_cache.clone()
}

/// Apply the current embedding config to the in-memory backend, if present.
///
/// Config writes happen in several shells (Tauri commands, HTTP routes, and
/// settings tools). Keeping the hot-reload side effect here prevents server
/// mode from lagging behind the persisted `config.json` value.
pub fn apply_embedding_config_to_backend(config: &EmbeddingConfig, source: &str) -> Result<()> {
    let backend =
        crate::get_memory_backend().ok_or_else(|| anyhow!("Memory backend not initialized"))?;

    if config.enabled {
        let provider = crate::memory::create_embedding_provider(config)?;
        backend.set_embedder(provider);
        app_info!(
            "memory",
            "embedding",
            "Embedding provider applied after config save (source={})",
            source
        );
    } else {
        backend.clear_embedder();
        app_info!(
            "memory",
            "embedding",
            "Embedding provider cleared after config save (source={})",
            source
        );
    }

    Ok(())
}

pub fn embedding_model_config_templates() -> Vec<EmbeddingModelTemplate> {
    crate::memory::embedding_model_templates()
}

pub fn list_embedding_model_configs() -> Vec<EmbeddingModelConfig> {
    crate::config::cached_config().embedding_models.clone()
}

pub fn get_memory_embedding_state() -> MemoryEmbeddingState {
    let store = crate::config::cached_config();
    memory_embedding_state(&store.memory_embedding, &store.embedding_models)
}

pub fn active_embedding_signature() -> Option<String> {
    let store = crate::config::cached_config();
    if !store.memory_embedding.enabled {
        return None;
    }
    // `MemoryEmbeddingSelection.active_signature` is the persisted SHA256 of
    // the active model; prefer it on the hot path so add/update/search/stats
    // skip the per-call resolve + hash. Fall back to recompute if the cache
    // missed (e.g. legacy configs that predate the field).
    if let Some(sig) = store.memory_embedding.active_signature.as_ref() {
        return Some(sig.clone());
    }
    resolve_memory_embedding_config(&store.memory_embedding, &store.embedding_models)
        .ok()
        .flatten()
        .map(|(_, _, signature)| signature)
}

pub fn save_embedding_model_config(
    config: EmbeddingModelConfig,
    source: &str,
) -> Result<EmbeddingModelConfig> {
    let config = config.normalize_for_save();
    config.validate()?;
    let saved = config.clone();
    let saved_signature = saved.signature();
    let should_reload_active = crate::config::mutate_config(
        ("embedding_models.save", source),
        move |store| {
            let is_active = store.memory_embedding.enabled
                && store.memory_embedding.model_config_id.as_deref() == Some(saved.id.as_str());
            if is_active {
                let existing_signature = store
                    .embedding_models
                    .iter()
                    .find(|item| item.id == saved.id)
                    .map(EmbeddingModelConfig::signature);
                if existing_signature.as_deref() != Some(saved_signature.as_str()) {
                    return Err(anyhow!(
                    "Cannot change the current memory embedding model config. Switch or disable it first."
                ));
                }
            }
            if let Some(existing) = store
                .embedding_models
                .iter_mut()
                .find(|item| item.id == saved.id)
            {
                *existing = saved.clone();
            } else {
                store.embedding_models.push(saved.clone());
            }
            Ok(is_active)
        },
    )?;
    app_info!(
        "memory",
        "embedding_models",
        "Embedding model config saved: id={} name={} source={}",
        config.id,
        config.name,
        source
    );
    if should_reload_active {
        apply_memory_embedding_from_config(source)?;
        app_info!(
            "memory",
            "embedding_models",
            "Reloaded active embedding provider after config save: id={} source={}",
            config.id,
            source
        );
    }
    Ok(config)
}

pub fn save_legacy_embedding_config(
    config: EmbeddingConfig,
    source: &str,
) -> Result<MemoryEmbeddingState> {
    if !config.enabled {
        return disable_memory_embedding(source);
    }

    let mut model = EmbeddingModelConfig {
        id: String::new(),
        name: config
            .api_model
            .clone()
            .or_else(|| config.api_base_url.clone())
            .unwrap_or_else(|| "Embedding Model".to_string()),
        provider_type: config.provider_type.clone(),
        api_base_url: config.api_base_url.clone(),
        api_key: config.api_key.clone(),
        api_model: config.api_model.clone(),
        api_dimensions: config.api_dimensions,
        source: Some("legacy-embedding-config".to_string()),
    };
    let signature = model.signature();
    model.id = format!("legacy-embedding-{}", &signature[..12]);
    let model = model.normalize_for_save();
    let saved = save_embedding_model_config(model, source)?;
    Ok(set_memory_embedding_default(&saved.id, ReembedMode::KeepExisting, source)?.state)
}

pub fn delete_embedding_model_config(id: &str, source: &str) -> Result<()> {
    let id = id.to_string();
    let log_id = id.clone();
    crate::config::mutate_config(("embedding_models.delete", source), move |store| {
        if store.memory_embedding.enabled
            && store.memory_embedding.model_config_id.as_deref() == Some(id.as_str())
        {
            return Err(anyhow!(
                "Cannot delete the current memory embedding model. Switch or disable it first."
            ));
        }
        store.embedding_models.retain(|item| item.id != id);
        Ok(())
    })?;
    app_info!(
        "memory",
        "embedding_models",
        "Embedding model config deleted: id={} source={}",
        log_id,
        source
    );
    Ok(())
}

pub fn disable_memory_embedding(source: &str) -> Result<MemoryEmbeddingState> {
    crate::config::mutate_config(("memory_embedding.disable", source), |store| {
        store.memory_embedding = MemoryEmbeddingSelection::default();
        Ok(())
    })?;
    if let Some(backend) = crate::get_memory_backend() {
        backend.clear_embedder();
    }
    app_info!(
        "memory",
        "embedding",
        "Memory embedding disabled (source={})",
        source
    );
    Ok(get_memory_embedding_state())
}

/// Persist the user's choice of memory embedding model and kick off a
/// background reembed job under [`ReembedMode`].
///
/// Side effects:
/// 1. The runtime embedder is swapped immediately, so subsequent searches use
///    the new model. Old vectors stay searchable until the reembed job
///    overwrites them (KeepExisting) or are wiped before the job starts
///    (DeleteAll).
/// 2. `MemoryEmbeddingSelection.{enabled,model_config_id,active_signature}` are
///    written via `mutate_config`.
/// 3. `embedding_cache` rows whose signature does not match the new model are
///    pruned synchronously — the table is small.
/// 4. A new `MemoryReembed` background job is spawned. Any pre-existing
///    in-flight reembed is cancelled first to keep the invariant of "at most
///    one reembed running globally". The function returns immediately; UI
///    progress comes through `local_model_job:*` events.
pub fn set_memory_embedding_default(
    model_config_id: &str,
    mode: ReembedMode,
    source: &str,
) -> Result<MemoryEmbeddingSetDefaultResult> {
    let store = crate::config::cached_config();
    let model = store
        .embedding_models
        .iter()
        .find(|item| item.id == model_config_id)
        .cloned()
        .ok_or_else(|| anyhow!("Embedding model config not found: {model_config_id}"))?;
    model.validate()?;
    let runtime_config = model.to_runtime_config(true);
    let signature = model.signature();
    app_info!(
        "memory",
        "embedding",
        "Switch memory embedding model requested: id={} name={} mode={:?} source={}",
        model.id,
        model.name,
        mode,
        source
    );

    apply_embedding_config_to_backend(&runtime_config, source)?;
    crate::config::mutate_config(("memory_embedding.set_default", source), |store| {
        store.memory_embedding.enabled = true;
        store.memory_embedding.model_config_id = Some(model_config_id.to_string());
        store.memory_embedding.active_signature = Some(signature.clone());
        Ok(())
    })?;

    if let Some(backend) = crate::get_memory_backend() {
        if let Ok(pruned) = backend.prune_embedding_cache_to_signature(&signature) {
            if pruned > 0 {
                app_info!(
                    "memory",
                    "embedding",
                    "Pruned {} stale embedding_cache rows after model switch",
                    pruned
                );
            }
        }
    }

    let mut reembed_error = None;
    match start_memory_reembed_job(model_config_id, mode) {
        Ok(snapshot) => {
            app_info!(
                "memory",
                "embedding",
                "Memory reembed job spawned: id={} model={} mode={:?} source={}",
                snapshot.job_id,
                model_config_id,
                mode,
                source
            );
        }
        Err(e) => {
            let msg = e.to_string();
            app_warn!(
                "memory",
                "embedding",
                "Failed to spawn memory reembed job: {}",
                msg
            );
            reembed_error = Some(msg);
        }
    }

    Ok(MemoryEmbeddingSetDefaultResult {
        state: get_memory_embedding_state(),
        reembedded: 0,
        reembed_error,
    })
}

pub fn apply_memory_embedding_from_config(source: &str) -> Result<()> {
    let store = crate::config::cached_config();
    match resolve_memory_embedding_config(&store.memory_embedding, &store.embedding_models)? {
        Some((_, runtime_config, _)) => apply_embedding_config_to_backend(&runtime_config, source),
        None => {
            if let Some(backend) = crate::get_memory_backend() {
                backend.clear_embedder();
            }
            Ok(())
        }
    }
}

/// Extract keywords from a query, filtering English + Chinese stopwords for
/// better FTS matching. Falls back to `sanitize_fts_query(query)` when every
/// word is a stopword so rare legitimate single-stopword queries still match.
pub(crate) fn expand_query(query: &str) -> Option<String> {
    use std::collections::HashSet;

    let stopwords_en: HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "in", "on", "at", "to", "for", "of", "with",
        "by", "from", "this", "that", "it", "i", "you", "we", "they", "my", "your", "do", "does",
        "how", "what", "where", "when", "why", "which", "can", "could", "would", "should", "have",
        "has", "had", "be", "been", "being", "not", "no", "or", "and", "but", "if", "so", "as",
        "than", "too", "very", "about", "up", "out", "just", "also", "more", "some", "any", "all",
        "each",
    ]
    .into_iter()
    .collect();

    let stopwords_zh: HashSet<&str> = [
        "的", "了", "在", "是", "我", "有", "和", "的", "不", "人", "都", "一", "一个", "上", "也",
        "了", "到", "说", "要", "去", "你", "会", "着", "没有", "看", "好", "自己", "这", "那",
        "他", "她", "它", "们", "吗", "吧", "呢", "啊", "把", "被", "从", "对", "让", "给",
    ]
    .into_iter()
    .collect();

    format_fts_terms(query.split_whitespace().filter(|w| {
        let lower = w.to_lowercase();
        lower.len() > 1 && !stopwords_en.contains(lower.as_str()) && !stopwords_zh.contains(*w)
    }))
    .or_else(|| sanitize_fts_query(query))
}
