use crate::provider::ProviderConfig;
use crate::AppState;
use tauri::State;

// ── Provider Management Commands ──────────────────────────────────

#[tauri::command]
pub async fn get_providers(state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, String> {
    let store = state.config.lock().await;
    Ok(store.providers.clone())
}

#[tauri::command]
pub async fn add_provider(
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<ProviderConfig, String> {
    let mut store = state.config.lock().await;
    let new_provider = ProviderConfig::new(
        config.name,
        config.api_type,
        config.base_url,
        config.api_key,
    );
    // Add models from the incoming config
    let mut provider_with_models = new_provider;
    provider_with_models.models = config.models;

    let masked = provider_with_models.masked();
    store.providers.push(provider_with_models);
    oc_core::config::save_config(&store).map_err(|e| e.to_string())?;
    Ok(masked)
}

#[tauri::command]
pub async fn update_provider(
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.config.lock().await;
    if let Some(existing) = store.providers.iter_mut().find(|p| p.id == config.id) {
        existing.name = config.name;
        existing.api_type = config.api_type;
        existing.base_url = config.base_url;
        // Only update API key if a real key is provided (not the masked version)
        if !config.api_key.contains("...") && config.api_key != "****" {
            existing.api_key = config.api_key;
        }
        existing.models = config.models;
        existing.enabled = config.enabled;
        existing.user_agent = config.user_agent;
        existing.thinking_style = config.thinking_style;
        oc_core::config::save_config(&store).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err(format!("Provider not found: {}", config.id))
    }
}

#[tauri::command]
pub async fn reorder_providers(
    provider_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.config.lock().await;
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
    oc_core::config::save_config(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_provider(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.config.lock().await;
    let len_before = store.providers.len();
    store.providers.retain(|p| p.id != provider_id);
    if store.providers.len() == len_before {
        return Err(format!("Provider not found: {}", provider_id));
    }
    // Clear active model if it was from the deleted provider
    if let Some(ref active) = store.active_model {
        if active.provider_id == provider_id {
            store.active_model = None;
            *state.agent.lock().await = None;
        }
    }
    oc_core::config::save_config(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn has_providers(state: State<'_, AppState>) -> Result<bool, String> {
    let store = state.config.lock().await;
    Ok(!store.providers.is_empty())
}
