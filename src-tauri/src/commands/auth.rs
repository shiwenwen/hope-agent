use crate::agent::{self, AssistantAgent};
use crate::oauth;
use crate::provider::{self, ActiveModel, ApiType, ModelConfig, ProviderConfig};
use crate::AppState;
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub async fn initialize_agent(api_key: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;

    // Create an Anthropic provider
    let mut provider = ProviderConfig::new(
        "Anthropic".to_string(),
        ApiType::Anthropic,
        "https://api.anthropic.com".to_string(),
        api_key.clone(),
    );
    provider.models.push(ModelConfig {
        id: "claude-sonnet-4-6".to_string(),
        name: "Claude Sonnet 4.6".to_string(),
        input_types: vec!["text".to_string(), "image".to_string()],
        context_window: 200_000,
        max_tokens: 8192,
        reasoning: false,
        cost_input: 3.0,
        cost_output: 15.0,
    });

    let provider_id = provider.id.clone();
    let model_id = "claude-sonnet-4-6".to_string();

    let agent = AssistantAgent::new_from_provider(&provider, &model_id);
    store.providers.push(provider);
    store.active_model = Some(ActiveModel {
        provider_id,
        model_id,
    });
    provider::save_store(&store).map_err(|e| e.to_string())?;
    *state.agent.lock().await = Some(agent);
    Ok(())
}

// ── Codex OAuth Auth ──────────────────────────────────────────────

#[tauri::command]
pub async fn start_codex_auth(state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut lock = state.auth_result.lock().await;
        *lock = None;
    }
    let auth_result = state.auth_result.clone();
    oauth::start_oauth_flow(auth_result)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_auth_status(state: State<'_, AppState>) -> Result<oauth::AuthStatus, String> {
    let lock = state.auth_result.lock().await;
    match lock.as_ref() {
        None => Ok(oauth::AuthStatus {
            authenticated: false,
            error: None,
        }),
        Some(Ok(_)) => Ok(oauth::AuthStatus {
            authenticated: true,
            error: None,
        }),
        Some(Err(e)) => Ok(oauth::AuthStatus {
            authenticated: false,
            error: Some(e.to_string()),
        }),
    }
}

#[tauri::command]
pub async fn finalize_codex_auth(state: State<'_, AppState>) -> Result<(), String> {
    let token = {
        let mut lock = state.auth_result.lock().await;
        match lock.take() {
            Some(Ok(token)) => token,
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("Auth not complete yet".to_string()),
        }
    };

    let account_id = token
        .account_id
        .clone()
        .or_else(|| oauth::extract_account_id(&token.access_token))
        .ok_or_else(|| "Failed to extract account ID from token".to_string())?;

    // Ensure Codex provider exists in store
    let default_model_id;
    {
        let mut store = state.provider_store.lock().await;
        let codex_provider_id = provider::ensure_codex_provider(&mut store);
        default_model_id = "gpt-5.4".to_string();
        store.active_model = Some(ActiveModel {
            provider_id: codex_provider_id,
            model_id: default_model_id.clone(),
        });
        provider::save_store(&store).map_err(|e| e.to_string())?;
    }

    let agent = AssistantAgent::new_openai(&token.access_token, &account_id, &default_model_id);
    *state.agent.lock().await = Some(agent);
    *state.codex_token.lock().await = Some((token.access_token.clone(), account_id));
    Ok(())
}

#[tauri::command]
pub async fn try_restore_session(state: State<'_, AppState>) -> Result<bool, String> {
    // First, load provider store from disk
    {
        let mut store = state.provider_store.lock().await;
        match provider::load_store() {
            Ok(loaded) => *store = loaded,
            Err(e) => app_warn!("app", "session", "Failed to load provider store: {}", e),
        }
    }

    // Try to restore Codex OAuth session
    match oauth::load_token() {
        Ok(Some(mut token)) => {
            if oauth::is_token_expired(&token) {
                app_info!(
                    "app",
                    "session",
                    "Saved token is expired, attempting refresh..."
                );
                if let Some(refresh_token) = &token.refresh_token {
                    match oauth::refresh_access_token(refresh_token).await {
                        Ok(new_token) => {
                            app_info!("app", "session", "Token refreshed successfully");
                            token = new_token;
                        }
                        Err(e) => {
                            app_warn!(
                                "app",
                                "session",
                                "Token refresh failed: {}, clearing saved session",
                                e
                            );
                            let _ = oauth::clear_token();
                            return Ok(try_restore_non_codex_session(&state).await);
                        }
                    }
                } else {
                    app_warn!(
                        "app",
                        "session",
                        "Token expired and no refresh_token available"
                    );
                    let _ = oauth::clear_token();
                    return Ok(try_restore_non_codex_session(&state).await);
                }
            }

            let account_id = token
                .account_id
                .clone()
                .or_else(|| oauth::extract_account_id(&token.access_token));

            match account_id {
                Some(id) => {
                    // Ensure Codex provider exists
                    let model_id;
                    {
                        let mut store = state.provider_store.lock().await;
                        let codex_provider_id = provider::ensure_codex_provider(&mut store);

                        // Determine which model to activate:
                        // - If user already has a saved active_model (even non-Codex), respect it.
                        // - Only default to Codex gpt-5.4 if no active_model is set at all.
                        if store.active_model.is_none() {
                            model_id = "gpt-5.4".to_string();
                            store.active_model = Some(ActiveModel {
                                provider_id: codex_provider_id,
                                model_id: model_id.clone(),
                            });
                            provider::save_store(&store).map_err(|e| e.to_string())?;
                        } else {
                            _ = store.active_model.as_ref().unwrap().model_id.clone();
                        }
                    }

                    // Create agent based on the active model's provider type
                    {
                        let store = state.provider_store.lock().await;
                        if let Some(ref active) = store.active_model {
                            let active_provider =
                                store.providers.iter().find(|p| p.id == active.provider_id);
                            if let Some(provider) = active_provider {
                                if provider.api_type == ApiType::Codex {
                                    let agent = AssistantAgent::new_openai(
                                        &token.access_token,
                                        &id,
                                        &active.model_id,
                                    );
                                    *state.agent.lock().await = Some(agent);
                                } else {
                                    let agent = AssistantAgent::new_from_provider(
                                        provider,
                                        &active.model_id,
                                    );
                                    *state.agent.lock().await = Some(agent);
                                }
                            }
                        }
                    }
                    *state.codex_token.lock().await = Some((token.access_token.clone(), id));
                    Ok(true)
                }
                None => {
                    app_warn!(
                        "app",
                        "session",
                        "Failed to extract account_id from saved token"
                    );
                    let _ = oauth::clear_token();
                    Ok(try_restore_non_codex_session(&state).await)
                }
            }
        }
        Ok(None) => Ok(try_restore_non_codex_session(&state).await),
        Err(e) => {
            app_warn!("app", "session", "Failed to load saved token: {}", e);
            Ok(try_restore_non_codex_session(&state).await)
        }
    }
}

/// Try to restore from a non-Codex provider (API key providers)
pub(crate) async fn try_restore_non_codex_session(state: &State<'_, AppState>) -> bool {
    let store = state.provider_store.lock().await;
    if let Some(ref active) = store.active_model {
        if let Some(provider) = store
            .providers
            .iter()
            .find(|p| p.id == active.provider_id && p.enabled)
        {
            if provider.api_type != ApiType::Codex {
                // Need to drop store lock before acquiring agent lock
                let provider_clone = provider.clone();
                let model_id = active.model_id.clone();
                drop(store);
                let agent = AssistantAgent::new_from_provider(&provider_clone, &model_id);
                *state.agent.lock().await = Some(agent);
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn logout_codex(state: State<'_, AppState>) -> Result<(), String> {
    *state.agent.lock().await = None;
    *state.codex_token.lock().await = None;

    // Remove Codex provider from store
    {
        let mut store = state.provider_store.lock().await;
        store.providers.retain(|p| p.api_type != ApiType::Codex);
        if let Some(ref active) = store.active_model {
            // If active model was from a Codex provider, clear it
            if !store.providers.iter().any(|p| p.id == active.provider_id) {
                store.active_model = None;
            }
        }
        provider::save_store(&store).map_err(|e| e.to_string())?;
    }

    oauth::clear_token().map_err(|e| e.to_string())?;
    Ok(())
}

// ── Model & Reasoning Commands ────────────────────────────────────

#[derive(Serialize)]
pub struct CurrentSettings {
    model: String,
    reasoning_effort: String,
}

#[tauri::command]
pub async fn get_codex_models() -> Result<Vec<agent::CodexModel>, String> {
    Ok(agent::get_codex_models())
}

#[tauri::command]
pub async fn get_current_settings(state: State<'_, AppState>) -> Result<CurrentSettings, String> {
    let store = state.provider_store.lock().await;
    let model = store
        .active_model
        .as_ref()
        .map(|am| am.model_id.clone())
        .unwrap_or_else(|| "gpt-5.4".to_string());
    let effort = state.reasoning_effort.lock().await.clone();
    Ok(CurrentSettings {
        model,
        reasoning_effort: effort,
    })
}

#[tauri::command]
pub async fn set_codex_model(model: String, state: State<'_, AppState>) -> Result<(), String> {
    let valid = agent::get_codex_models().iter().any(|m| m.id == model);
    if !valid {
        return Err(format!("Unknown model: {}", model));
    }

    // Update active model in store
    {
        let mut store = state.provider_store.lock().await;
        if let Some(ref mut active) = store.active_model {
            active.model_id = model.clone();
        }
        provider::save_store(&store).map_err(|e| e.to_string())?;
    }

    // Rebuild agent with new model if authenticated
    let token_info = state.codex_token.lock().await.clone();
    if let Some((access_token, account_id)) = token_info {
        let agent = AssistantAgent::new_openai(&access_token, &account_id, &model);
        *state.agent.lock().await = Some(agent);
    }

    Ok(())
}

#[tauri::command]
pub async fn set_reasoning_effort(
    effort: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let valid = ["none", "low", "medium", "high", "xhigh"];
    if !valid.contains(&effort.as_str()) {
        return Err(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort, valid
        ));
    }
    *state.reasoning_effort.lock().await = effort;
    Ok(())
}
