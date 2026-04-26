use crate::commands::CmdError;
use crate::provider::ProviderConfig;
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
    ha_core::provider::add_provider(config, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn update_provider(
    config: ProviderConfig,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::provider::update_provider(config, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn reorder_providers(
    provider_ids: Vec<String>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::provider::reorder_providers(provider_ids, "ui").map_err(Into::into)
}

#[tauri::command]
pub async fn delete_provider(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    // Capture whether the active agent needs to be torn down, then persist.
    let active_was_removed = ha_core::provider::delete_provider(provider_id, "ui")?;

    if active_was_removed {
        *state.agent.lock().await = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn has_providers(_state: State<'_, AppState>) -> Result<bool, CmdError> {
    Ok(!ha_core::config::cached_config().providers.is_empty())
}
