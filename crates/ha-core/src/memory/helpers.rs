use super::types::*;
use super::{
    active_signature_for, memory_embedding_state, resolve_memory_embedding_config,
    start_memory_reembed_job, EmbeddingConfig, EmbeddingModelConfig, EmbeddingModelTemplate,
    EmbeddingSelectionState, EmbeddingSetDefaultResult, ReembedMode,
};
use anyhow::{anyhow, Result};

/// Clean each word (keep alphanumeric / `_` / `-`), wrap non-empty results in
/// double quotes for FTS5 MATCH literal matching, and AND-join independent
/// terms. Prefix variants are reserved for identifiers and very short tokens;
/// adding `term*` to every natural-language word turns ordinary multi-word
/// queries into a broad OR scan on large indexes.
fn format_fts_terms<'a, I: Iterator<Item = &'a str>>(words: I) -> Option<String> {
    let mut groups: Vec<String> = Vec::new();

    for word in words {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if clean.is_empty() {
            continue;
        }

        let exact = format!("\"{}\"", clean);
        let char_count = clean.chars().count();
        let needs_prefix = clean.contains('_') || clean.contains('-') || char_count <= 2;
        let group = if needs_prefix && char_count > 1 {
            let prefix = format!("\"{}\"*", clean);
            format!("({exact} OR {prefix})")
        } else {
            exact
        };
        if !groups.contains(&group) {
            groups.push(group);
        }
    }

    if groups.is_empty() {
        None
    } else {
        Some(groups.join(" AND "))
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

/// Adaptive lexical weights for hybrid RRF. A sparse lexical arm usually
/// represents an exact identifier / phrase hit and must not be displaced by a
/// broad vector neighborhood merely because the configured vector weight is
/// numerically larger. Broad lexical result sets keep the configured weights
/// unchanged.
pub(crate) fn adaptive_lexical_rrf_weights(
    text_weight: f32,
    vector_weight: f32,
    primary_count: usize,
    literal_count: usize,
    final_limit: usize,
) -> (f64, f64) {
    let text_weight = text_weight.max(0.0) as f64;
    let vector_weight = vector_weight.max(0.0) as f64;
    let precision_boost = vector_weight + text_weight.max(0.05);
    let primary_weight = text_weight
        + if primary_count > 0 && primary_count <= final_limit {
            precision_boost
        } else {
            0.0
        };
    let literal_weight = text_weight * 0.5
        + if primary_count == 0 && literal_count > 0 && literal_count <= final_limit {
            precision_boost
        } else {
            0.0
        };
    (primary_weight, literal_weight)
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

pub fn get_memory_embedding_state() -> EmbeddingSelectionState {
    let store = crate::config::cached_config();
    memory_embedding_state(&store.memory_embedding, &store.embedding_models)
}

pub fn get_external_memory_provider_preflight() -> ExternalMemoryProviderPreflightReport {
    let cfg = crate::memory::hydrate_external_memory_provider_config(
        crate::config::cached_config().memory_providers.clone(),
    );
    let (stats, stats_error) = external_memory_provider_stats_for_planning();
    cfg.sync_preflight_with_stats_status(&stats, stats_error)
}

pub async fn run_external_memory_provider_sync() -> ExternalMemoryProviderSyncReport {
    let cfg = crate::config::cached_config().memory_providers.clone();
    let (stats, stats_error) = external_memory_provider_stats_for_planning();
    crate::memory::execute_external_memory_provider_sync(cfg, stats, stats_error).await
}

pub(crate) fn external_memory_provider_stats_for_planning() -> (MemoryStats, Option<String>) {
    let stats_result = match crate::get_memory_backend() {
        Some(backend) => backend.stats(None).map_err(|err| err.to_string()),
        None => Err("memory backend unavailable".to_string()),
    };
    let (stats, stats_error) = match stats_result {
        Ok(stats) => (stats, None),
        Err(err) => (
            MemoryStats {
                total: 0,
                by_type: std::collections::HashMap::new(),
                by_source: std::collections::HashMap::new(),
                with_embedding: 0,
                oldest: None,
                newest: None,
            },
            Some(err),
        ),
    };
    (stats, stats_error)
}

pub fn active_embedding_signature() -> Option<String> {
    let store = crate::config::cached_config();
    active_signature_for(&store.memory_embedding, &store.embedding_models)
}

/// Which subsystems currently have `model_id` as their active embedding model
/// (memory, knowledge). The `embedding_models` library is shared between both
/// (D7), so editing or deleting an entry that is active anywhere must be refused
/// — this is the single source for that cross-protection check.
pub(crate) fn embedding_model_active_subsystems(
    cfg: &crate::config::AppConfig,
    model_id: &str,
) -> (bool, bool) {
    let memory = cfg.memory_embedding.enabled
        && cfg.memory_embedding.model_config_id.as_deref() == Some(model_id);
    let knowledge = cfg.knowledge_embedding.enabled
        && cfg.knowledge_embedding.model_config_id.as_deref() == Some(model_id);
    (memory, knowledge)
}

pub fn save_embedding_model_config(
    config: EmbeddingModelConfig,
    source: &str,
) -> Result<EmbeddingModelConfig> {
    let config = config.normalize_for_save();
    config.validate()?;
    let saved = config.clone();
    let saved_signature = saved.signature();
    // The model library is shared between memory and knowledge (D7). Editing a
    // config that is the *active* model of either subsystem would change its
    // signature out from under already-embedded vectors, so it is refused; both
    // active checks gate the same way. The closure returns which subsystem(s)
    // need their runtime provider reloaded after a no-signature-change save.
    let (reload_memory, reload_knowledge) = crate::config::mutate_config(
        ("embedding_models.save", source),
        move |store| {
            let (memory_active, knowledge_active) =
                embedding_model_active_subsystems(store, saved.id.as_str());
            if memory_active || knowledge_active {
                let existing_signature = store
                    .embedding_models
                    .iter()
                    .find(|item| item.id == saved.id)
                    .map(EmbeddingModelConfig::signature);
                if existing_signature.as_deref() != Some(saved_signature.as_str()) {
                    return Err(anyhow!(
                    "Cannot change an embedding model config while it is the active memory or knowledge model. Switch or disable it first."
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
            Ok((memory_active, knowledge_active))
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
    if reload_memory {
        apply_memory_embedding_from_config(source)?;
        app_info!(
            "memory",
            "embedding_models",
            "Reloaded active memory embedding provider after config save: id={} source={}",
            config.id,
            source
        );
    }
    if reload_knowledge {
        crate::knowledge::apply_knowledge_embedding_from_config(source);
        app_info!(
            "knowledge",
            "embedding_models",
            "Reloaded active knowledge embedding provider after config save: id={} source={}",
            config.id,
            source
        );
    }
    Ok(config)
}

pub fn save_legacy_embedding_config(
    config: EmbeddingConfig,
    source: &str,
) -> Result<EmbeddingSelectionState> {
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
    Ok(set_memory_embedding_default(&saved.id, ReembedMode::KeepExisting, source, None)?.state)
}

pub fn delete_embedding_model_config(id: &str, source: &str) -> Result<()> {
    let id = id.to_string();
    let log_id = id.clone();
    crate::config::mutate_config(("embedding_models.delete", source), move |store| {
        let (memory_active, knowledge_active) =
            embedding_model_active_subsystems(store, id.as_str());
        if memory_active || knowledge_active {
            return Err(anyhow!(
                "Cannot delete an embedding model while it is the active memory or knowledge model. Switch or disable it first."
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

pub fn disable_memory_embedding(source: &str) -> Result<EmbeddingSelectionState> {
    crate::config::mutate_config(("memory_embedding.disable", source), |store| {
        // Pause semantics: keep `model_config_id` / `active_signature` /
        // `last_reembedded_signature` so re-enabling the same model can short
        // circuit reembed and the frontend's "remember last model" toggle path
        // (EmbeddingView::handleToggle stillValid branch) still finds the id.
        store.memory_embedding.enabled = false;
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
/// 2. `EmbeddingSelection.{enabled,model_config_id,active_signature}` are
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
    parent_job_id: Option<&str>,
) -> Result<EmbeddingSetDefaultResult> {
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

    // 仅 KeepExisting 模式适用同 signature 短路：DeleteAll 的「先清空再重建」
    // 语义只在 `start_memory_reembed_job` 内通过 `clear_all_embeddings()` 实现，
    // 跳过任务派发会让用户的「从头重建」请求既不清空也不重建。
    let same_signature =
        store.memory_embedding.last_reembedded_signature.as_deref() == Some(signature.as_str());

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

    // pending_count 必须在 `enabled = true` 写入**之后**再算（TOCTOU）：disable
    // 期间 add_memory 写 row 时 embedding_signature = NULL，预读会漏检这些 row。
    let pending_count = if same_signature && mode == ReembedMode::KeepExisting {
        crate::get_memory_backend()
            .map(|b| b.count_memories_pending_embedding(&signature).unwrap_or(0))
            .unwrap_or(0)
    } else {
        0
    };
    // 有活跃 reembed 任务时强制 spawn：cancel 是 per-batch 的异步检查，旧任务
    // 最后一批可能在 skip 之后才退出，把 last_reembedded_signature 改成它针对
    // 的（已不再活跃的）模型。spawn 新任务由 start_memory_reembed_job 内部的
    // cancel 调用 + 新 batch 写回兜底。
    let has_active_reembed = if same_signature && mode == ReembedMode::KeepExisting {
        crate::local_model_jobs::has_active_job(
            crate::local_model_jobs::LocalModelJobKind::MemoryReembed,
        )
        .unwrap_or(false)
    } else {
        false
    };
    let already_reembedded_for_signature = same_signature
        && mode == ReembedMode::KeepExisting
        && pending_count == 0
        && !has_active_reembed;

    let mut reembed_error = None;
    if already_reembedded_for_signature {
        // already_reembedded_for_signature 守门已含 !has_active_reembed，到这里
        // 全局必然已无在跑的 MemoryReembed 任务，无需再 cancel。
        app_info!(
            "memory",
            "embedding",
            "Skipping reembed: signature {} already covers all memories. Source={}",
            crate::truncate_utf8(&signature, 12),
            source
        );
    } else {
        if same_signature && pending_count > 0 {
            app_info!(
                "memory",
                "embedding",
                "Re-running reembed despite signature match: {} pending memory rows. Source={}",
                pending_count,
                source
            );
            // 失效 last_reembedded_signature：reembed 失败/取消时它保持 None，
            // get_memory_embedding_state 才能报告 needsReembed=true 提醒用户。
            crate::config::mutate_config(
                ("memory_embedding.invalidate_reembedded", source),
                |store| {
                    store.memory_embedding.last_reembedded_signature = None;
                    Ok(())
                },
            )?;
        }
        match start_memory_reembed_job(model_config_id, mode, parent_job_id) {
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
    }

    // `reembedded` stays in the response shape for wire compatibility but is
    // always 0 now: the actual reembed runs as a background job (see
    // `start_memory_reembed_job`). Counts come through the standard
    // `local_model_job:*` event stream instead.
    Ok(EmbeddingSetDefaultResult {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_query_adds_prefix_terms_for_identifier_fragments() {
        let query = expand_query("prepare_messages").expect("query should be usable");
        assert!(query.contains("\"prepare_messages\""));
        assert!(query.contains("\"prepare_messages\"*"));
    }

    #[test]
    fn expand_query_keeps_chinese_short_terms_searchable() {
        let query = expand_query("中文").expect("query should be usable");
        assert!(query.contains("\"中文\""));
        assert!(query.contains("\"中文\"*"));
    }

    #[test]
    fn expand_query_requires_all_normal_words_without_prefix_scans() {
        let query = expand_query("release incident").expect("query should be usable");
        assert_eq!(query, "\"release\" AND \"incident\"");
    }

    #[test]
    fn sparse_lexical_rrf_weight_beats_broad_vector_arm() {
        let (primary, literal) = adaptive_lexical_rrf_weights(0.4, 0.6, 1, 0, 10);
        assert!(primary > 0.6);
        assert!((literal - 0.2).abs() < 1e-6);

        let (primary, literal) = adaptive_lexical_rrf_weights(0.4, 0.6, 0, 1, 10);
        assert!((primary - 0.4).abs() < 1e-6);
        assert!(literal > 0.6);
    }

    #[test]
    fn broad_lexical_rrf_weights_preserve_configured_mix() {
        let (primary, literal) = adaptive_lexical_rrf_weights(0.4, 0.6, 30, 0, 10);
        assert!((primary - 0.4).abs() < 1e-6);
        assert!((literal - 0.2).abs() < 1e-6);
    }
}
