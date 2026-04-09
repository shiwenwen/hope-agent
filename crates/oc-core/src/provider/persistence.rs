use anyhow::Result;
use arc_swap::ArcSwap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::paths;

use super::store::ProviderStore;
use super::types::{
    ActiveModel, ApiType, AvailableModel, ModelConfig, ProviderConfig, ThinkingStyle,
};

// ── Persistence ───────────────────────────────────────────────────

fn config_path() -> Result<PathBuf> {
    paths::config_path()
}

/// Process-wide in-memory snapshot of the provider store.
///
/// Populated lazily on first access and refreshed atomically on every
/// successful [`save_store`]. All reads are lock-free acquire loads — this is
/// why [`cached_store`] is safe to call from hot paths (tool execution, chat
/// loops, memory lookups, channel workers) without any synchronization cost.
fn cache() -> &'static ArcSwap<ProviderStore> {
    static CACHE: OnceLock<ArcSwap<ProviderStore>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let initial = read_from_disk().unwrap_or_default();
        ArcSwap::from_pointee(initial)
    })
}

fn read_from_disk() -> Result<ProviderStore> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(ProviderStore::default());
    }
    let data = std::fs::read_to_string(&path)?;
    let store: ProviderStore = serde_json::from_str(&data)?;
    Ok(store)
}

/// Shared read-only snapshot of the provider store. **Lock-free, zero data
/// clone** — one atomic acquire load plus an `Arc` refcount bump.
///
/// Use this in hot paths and read-only accesses. The returned `Arc` is a
/// point-in-time snapshot; a concurrent [`save_store`] will not affect it.
pub fn cached_store() -> Arc<ProviderStore> {
    cache().load_full()
}

/// Load an owned copy of the provider store. Clones the cached snapshot;
/// use when you need to mutate and then call [`save_store`]. Read-only
/// callers should use [`cached_store`] instead.
pub fn load_store() -> Result<ProviderStore> {
    Ok((*cached_store()).clone())
}

/// Persist the provider store to disk and refresh the in-memory cache.
///
/// Callers must pass the full, mutated store — this function does not merge
/// with the existing on-disk content.
pub fn save_store(store: &ProviderStore) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Debug: log channel account IDs on every save to detect accidental overwrite
    let account_ids: Vec<&str> = store
        .channels
        .accounts
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    app_debug!(
        "provider",
        "save_store",
        "Saving config with {} channel account(s): {:?}",
        account_ids.len(),
        account_ids
    );
    let data = serde_json::to_string_pretty(store)?;
    std::fs::write(&path, data)?;

    // Atomically publish the new snapshot so subsequent cached_store() calls
    // see the refreshed state without touching disk.
    cache().store(Arc::new(store.clone()));
    Ok(())
}

/// Force a fresh disk read into the cache. Use after an out-of-band write
/// to `config.json` (e.g. [`crate::backup::restore_backup`]) so hot-path
/// readers don't keep serving the stale snapshot.
pub fn reload_cache_from_disk() -> Result<()> {
    let fresh = read_from_disk()?;
    cache().store(Arc::new(fresh));
    Ok(())
}

// ── Helper: Build available models list ───────────────────────────

pub fn build_available_models(providers: &[ProviderConfig]) -> Vec<AvailableModel> {
    let mut models = Vec::new();
    for p in providers {
        if !p.enabled {
            continue;
        }
        for m in &p.models {
            models.push(AvailableModel {
                provider_id: p.id.clone(),
                provider_name: p.name.clone(),
                api_type: p.api_type.clone(),
                model_id: m.id.clone(),
                model_name: m.name.clone(),
                input_types: m.input_types.clone(),
                context_window: m.context_window,
                max_tokens: m.max_tokens,
                reasoning: m.reasoning,
            });
        }
    }
    models
}

// ── Helper: Parse model reference ─────────────────────────────────

/// Parse a "provider_id::model_id" string into an ActiveModel.
/// Returns None if the format is invalid.
pub fn parse_model_ref(ref_str: &str) -> Option<ActiveModel> {
    let parts: Vec<&str> = ref_str.splitn(2, "::").collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(ActiveModel {
            provider_id: parts[0].to_string(),
            model_id: parts[1].to_string(),
        })
    } else {
        None
    }
}

/// Format an ActiveModel as "provider_id::model_id" string.
#[allow(dead_code)]
pub fn format_model_ref(model: &ActiveModel) -> String {
    format!("{}::{}", model.provider_id, model.model_id)
}

/// Resolve the ordered model chain for a given agent.
/// Returns (primary, fallbacks) where primary is the first model to try
/// and fallbacks are tried in order if primary fails.
///
/// Resolution logic:
/// 1. If the agent has a custom primary, use it; otherwise use global active_model
/// 2. If the agent has custom fallbacks, use them; otherwise use global fallback_models
pub fn resolve_model_chain(
    agent_model: &crate::agent_config::AgentModelConfig,
    store: &ProviderStore,
) -> (Option<ActiveModel>, Vec<ActiveModel>) {
    // Resolve primary
    let primary = agent_model
        .primary
        .as_ref()
        .and_then(|s| parse_model_ref(s))
        .or_else(|| store.active_model.clone());

    // Resolve fallbacks
    let fallbacks = if !agent_model.fallbacks.is_empty() {
        // Agent has custom fallbacks
        agent_model
            .fallbacks
            .iter()
            .filter_map(|s| parse_model_ref(s))
            .collect()
    } else {
        // Use global fallbacks
        store.fallback_models.clone()
    };

    (primary, fallbacks)
}

/// Find a ProviderConfig by provider_id from the store.
/// Only returns enabled providers.
pub fn find_provider<'a>(
    providers: &'a [ProviderConfig],
    provider_id: &str,
) -> Option<&'a ProviderConfig> {
    providers.iter().find(|p| p.id == provider_id && p.enabled)
}

// ── Helper: Create built-in Codex provider ────────────────────────

/// Create or update the built-in Codex provider with OAuth token info.
/// Returns the provider ID.
pub fn ensure_codex_provider(store: &mut ProviderStore) -> String {
    // Check if a Codex provider already exists
    if let Some(existing) = store
        .providers
        .iter()
        .find(|p| p.api_type == ApiType::Codex)
    {
        return existing.id.clone();
    }

    let codex_models = vec![
        ModelConfig {
            id: "gpt-5.4".into(),
            name: "GPT-5.4".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.3-codex".into(),
            name: "GPT-5.3 Codex".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.3-codex-spark".into(),
            name: "GPT-5.3 Codex Spark".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.2".into(),
            name: "GPT-5.2".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.2-codex".into(),
            name: "GPT-5.2 Codex".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.1".into(),
            name: "GPT-5.1".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.1-codex-max".into(),
            name: "GPT-5.1 Codex Max".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.1-codex-mini".into(),
            name: "GPT-5.1 Codex Mini".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
    ];

    let provider = ProviderConfig {
        id: uuid::Uuid::new_v4().to_string(),
        name: "ChatGPT (Codex)".into(),
        api_type: ApiType::Codex,
        base_url: ApiType::Codex.default_base_url().into(),
        api_key: String::new(), // OAuth, no API key
        models: codex_models,
        enabled: true,
        user_agent: super::types::default_user_agent(),
        thinking_style: ThinkingStyle::default(),
    };

    let id = provider.id.clone();
    store.providers.push(provider);
    id
}
