use crate::commands::CmdError;
use crate::provider::{self, ProviderConfig};
use crate::AppState;
use tauri::State;

// ── Provider Management Commands ──────────────────────────────────

#[tauri::command]
pub async fn get_providers(_state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, CmdError> {
    Ok(ha_core::config::cached_config().providers.clone())
}

#[tauri::command]
pub async fn add_provider(
    config: ProviderConfig,
    _state: State<'_, AppState>,
) -> Result<ProviderConfig, CmdError> {
    let new_provider = ProviderConfig::new(
        config.name,
        config.api_type,
        config.base_url,
        config.api_key,
    );
    // Add models and auth profiles from the incoming config
    let mut provider_with_models = new_provider;
    provider_with_models.models = config.models;
    provider_with_models.auth_profiles = config.auth_profiles;
    provider_with_models.thinking_style = config.thinking_style;
    provider_with_models.allow_private_network = config.allow_private_network;

    let masked = provider_with_models.masked();
    ha_core::config::mutate_config(("providers.add", "ui"), |store| {
        store.providers.push(provider_with_models);
        Ok(())
    })?;
    Ok(masked)
}

#[tauri::command]
pub async fn update_provider(
    config: ProviderConfig,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("providers.update", "ui"), |store| {
        let Some(existing) = store.providers.iter_mut().find(|p| p.id == config.id) else {
            return Err(anyhow::anyhow!("Provider not found: {}", config.id));
        };
        existing.name = config.name;
        existing.api_type = config.api_type;
        existing.base_url = config.base_url;
        // Only update API key if a real key is provided (not the masked version)
        if !provider::is_masked_key(&config.api_key) {
            existing.api_key = config.api_key;
        }
        // Merge auth profile keys: preserve real keys when incoming is masked
        existing.auth_profiles =
            provider::merge_profile_keys(&existing.auth_profiles, &config.auth_profiles);
        existing.models = config.models;
        existing.enabled = config.enabled;
        existing.user_agent = config.user_agent;
        existing.thinking_style = config.thinking_style;
        existing.allow_private_network = config.allow_private_network;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn reorder_providers(
    provider_ids: Vec<String>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("providers.reorder", "ui"), |store| {
        let mut reordered = Vec::with_capacity(provider_ids.len());
        for id in &provider_ids {
            if let Some(p) = store.providers.iter().find(|p| &p.id == id) {
                reordered.push(p.clone());
            }
        }
        // Append any providers not in the list (safety)
        for p in &store.providers {
            if !provider_ids.contains(&p.id) {
                reordered.push(p.clone());
            }
        }
        store.providers = reordered;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_provider(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    // Capture whether the active agent needs to be torn down, then persist.
    let active_was_removed = ha_core::config::mutate_config(("providers.delete", "ui"), |store| {
        let len_before = store.providers.len();
        store.providers.retain(|p| p.id != provider_id);
        if store.providers.len() == len_before {
            return Err(anyhow::anyhow!("Provider not found: {}", provider_id));
        }
        let removed_active = store
            .active_model
            .as_ref()
            .map(|am| am.provider_id == provider_id)
            .unwrap_or(false);
        if removed_active {
            store.active_model = None;
        }
        Ok(removed_active)
    })?;

    if active_was_removed {
        *state.agent.lock().await = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn has_providers(_state: State<'_, AppState>) -> Result<bool, CmdError> {
    Ok(!ha_core::config::cached_config().providers.is_empty())
}
