use crate::agent::{self, AssistantAgent};
use crate::commands::CmdError;
use crate::oauth;
use crate::provider::{self, ActiveModelUpdate, ApiType, ProviderConfig};
use crate::AppState;
use ha_core::{app_info, app_warn};
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub async fn initialize_agent(api_key: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    let provider = ProviderConfig::new_default_anthropic(api_key);
    let model_id = provider.models[0].id.clone();
    let agent = AssistantAgent::try_new_from_provider(&provider, &model_id).await?;

    ha_core::provider::add_and_activate_provider(provider, model_id, "onboarding")?;
    *state.agent.lock().await = Some(agent);
    Ok(())
}

// ── Codex OAuth Auth ──────────────────────────────────────────────

#[tauri::command]
pub async fn start_codex_auth(state: State<'_, AppState>) -> Result<(), CmdError> {
    {
        let mut lock = state.auth_result.lock().await;
        *lock = None;
    }
    let auth_result = state.auth_result.clone();
    oauth::start_oauth_flow(auth_result)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn check_auth_status(state: State<'_, AppState>) -> Result<oauth::AuthStatus, CmdError> {
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
pub async fn finalize_codex_auth(state: State<'_, AppState>) -> Result<(), CmdError> {
    let token = {
        let mut lock = state.auth_result.lock().await;
        match lock.take() {
            Some(Ok(token)) => token,
            Some(Err(e)) => return Err(e.into()),
            None => return Err(CmdError::msg("Auth not complete yet")),
        }
    };

    let account_id = token
        .account_id
        .clone()
        .or_else(|| oauth::extract_account_id(&token.access_token))
        .ok_or_else(|| CmdError::msg("Failed to extract account ID from token"))?;

    // Ensure Codex provider exists in store
    let default_model_id = "gpt-5.4".to_string();
    let model_for_agent = default_model_id.clone();
    provider::ensure_codex_provider_persisted(
        ActiveModelUpdate::Always(model_for_agent.clone()),
        "oauth-finalize",
    )?;

    let agent = AssistantAgent::new_openai(&token.access_token, &account_id, &default_model_id);
    *state.agent.lock().await = Some(agent);
    *state.codex_token.lock().await = Some((token.access_token.clone(), account_id));
    Ok(())
}

#[tauri::command]
pub async fn try_restore_session(state: State<'_, AppState>) -> Result<bool, CmdError> {
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
                    // Ensure Codex provider exists and fall back to Codex
                    // `gpt-5.4` only when no active_model is set. Respect any
                    // already-chosen active model (including non-Codex).
                    provider::ensure_codex_provider_persisted(
                        ActiveModelUpdate::IfMissing("gpt-5.4".to_string()),
                        "session-restore",
                    )?;

                    // Create agent based on the active model's provider type
                    {
                        let store = ha_core::config::cached_config();
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
                                    let agent = AssistantAgent::try_new_from_provider(
                                        provider,
                                        &active.model_id,
                                    )
                                    .await?;
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
    let store = ha_core::config::cached_config();
    if let Some(ref active) = store.active_model {
        if let Some(provider) = store
            .providers
            .iter()
            .find(|p| p.id == active.provider_id && p.enabled)
        {
            if provider.api_type != ApiType::Codex {
                let provider_clone = provider.clone();
                let model_id = active.model_id.clone();
                drop(store);
                let agent =
                    match AssistantAgent::try_new_from_provider(&provider_clone, &model_id).await {
                        Ok(agent) => agent,
                        Err(e) => {
                            app_warn!("app", "session", "Failed to restore provider agent: {}", e);
                            return false;
                        }
                    };
                *state.agent.lock().await = Some(agent);
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn logout_codex(state: State<'_, AppState>) -> Result<(), CmdError> {
    *state.agent.lock().await = None;
    *state.codex_token.lock().await = None;

    provider::delete_providers_by_api_type(ApiType::Codex, "ui")?;

    oauth::clear_token()?;
    Ok(())
}

// ── Model & Reasoning Commands ────────────────────────────────────

#[derive(Serialize)]
pub struct CurrentSettings {
    model: String,
    reasoning_effort: String,
}

#[tauri::command]
pub async fn get_codex_models() -> Result<Vec<agent::CodexModel>, CmdError> {
    Ok(agent::get_codex_models())
}

#[tauri::command]
pub async fn get_current_settings(state: State<'_, AppState>) -> Result<CurrentSettings, CmdError> {
    let model = ha_core::config::cached_config()
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
pub async fn set_codex_model(model: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    if !agent::is_valid_codex_model(&model) {
        return Err(CmdError::msg(format!("Unknown model: {}", model)));
    }

    // Update active model in store
    let model_for_mut = model.clone();
    ha_core::config::mutate_config(("active_model", "set-codex-model"), |store| {
        if let Some(ref mut active) = store.active_model {
            active.model_id = model_for_mut;
        }
        Ok(())
    })?;

    // Rebuild agent with new model if authenticated
    let token_info = state.codex_token.lock().await.clone();
    if let Some((access_token, account_id)) = token_info {
        let agent = AssistantAgent::new_openai(&access_token, &account_id, &model);
        *state.agent.lock().await = Some(agent);
    }

    Ok(())
}

/// Core logic for setting reasoning effort. Usable from both Tauri commands
/// and internal callers (e.g. channel worker).
pub(crate) async fn set_reasoning_effort_core(
    effort: &str,
    state: &AppState,
) -> Result<(), CmdError> {
    let valid = ["none", "low", "medium", "high", "xhigh"];
    if !valid.contains(&effort) {
        return Err(CmdError::msg(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort, valid
        )));
    }
    *state.reasoning_effort.lock().await = effort.to_string();
    Ok(())
}

#[tauri::command]
pub async fn set_reasoning_effort(
    effort: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    set_reasoning_effort_core(&effort, &state).await
}
