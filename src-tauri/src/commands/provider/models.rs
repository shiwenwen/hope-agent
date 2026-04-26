use crate::agent::AssistantAgent;
use crate::commands::CmdError;
use crate::provider::{self, ActiveModel, ApiType, AvailableModel};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_available_models(
    _state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, CmdError> {
    Ok(provider::build_available_models(
        &ha_core::config::cached_config().providers,
    ))
}

#[tauri::command]
pub async fn get_active_model(
    _state: State<'_, AppState>,
) -> Result<Option<ActiveModel>, CmdError> {
    Ok(ha_core::config::cached_config().active_model.clone())
}

/// Core logic for switching the active model. Usable from both Tauri commands
/// and internal callers (e.g. channel worker).
pub(crate) async fn set_active_model_core(
    provider_id: &str,
    model_id: &str,
    state: &AppState,
) -> Result<(), CmdError> {
    let provider =
        ha_core::provider::set_active_model(provider_id.to_string(), model_id.to_string(), "ui")?;

    // For Codex, use stored token info; otherwise build agent from provider.
    if provider.api_type == ApiType::Codex {
        let token_info = state.codex_token.lock().await.clone();
        if let Some((access_token, account_id)) = token_info {
            let agent = AssistantAgent::new_openai(&access_token, &account_id, model_id);
            *state.agent.lock().await = Some(agent);
        }
    } else {
        let agent = AssistantAgent::new_from_provider(&provider, model_id);
        *state.agent.lock().await = Some(agent);
    }
    Ok(())
}

#[tauri::command]
pub async fn set_active_model(
    provider_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    set_active_model_core(&provider_id, &model_id, &state).await
}

#[tauri::command]
pub async fn get_fallback_models(
    _state: State<'_, AppState>,
) -> Result<Vec<ActiveModel>, CmdError> {
    Ok(ha_core::config::cached_config().fallback_models.clone())
}

#[tauri::command]
pub async fn set_fallback_models(
    models: Vec<ActiveModel>,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("fallback_models", "ui"), |store| {
        store.fallback_models = models;
        Ok(())
    })
    .map_err(Into::into)
}

// has_providers is in crud.rs
