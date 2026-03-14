mod agent;
mod oauth;
mod tools;

use agent::{AssistantAgent, CodexModel};
use oauth::TokenData;
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::State;
use serde::Serialize;

struct AppState {
    agent: Mutex<Option<AssistantAgent>>,
    auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    codex_model: Mutex<String>,
    reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    codex_token: Mutex<Option<(String, String)>>,  // (access_token, account_id)
}

#[derive(Serialize)]
struct CurrentSettings {
    model: String,
    reasoning_effort: String,
}

// ── Anthropic API Key Auth ────────────────────────────────────────

#[tauri::command]
async fn initialize_agent(
    api_key: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let agent = AssistantAgent::new_anthropic(&api_key);
    *state.agent.lock().await = Some(agent);
    Ok(())
}

// ── Codex OAuth Auth ──────────────────────────────────────────────

#[tauri::command]
async fn start_codex_auth(
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Reset previous auth result
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
        None => {
            // Still waiting for callback
            Ok(oauth::AuthStatus {
                authenticated: false,
                error: None,
            })
        }
        Some(Ok(_token)) => {
            Ok(oauth::AuthStatus {
                authenticated: true,
                error: None,
            })
        }
        Some(Err(e)) => {
            Ok(oauth::AuthStatus {
                authenticated: false,
                error: Some(e.to_string()),
            })
        }
    }
}

#[tauri::command]
async fn finalize_codex_auth(
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Take the auth result and initialize the agent with the token
    let token = {
        let mut lock = state.auth_result.lock().await;
        match lock.take() {
            Some(Ok(token)) => token,
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("Auth not complete yet".to_string()),
        }
    };

    // Extract account_id (should already be in token, but fallback to JWT parsing)
    let account_id = token.account_id
        .clone()
        .or_else(|| oauth::extract_account_id(&token.access_token))
        .ok_or_else(|| "Failed to extract account ID from token".to_string())?;

    let model = state.codex_model.lock().await.clone();
    let agent = AssistantAgent::new_openai(&token.access_token, &account_id, &model);
    *state.agent.lock().await = Some(agent);

    // Store token info for rebuilding agent on model change
    *state.codex_token.lock().await = Some((token.access_token.clone(), account_id));
    Ok(())
}

#[tauri::command]
async fn try_restore_session(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    match oauth::load_token() {
        Ok(Some(mut token)) => {
            // Check if token is expired and try to refresh
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
                            return Ok(false);
                        }
                    }
                } else {
                    log::warn!("Token expired and no refresh_token available");
                    let _ = oauth::clear_token();
                    return Ok(false);
                }
            }

            // Extract account_id
            let account_id = token.account_id
                .clone()
                .or_else(|| oauth::extract_account_id(&token.access_token));

            match account_id {
                Some(id) => {
                    let model = state.codex_model.lock().await.clone();
                    let agent = AssistantAgent::new_openai(&token.access_token, &id, &model);
                    *state.agent.lock().await = Some(agent);
                    // Store token info for rebuilding agent on model change
                    *state.codex_token.lock().await = Some((token.access_token.clone(), id));
                    Ok(true)
                }
                None => {
                    log::warn!("Failed to extract account_id from saved token");
                    let _ = oauth::clear_token();
                    Ok(false)
                }
            }
        }
        Ok(None) => Ok(false),
        Err(e) => {
            log::warn!("Failed to load saved token: {}", e);
            Ok(false)
        }
    }
}

#[tauri::command]
async fn logout_codex(
    state: State<'_, AppState>,
) -> Result<(), String> {
    *state.agent.lock().await = None;
    *state.codex_token.lock().await = None;
    oauth::clear_token().map_err(|e| e.to_string())?;
    Ok(())
}

// ── Chat ──────────────────────────────────────────────────────────

// ── Model & Reasoning Commands ────────────────────────────────────

#[tauri::command]
async fn get_codex_models() -> Result<Vec<CodexModel>, String> {
    Ok(agent::get_codex_models())
}

#[tauri::command]
async fn get_current_settings(
    state: State<'_, AppState>,
) -> Result<CurrentSettings, String> {
    let model = state.codex_model.lock().await.clone();
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
    // Validate model
    let valid = agent::get_codex_models().iter().any(|m| m.id == model);
    if !valid {
        return Err(format!("Unknown model: {}", model));
    }

    // Update saved model
    *state.codex_model.lock().await = model.clone();

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

// ── Chat ──────────────────────────────────────────────────────────

#[tauri::command]
async fn chat(
    message: String,
    on_event: tauri::ipc::Channel<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let effort = state.reasoning_effort.lock().await.clone();
    let effort_ref = if effort == "none" { None } else { Some(effort.as_str()) };
    let agent_lock = state.agent.lock().await;
    match agent_lock.as_ref() {
        Some(agent) => {
            agent.chat(&message, effort_ref, move |delta| {
                let _ = on_event.send(delta.to_string());
            }).await.map_err(|e| e.to_string())
        }
        None => Err("Agent not initialized. Please sign in first.".to_string()),
    }
}

// ── App Entry ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            codex_model: Mutex::new("gpt-5.4".to_string()),
            reasoning_effort: Mutex::new("medium".to_string()),
            codex_token: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            initialize_agent,
            start_codex_auth,
            check_auth_status,
            finalize_codex_auth,
            try_restore_session,
            logout_codex,
            get_codex_models,
            get_current_settings,
            set_codex_model,
            set_reasoning_effort,
            chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
