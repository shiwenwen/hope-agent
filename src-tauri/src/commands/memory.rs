use crate::commands::CmdError;
use crate::get_memory_backend;
use crate::memory;

#[tauri::command]
pub async fn memory_add(entry: memory::NewMemory) -> Result<i64, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let id = ha_core::blocking::run_blocking(move || backend.add(entry)).await?;
    Ok(id)
}

#[tauri::command]
pub async fn memory_update(id: i64, content: String, tags: Vec<String>) -> Result<(), CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.update(id, &content, &tags)).await?;
    Ok(())
}

#[tauri::command]
pub async fn memory_toggle_pin(id: i64, pinned: bool) -> Result<(), CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.toggle_pin(id, pinned)).await?;
    Ok(())
}

#[tauri::command]
pub async fn memory_delete(id: i64) -> Result<(), CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.delete(id)).await?;
    Ok(())
}

#[tauri::command]
pub async fn memory_get(id: i64) -> Result<Option<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let entry = ha_core::blocking::run_blocking(move || backend.get(id)).await?;
    Ok(entry)
}

/// List structured claims (next-gen Dreaming, read-only). Optional scope
/// (`scope_type` + `scope_id` primitives — never a structured object, so the
/// HTTP query transport can't silently degrade the filter) / status /
/// claim_type; newest-updated first. Maps to `GET /api/claims`.
#[tauri::command]
pub async fn claim_list(
    scope_type: Option<String>,
    scope_id: Option<String>,
    status: Option<String>,
    claim_type: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<memory::claims::ClaimRecord>, CmdError> {
    let scope = memory::claims::parse_claim_scope(scope_type.as_deref(), scope_id.as_deref())?;
    let claims = ha_core::blocking::run_blocking(move || {
        memory::claims::list_claims(memory::claims::ClaimListFilter {
            scope,
            status,
            claim_type,
            limit,
            offset,
        })
    })
    .await?;
    Ok(claims)
}

/// Fetch a single claim plus its evidence + legacy-memory links. Returns
/// `null` if the id is unknown. Maps to `GET /api/claims/{id}`.
#[tauri::command]
pub async fn claim_get(id: String) -> Result<Option<memory::claims::ClaimDetail>, CmdError> {
    let detail = ha_core::blocking::run_blocking(move || memory::claims::get_claim(&id)).await?;
    Ok(detail)
}

/// User correction (Lucid Review, design §5.2 §5.3): partial-update one claim —
/// edit content/triple/tags, change status (approve / reject / mark-outdated),
/// move scope, or pin/unpin. Writes evidence + a decision-log entry and emits
/// `memory:claim_changed`. Flat params (not a wrapped struct) so the call shape
/// matches the HTTP `PATCH /api/claims/{id}` body verbatim — `id` interpolates
/// into the path there, the rest becomes the JSON body.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn claim_update(
    id: String,
    content: Option<String>,
    subject: Option<String>,
    predicate: Option<String>,
    object: Option<String>,
    tags: Option<Vec<String>>,
    status: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    pinned: Option<bool>,
    note: Option<String>,
) -> Result<memory::claims::ClaimActionOutcome, CmdError> {
    let outcome = ha_core::blocking::run_blocking(move || {
        memory::claims::update_claim(memory::claims::ClaimUpdate {
            claim_id: id,
            content,
            subject,
            predicate,
            object,
            tags,
            status,
            scope_type,
            scope_id,
            pinned,
            note,
        })
    })
    .await?;
    Ok(outcome)
}

/// Forget a claim (design §5.3): `permanent=false` archives it (kept as an audit
/// trail, linked legacy memories stop injecting); `true` hard-deletes the claim
/// graph + any legacy memory it solely managed. Maps to
/// `POST /api/claims/{id}/forget`.
#[tauri::command]
pub async fn claim_forget(
    id: String,
    permanent: Option<bool>,
    note: Option<String>,
) -> Result<memory::claims::ClaimActionOutcome, CmdError> {
    let outcome = ha_core::blocking::run_blocking(move || {
        memory::claims::forget_claim(&id, permanent.unwrap_or(false), note.as_deref())
    })
    .await?;
    Ok(outcome)
}

/// Dry-run: scan legacy memories and return a backfill plan (exact summary +
/// capped candidate preview), writing nothing. Maps to
/// `GET /api/memory/backfill/plan`. Full-table scan runs on a blocking thread.
#[tauri::command]
pub async fn memory_backfill_plan() -> Result<memory::claims::BackfillPlan, CmdError> {
    tokio::task::spawn_blocking(memory::claims::plan_backfill)
        .await
        .map_err(|e| CmdError::msg(format!("backfill plan task failed: {e}")))?
        .map_err(Into::into)
}

/// Apply the backfill deterministically (re-scan, NOT trusting any client-sent
/// candidate list): write a claim + memory evidence + detached link for each
/// not-yet-linked memory. Maps to `POST /api/memory/backfill/apply`.
#[tauri::command]
pub async fn memory_backfill_apply() -> Result<memory::claims::BackfillApplyResult, CmdError> {
    tokio::task::spawn_blocking(memory::claims::apply_backfill)
        .await
        .map_err(|e| CmdError::msg(format!("backfill apply task failed: {e}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_list(
    scope: Option<memory::MemoryScope>,
    types: Option<Vec<memory::MemoryType>>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let entries = ha_core::blocking::run_blocking(move || {
        backend.list(
            scope.as_ref(),
            types.as_deref(),
            limit.unwrap_or(50),
            offset.unwrap_or(0),
        )
    })
    .await?;
    Ok(entries)
}

#[tauri::command]
pub async fn memory_search(
    query: memory::MemorySearchQuery,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let results = ha_core::blocking::run_blocking(move || backend.search(&query)).await?;
    Ok(results)
}

#[tauri::command]
pub async fn memory_count(scope: Option<memory::MemoryScope>) -> Result<usize, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let count = ha_core::blocking::run_blocking(move || backend.count(scope.as_ref())).await?;
    Ok(count)
}

#[tauri::command]
pub async fn memory_export(scope: Option<memory::MemoryScope>) -> Result<String, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let md =
        ha_core::blocking::run_blocking(move || backend.export_markdown(scope.as_ref())).await?;
    Ok(md)
}

#[tauri::command]
pub async fn memory_find_similar(
    content: String,
    threshold: Option<f32>,
    limit: Option<usize>,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let dedup_cfg = memory::load_dedup_config();
    let threshold = threshold.unwrap_or(dedup_cfg.threshold_merge);
    let limit = limit.unwrap_or(5);
    let results = ha_core::blocking::run_blocking(move || {
        backend.find_similar(&content, None, None, threshold, limit)
    })
    .await?;
    Ok(results)
}

#[tauri::command]
pub async fn memory_delete_batch(ids: Vec<i64>) -> Result<usize, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let deleted = ha_core::blocking::run_blocking(move || backend.delete_batch(&ids)).await?;
    Ok(deleted)
}

#[tauri::command]
pub async fn memory_get_import_from_ai_prompt(locale: Option<String>) -> Result<String, CmdError> {
    let locale = locale.as_deref().unwrap_or("en");
    Ok(memory::import_prompt::import_from_ai_prompt(locale).to_string())
}

#[tauri::command]
pub async fn memory_import(
    content: String,
    format: String,
    dedup: bool,
) -> Result<memory::ImportResult, CmdError> {
    let entries = memory::parse_import(&content, &format)?;
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let result =
        ha_core::blocking::run_blocking(move || backend.import_entries(entries, dedup)).await?;
    Ok(result)
}

#[tauri::command]
pub async fn memory_reembed(ids: Option<Vec<i64>>) -> Result<usize, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let count = ha_core::blocking::run_blocking(move || match ids {
        Some(ids) => backend.reembed_batch(&ids),
        None => backend.reembed_all(),
    })
    .await?;
    Ok(count)
}

#[tauri::command]
pub async fn memory_stats(
    scope: Option<memory::MemoryScope>,
) -> Result<memory::MemoryStats, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let stats = ha_core::blocking::run_blocking(move || backend.stats(scope.as_ref())).await?;
    Ok(stats)
}

#[tauri::command]
pub async fn get_extract_config() -> Result<memory::MemoryExtractConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.memory_extract)
}

#[tauri::command]
pub async fn save_extract_config(config: memory::MemoryExtractConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_extract", "settings-ui"), move |store| {
        store.memory_extract = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_memory_selection_config() -> Result<memory::MemorySelectionConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.memory_selection)
}

#[tauri::command]
pub async fn save_memory_selection_config(
    config: memory::MemorySelectionConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_selection", "settings-ui"), move |store| {
        store.memory_selection = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_memory_budget_config() -> Result<memory::MemoryBudgetConfig, CmdError> {
    Ok(ha_core::config::cached_config().memory_budget.clone())
}

#[tauri::command]
pub async fn save_memory_budget_config(config: memory::MemoryBudgetConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_budget", "settings-ui"), move |store| {
        store.memory_budget = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_dedup_config() -> Result<memory::DedupConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.dedup)
}

#[tauri::command]
pub async fn save_dedup_config(config: memory::DedupConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_dedup", "settings-ui"), move |store| {
        store.dedup = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── Search Tuning Configs ──────────────────────────────────────

#[tauri::command]
pub async fn get_hybrid_search_config() -> Result<memory::HybridSearchConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.hybrid_search)
}

#[tauri::command]
pub async fn save_hybrid_search_config(config: memory::HybridSearchConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("hybrid_search", "settings-ui"), move |store| {
        store.hybrid_search = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_temporal_decay_config() -> Result<memory::TemporalDecayConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.temporal_decay)
}

#[tauri::command]
pub async fn save_temporal_decay_config(
    config: memory::TemporalDecayConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("temporal_decay", "settings-ui"), move |store| {
        store.temporal_decay = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_mmr_config() -> Result<memory::MmrConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.mmr)
}

#[tauri::command]
pub async fn save_mmr_config(config: memory::MmrConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_mmr", "settings-ui"), move |store| {
        store.mmr = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_embedding_cache_config() -> Result<memory::EmbeddingCacheConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.embedding_cache)
}

#[tauri::command]
pub async fn save_embedding_cache_config(
    config: memory::EmbeddingCacheConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("embedding_cache", "settings-ui"), move |store| {
        store.embedding_cache = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_multimodal_config() -> Result<memory::MultimodalConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.multimodal)
}

#[tauri::command]
pub async fn save_multimodal_config(config: memory::MultimodalConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("multimodal", "settings-ui"), move |store| {
        store.multimodal = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_embedding_config() -> Result<memory::EmbeddingConfig, CmdError> {
    let store = ha_core::config::cached_config();
    let resolved =
        memory::resolve_memory_embedding_config(&store.memory_embedding, &store.embedding_models)?;
    Ok(resolved
        .map(|(_, config, _)| config)
        .unwrap_or_else(memory::EmbeddingConfig::default))
}

#[tauri::command]
pub async fn save_embedding_config(config: memory::EmbeddingConfig) -> Result<(), CmdError> {
    memory::save_legacy_embedding_config(config, "settings-ui")?;
    Ok(())
}

#[tauri::command]
pub async fn get_embedding_presets() -> Result<Vec<memory::EmbeddingPreset>, CmdError> {
    Ok(memory::embedding_presets())
}

#[tauri::command]
pub async fn list_local_embedding_models() -> Result<Vec<memory::LocalEmbeddingModel>, CmdError> {
    Ok(memory::list_local_models_with_status())
}

#[tauri::command]
pub async fn embedding_model_config_list() -> Result<Vec<memory::EmbeddingModelConfig>, CmdError> {
    Ok(memory::list_embedding_model_configs())
}

#[tauri::command]
pub async fn embedding_model_config_templates(
) -> Result<Vec<memory::EmbeddingModelTemplate>, CmdError> {
    Ok(memory::embedding_model_config_templates())
}

#[tauri::command]
pub async fn embedding_model_config_save(
    config: memory::EmbeddingModelConfig,
) -> Result<memory::EmbeddingModelConfig, CmdError> {
    memory::save_embedding_model_config(config, "settings-ui").map_err(Into::into)
}

#[tauri::command]
pub async fn embedding_model_config_delete(id: String) -> Result<(), CmdError> {
    memory::delete_embedding_model_config(&id, "settings-ui").map_err(Into::into)
}

#[tauri::command]
pub async fn embedding_model_config_test(
    config: memory::EmbeddingModelConfig,
) -> Result<String, CmdError> {
    let config = config.normalize_for_save();
    config.validate()?;
    ha_core::provider::test::test_embedding(config.to_runtime_config(true))
        .await
        .map_err(CmdError::msg)
}

#[tauri::command]
pub async fn memory_embedding_get() -> Result<memory::EmbeddingSelectionState, CmdError> {
    Ok(memory::get_memory_embedding_state())
}

#[tauri::command]
pub async fn memory_embedding_set_default(
    model_config_id: String,
    mode: memory::ReembedMode,
) -> Result<memory::EmbeddingSetDefaultResult, CmdError> {
    memory::set_memory_embedding_default(&model_config_id, mode, "settings-ui", None)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_reembed_start(
    mode: memory::ReembedMode,
) -> Result<ha_core::local_model_jobs::LocalModelJobSnapshot, CmdError> {
    let model_id = ha_core::config::cached_config()
        .memory_embedding
        .model_config_id
        .clone()
        .ok_or_else(|| {
            CmdError::msg("No memory embedding model is currently active".to_string())
        })?;
    let snapshot = ha_core::blocking::run_blocking(move || {
        memory::start_memory_reembed_job(&model_id, mode, None)
    })
    .await?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn memory_embedding_disable() -> Result<memory::EmbeddingSelectionState, CmdError> {
    memory::disable_memory_embedding("settings-ui").map_err(Into::into)
}

// ── Core Memory (memory.md) commands ────────────────────────────

#[tauri::command]
pub async fn get_global_memory_md() -> Result<Option<String>, CmdError> {
    let path = crate::paths::root_dir()?.join("memory.md");
    if path.exists() {
        std::fs::read_to_string(&path).map(Some).map_err(Into::into)
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn save_global_memory_md(content: String) -> Result<(), CmdError> {
    let path = crate::paths::root_dir()?.join("memory.md");
    std::fs::write(&path, content).map_err(Into::into)
}

#[tauri::command]
pub async fn get_agent_memory_md(id: String) -> Result<Option<String>, CmdError> {
    let path = crate::paths::agent_dir(&id)?.join("memory.md");
    if path.exists() {
        std::fs::read_to_string(&path).map(Some).map_err(Into::into)
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn save_agent_memory_md(id: String, content: String) -> Result<(), CmdError> {
    let dir = crate::paths::agent_dir(&id)?;
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("memory.md"), content).map_err(Into::into)
}
