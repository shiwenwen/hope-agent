use crate::agent::{build_api_url, AssistantAgent};
use crate::provider::{self, ActiveModel, ApiType, AvailableModel};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_available_models(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, String> {
    let store = state.provider_store.lock().await;
    Ok(provider::build_available_models(&store.providers))
}

#[tauri::command]
pub async fn get_active_model(state: State<'_, AppState>) -> Result<Option<ActiveModel>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.active_model.clone())
}

/// Core logic for switching the active model. Usable from both Tauri commands
/// and internal callers (e.g. channel worker).
pub(crate) async fn set_active_model_core(
    provider_id: &str,
    model_id: &str,
    state: &AppState,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;

    // Find the provider
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider not found: {}", provider_id))?;

    // Verify model exists
    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(format!("Model not found: {}", model_id));
    }

    // For Codex, use stored token info
    if provider.api_type == ApiType::Codex {
        let token_info = state.codex_token.lock().await.clone();
        if let Some((access_token, account_id)) = token_info {
            let agent = AssistantAgent::new_openai(&access_token, &account_id, model_id);
            *state.agent.lock().await = Some(agent);
        }
    } else {
        // For other providers, create agent from config
        let agent = AssistantAgent::new_from_provider(provider, model_id);
        *state.agent.lock().await = Some(agent);
    }

    store.active_model = Some(ActiveModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    });
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_active_model(
    provider_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    set_active_model_core(&provider_id, &model_id, &state).await
}

#[tauri::command]
pub async fn get_fallback_models(state: State<'_, AppState>) -> Result<Vec<ActiveModel>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.fallback_models.clone())
}

#[tauri::command]
pub async fn set_fallback_models(
    models: Vec<ActiveModel>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    store.fallback_models = models;
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

// has_providers is in crud.rs
