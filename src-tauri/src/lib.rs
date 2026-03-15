mod agent;
mod oauth;
mod paths;
mod provider;
mod tools;

use agent::AssistantAgent;
use oauth::TokenData;
use provider::{
    ActiveModel, ApiType, AvailableModel, ModelConfig, ProviderConfig, ProviderStore,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::State;
use serde::Serialize;

struct AppState {
    agent: Mutex<Option<AssistantAgent>>,
    auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    /// Provider configuration store
    provider_store: Mutex<ProviderStore>,
    /// Reasoning effort for Codex models
    reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    codex_token: Mutex<Option<(String, String)>>,  // (access_token, account_id)
}

// ── Provider Management Commands ──────────────────────────────────

#[tauri::command]
async fn get_providers(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderConfig>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.providers.iter().map(|p| p.masked()).collect())
}

#[tauri::command]
async fn add_provider(
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<ProviderConfig, String> {
    let mut store = state.provider_store.lock().await;
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
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(masked)
}

#[tauri::command]
async fn update_provider(
    config: ProviderConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
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
        provider::save_store(&store).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err(format!("Provider not found: {}", config.id))
    }
}

#[tauri::command]
async fn delete_provider(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
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
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn test_provider(
    config: ProviderConfig,
) -> Result<String, String> {
    // Send a minimal request to verify the provider is reachable
    let client = reqwest::Client::new();
    match config.api_type {
        ApiType::Anthropic => {
            let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "model": "claude-sonnet-4-6",
                    "max_tokens": 10,
                    "messages": [{ "role": "user", "content": "Hi" }]
                }))
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            if resp.status().is_success() {
                Ok("连接成功！".to_string())
            } else {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                // 400 with model error is ok — means auth works
                if status == 400 || status == 404 {
                    Ok("认证成功（模型可能需要调整）".to_string())
                } else {
                    Err(format!("API 错误 ({}): {}", status, body))
                }
            }
        }
        ApiType::OpenaiChat | ApiType::OpenaiResponses => {
            let url = format!("{}/v1/models", config.base_url.trim_end_matches('/'));
            let mut req = client.get(&url);
            if !config.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", config.api_key));
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            if resp.status().is_success() {
                Ok("连接成功！".to_string())
            } else {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Err(format!("API 错误 ({}): {}", status, body))
            }
        }
        ApiType::Codex => {
            Ok("Codex 使用 OAuth 认证，无需测试 API Key".to_string())
        }
    }
}

#[tauri::command]
async fn get_available_models(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, String> {
    let store = state.provider_store.lock().await;
    Ok(provider::build_available_models(&store.providers))
}

#[tauri::command]
async fn get_active_model(
    state: State<'_, AppState>,
) -> Result<Option<ActiveModel>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.active_model.clone())
}

#[tauri::command]
async fn set_active_model(
    provider_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;

    // Find the provider
    let provider = store.providers.iter().find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider not found: {}", provider_id))?;

    // Verify model exists
    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(format!("Model not found: {}", model_id));
    }

    // For Codex, use stored token info
    if provider.api_type == ApiType::Codex {
        let token_info = state.codex_token.lock().await.clone();
        if let Some((access_token, account_id)) = token_info {
            let agent = AssistantAgent::new_openai(&access_token, &account_id, &model_id);
            *state.agent.lock().await = Some(agent);
        }
    } else {
        // For other providers, create agent from config
        let agent = AssistantAgent::new_from_provider(provider, &model_id);
        *state.agent.lock().await = Some(agent);
    }

    store.active_model = Some(ActiveModel {
        provider_id,
        model_id,
    });
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn has_providers(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let store = state.provider_store.lock().await;
    Ok(!store.providers.is_empty())
}

// ── Anthropic API Key Auth (legacy, creates a provider) ──────────

#[tauri::command]
async fn initialize_agent(
    api_key: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
async fn start_codex_auth(
    state: State<'_, AppState>,
) -> Result<(), String> {
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
async fn check_auth_status(
    state: State<'_, AppState>,
) -> Result<oauth::AuthStatus, String> {
    let lock = state.auth_result.lock().await;
    match lock.as_ref() {
        None => Ok(oauth::AuthStatus { authenticated: false, error: None }),
        Some(Ok(_)) => Ok(oauth::AuthStatus { authenticated: true, error: None }),
        Some(Err(e)) => Ok(oauth::AuthStatus { authenticated: false, error: Some(e.to_string()) }),
    }
}

#[tauri::command]
async fn finalize_codex_auth(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let token = {
        let mut lock = state.auth_result.lock().await;
        match lock.take() {
            Some(Ok(token)) => token,
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("Auth not complete yet".to_string()),
        }
    };

    let account_id = token.account_id
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
async fn try_restore_session(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // First, load provider store from disk
    {
        let mut store = state.provider_store.lock().await;
        match provider::load_store() {
            Ok(loaded) => *store = loaded,
            Err(e) => log::warn!("Failed to load provider store: {}", e),
        }
    }

    // Try to restore Codex OAuth session
    match oauth::load_token() {
        Ok(Some(mut token)) => {
            if oauth::is_token_expired(&token) {
                log::info!("Saved token is expired, attempting refresh...");
                if let Some(refresh_token) = &token.refresh_token {
                    match oauth::refresh_access_token(refresh_token).await {
                        Ok(new_token) => {
                            log::info!("Token refreshed successfully");
                            token = new_token;
                        }
                        Err(e) => {
                            log::warn!("Token refresh failed: {}, clearing saved session", e);
                            let _ = oauth::clear_token();
                            return Ok(try_restore_non_codex_session(&state).await);
                        }
                    }
                } else {
                    log::warn!("Token expired and no refresh_token available");
                    let _ = oauth::clear_token();
                    return Ok(try_restore_non_codex_session(&state).await);
                }
            }

            let account_id = token.account_id
                .clone()
                .or_else(|| oauth::extract_account_id(&token.access_token));

            match account_id {
                Some(id) => {
                    // Ensure Codex provider exists
                    let model_id;
                    {
                        let mut store = state.provider_store.lock().await;
                        let codex_provider_id = provider::ensure_codex_provider(&mut store);

                        // Use active model from store, or default
                        model_id = store.active_model
                            .as_ref()
                            .filter(|am| am.provider_id == codex_provider_id)
                            .map(|am| am.model_id.clone())
                            .unwrap_or_else(|| "gpt-5.4".to_string());

                        store.active_model = Some(ActiveModel {
                            provider_id: codex_provider_id,
                            model_id: model_id.clone(),
                        });
                        provider::save_store(&store).map_err(|e| e.to_string())?;
                    }

                    let agent = AssistantAgent::new_openai(&token.access_token, &id, &model_id);
                    *state.agent.lock().await = Some(agent);
                    *state.codex_token.lock().await = Some((token.access_token.clone(), id));
                    Ok(true)
                }
                None => {
                    log::warn!("Failed to extract account_id from saved token");
                    let _ = oauth::clear_token();
                    Ok(try_restore_non_codex_session(&state).await)
                }
            }
        }
        Ok(None) => Ok(try_restore_non_codex_session(&state).await),
        Err(e) => {
            log::warn!("Failed to load saved token: {}", e);
            Ok(try_restore_non_codex_session(&state).await)
        }
    }
}

/// Try to restore from a non-Codex provider (API key providers)
async fn try_restore_non_codex_session(state: &State<'_, AppState>) -> bool {
    let store = state.provider_store.lock().await;
    if let Some(ref active) = store.active_model {
        if let Some(provider) = store.providers.iter().find(|p| p.id == active.provider_id && p.enabled) {
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
async fn logout_codex(
    state: State<'_, AppState>,
) -> Result<(), String> {
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
struct CurrentSettings {
    model: String,
    reasoning_effort: String,
}

#[tauri::command]
async fn get_codex_models() -> Result<Vec<agent::CodexModel>, String> {
    Ok(agent::get_codex_models())
}

#[tauri::command]
async fn get_current_settings(
    state: State<'_, AppState>,
) -> Result<CurrentSettings, String> {
    let store = state.provider_store.lock().await;
    let model = store.active_model
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
async fn set_codex_model(
    model: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
async fn set_reasoning_effort(
    effort: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let valid = ["none", "low", "medium", "high", "xhigh"];
    if !valid.contains(&effort.as_str()) {
        return Err(format!("Invalid reasoning effort: {}. Valid: {:?}", effort, valid));
    }
    *state.reasoning_effort.lock().await = effort;
    Ok(())
}

use agent::Attachment;

#[tauri::command]
async fn chat(
    message: String,
    attachments: Vec<Attachment>,
    on_event: tauri::ipc::Channel<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let effort = state.reasoning_effort.lock().await.clone();
    let effort_ref = if effort == "none" { None } else { Some(effort.as_str()) };
    let agent_lock = state.agent.lock().await;
    match agent_lock.as_ref() {
        Some(agent) => {
            agent.chat(&message, &attachments, effort_ref, move |delta| {
                let _ = on_event.send(delta.to_string());
            }).await.map_err(|e| e.to_string())
        }
        None => Err("Agent not initialized. Please sign in first.".to_string()),
    }
}

// ── App Entry ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize directory structure and migrate legacy data
    if let Err(e) = paths::ensure_dirs() {
        log::error!("Failed to initialize data directories: {}", e);
    }
    if let Err(e) = paths::migrate_legacy_data() {
        log::error!("Failed to migrate legacy data: {}", e);
    }

    // Load provider store at startup
    let initial_store = provider::load_store().unwrap_or_default();

    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .manage(AppState {
            agent: Mutex::new(None),
            auth_result: Arc::new(Mutex::new(None)),
            provider_store: Mutex::new(initial_store),
            reasoning_effort: Mutex::new("medium".to_string()),
            codex_token: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            // Provider management
            get_providers,
            add_provider,
            update_provider,
            delete_provider,
            test_provider,
            get_available_models,
            get_active_model,
            set_active_model,
            has_providers,
            // Legacy auth
            initialize_agent,
            start_codex_auth,
            check_auth_status,
            finalize_codex_auth,
            try_restore_session,
            logout_codex,
            // Model & settings (legacy)
            get_codex_models,
            get_current_settings,
            set_codex_model,
            set_reasoning_effort,
            // Chat
            chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
