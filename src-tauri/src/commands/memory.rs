use crate::commands::CmdError;
use crate::get_memory_backend;
use crate::memory;

#[tauri::command]
pub async fn memory_add(entry: memory::NewMemory) -> Result<i64, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.add(entry).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_update(id: i64, content: String, tags: Vec<String>) -> Result<(), CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.update(id, &content, &tags).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_toggle_pin(id: i64, pinned: bool) -> Result<(), CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.toggle_pin(id, pinned).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_delete(id: i64) -> Result<(), CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.delete(id).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_get(id: i64) -> Result<Option<memory::MemoryEntry>, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.get(id).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_list(
    scope: Option<memory::MemoryScope>,
    types: Option<Vec<memory::MemoryType>>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend
        .list(
            scope.as_ref(),
            types.as_deref(),
            limit.unwrap_or(50),
            offset.unwrap_or(0),
        )
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_search(
    query: memory::MemorySearchQuery,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.search(&query).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_count(scope: Option<memory::MemoryScope>) -> Result<usize, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.count(scope.as_ref()).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_export(scope: Option<memory::MemoryScope>) -> Result<String, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.export_markdown(scope.as_ref()).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_find_similar(
    content: String,
    threshold: Option<f32>,
    limit: Option<usize>,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    let dedup_cfg = memory::load_dedup_config();
    let threshold = threshold.unwrap_or(dedup_cfg.threshold_merge);
    let limit = limit.unwrap_or(5);
    backend
        .find_similar(&content, None, None, threshold, limit)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_delete_batch(ids: Vec<i64>) -> Result<usize, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.delete_batch(&ids).map_err(Into::into)
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
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.import_entries(entries, dedup).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_reembed(ids: Option<Vec<i64>>) -> Result<usize, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    match ids {
        Some(ids) => backend.reembed_batch(&ids).map_err(Into::into),
        None => backend.reembed_all().map_err(Into::into),
    }
}

#[tauri::command]
pub async fn memory_stats(
    scope: Option<memory::MemoryScope>,
) -> Result<memory::MemoryStats, CmdError> {
    let backend =
        get_memory_backend().ok_or_else(|| CmdError::msg("Memory backend not initialized"))?;
    backend.stats(scope.as_ref()).map_err(Into::into)
}

#[tauri::command]
pub async fn get_extract_config() -> Result<memory::MemoryExtractConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.memory_extract)
}

#[tauri::command]
pub async fn save_extract_config(config: memory::MemoryExtractConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("memory_extract", "settings-ui"), |store| {
        store.memory_extract = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("memory_selection", "settings-ui"), |store| {
        store.memory_selection = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_memory_budget_config() -> Result<memory::MemoryBudgetConfig, CmdError> {
    Ok(ha_core::config::cached_config().memory_budget.clone())
}

#[tauri::command]
pub async fn save_memory_budget_config(config: memory::MemoryBudgetConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("memory_budget", "settings-ui"), |store| {
        store.memory_budget = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_dedup_config() -> Result<memory::DedupConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.dedup)
}

#[tauri::command]
pub async fn save_dedup_config(config: memory::DedupConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("memory_dedup", "settings-ui"), |store| {
        store.dedup = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("hybrid_search", "settings-ui"), |store| {
        store.hybrid_search = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("temporal_decay", "settings-ui"), |store| {
        store.temporal_decay = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_mmr_config() -> Result<memory::MmrConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.mmr)
}

#[tauri::command]
pub async fn save_mmr_config(config: memory::MmrConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("memory_mmr", "settings-ui"), |store| {
        store.mmr = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("embedding_cache", "settings-ui"), |store| {
        store.embedding_cache = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_multimodal_config() -> Result<memory::MultimodalConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.multimodal)
}

#[tauri::command]
pub async fn save_multimodal_config(config: memory::MultimodalConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("multimodal", "settings-ui"), |store| {
        store.multimodal = config;
        Ok(())
    })
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
pub async fn memory_embedding_get() -> Result<memory::MemoryEmbeddingState, CmdError> {
    Ok(memory::get_memory_embedding_state())
}

#[tauri::command]
pub async fn memory_embedding_set_default(
    model_config_id: String,
    mode: memory::ReembedMode,
) -> Result<memory::MemoryEmbeddingSetDefaultResult, CmdError> {
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
    memory::start_memory_reembed_job(&model_id, mode, None).map_err(Into::into)
}

#[tauri::command]
pub async fn memory_embedding_disable() -> Result<memory::MemoryEmbeddingState, CmdError> {
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
