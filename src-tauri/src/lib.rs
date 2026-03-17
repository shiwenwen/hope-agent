mod agent;
mod agent_config;
mod agent_loader;
mod oauth;
mod paths;
mod process_registry;
mod provider;
mod sandbox;
mod skills;
mod system_prompt;
mod tools;
mod user_config;

use agent::AssistantAgent;
use oauth::TokenData;
use provider::{
    ActiveModel, ApiType, AvailableModel, ModelConfig, ProviderConfig, ProviderStore,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::State;
use serde::Serialize;

static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Get stored AppHandle for global event emission (e.g., command approval)
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

struct AppState {
    agent: Mutex<Option<AssistantAgent>>,
    auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    /// Provider configuration store
    provider_store: Mutex<ProviderStore>,
    /// Reasoning effort for Codex models
    reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    codex_token: Mutex<Option<(String, String)>>,  // (access_token, account_id)
    /// Currently active agent ID
    current_agent_id: Mutex<String>,
}

// ── Provider Management Commands ──────────────────────────────────

#[tauri::command]
async fn get_providers(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderConfig>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.providers.clone())
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
        existing.user_agent = config.user_agent;
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
    use std::time::{Duration, Instant};

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(&config.user_agent)
        .build()
        .map_err(|e| format!("Client error: {}", e))?;

    let base = config.base_url.trim_end_matches('/');
    let has_version_suffix = base.ends_with("/v1") || base.ends_with("/v2") || base.ends_with("/v3");
    let mut steps: Vec<serde_json::Value> = Vec::new();
    let total_start = Instant::now();

    // Helper to build final JSON result
    macro_rules! build_result {
        ($success:expr, $msg:expr, $url:expr, $status:expr, $auth:expr) => {
            serde_json::to_string(&serde_json::json!({
                "success": $success,
                "message": $msg,
                "url": $url,
                "status": $status,
                "latencyMs": total_start.elapsed().as_millis() as u64,
                "auth": $auth,
                "steps": steps,
            })).unwrap_or_default()
        };
    }

    match config.api_type {
        ApiType::Anthropic => {
            let url = if has_version_suffix {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            };
            let body = serde_json::json!({
                "model": "test-model", "max_tokens": 1,
                "messages": [{ "role": "user", "content": "Hi" }]
            });

            // Try x-api-key
            let t = Instant::now();
            let resp = client.post(&url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body).send().await
                .map_err(|e| build_result!(false, format!("连接失败: {}", e), &url, 0, "x-api-key"))?;
            let status = resp.status().as_u16();
            steps.push(serde_json::json!({"endpoint": &url, "method": "POST", "auth": "x-api-key", "status": status, "latencyMs": t.elapsed().as_millis() as u64}));

            if resp.status().is_success() || status == 400 || status == 404 {
                return Ok(build_result!(true, if status == 200 { "连接成功" } else { "认证成功（模型名需调整）" }, &url, status, "x-api-key"));
            }

            // Fallback: Bearer auth
            if status == 401 || status == 403 {
                let t2 = Instant::now();
                let resp2 = client.post(&url)
                    .header("Authorization", format!("Bearer {}", config.api_key))
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body).send().await
                    .map_err(|e| build_result!(false, format!("连接失败: {}", e), &url, 0, "Bearer"))?;
                let s2 = resp2.status().as_u16();
                steps.push(serde_json::json!({"endpoint": &url, "method": "POST", "auth": "Bearer", "status": s2, "latencyMs": t2.elapsed().as_millis() as u64}));

                if resp2.status().is_success() || s2 == 400 || s2 == 404 {
                    return Ok(build_result!(true, "连接成功（Bearer 认证）", &url, s2, "Bearer"));
                }
                let detail = resp2.text().await.unwrap_or_default();
                return Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("认证失败 ({})", s2), "detail": detail,
                    "url": &url, "status": s2, "latencyMs": total_start.elapsed().as_millis() as u64, "steps": steps,
                })).unwrap_or_default());
            }

            let detail = resp.text().await.unwrap_or_default();
            Err(serde_json::to_string(&serde_json::json!({
                "success": false, "message": format!("API 错误 ({})", status), "detail": detail,
                "url": &url, "status": status, "latencyMs": total_start.elapsed().as_millis() as u64, "steps": steps,
            })).unwrap_or_default())
        }
        ApiType::OpenaiChat | ApiType::OpenaiResponses => {
            let models_url = if has_version_suffix { format!("{}/models", base) } else { format!("{}/v1/models", base) };
            let t = Instant::now();
            let mut req = client.get(&models_url);
            if !config.api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", config.api_key)); }
            let resp = req.send().await
                .map_err(|e| build_result!(false, format!("连接失败: {}", e), &models_url, 0, "Bearer"))?;
            let status = resp.status().as_u16();
            steps.push(serde_json::json!({"endpoint": &models_url, "method": "GET", "status": status, "latencyMs": t.elapsed().as_millis() as u64}));

            if resp.status().is_success() {
                return Ok(build_result!(true, "连接成功", &models_url, status, "Bearer"));
            }
            if status == 401 || status == 403 {
                let detail = resp.text().await.unwrap_or_default();
                return Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("认证失败 ({})", status), "detail": detail,
                    "url": &models_url, "status": status, "latencyMs": total_start.elapsed().as_millis() as u64, "steps": steps,
                })).unwrap_or_default());
            }

            // Fallback: chat/completions
            let chat_url = if has_version_suffix { format!("{}/chat/completions", base) } else { format!("{}/v1/chat/completions", base) };
            let t2 = Instant::now();
            let mut chat_req = client.post(&chat_url)
                .header("content-type", "application/json")
                .json(&serde_json::json!({"model": "test", "max_tokens": 1, "messages": [{"role": "user", "content": "Hi"}]}));
            if !config.api_key.is_empty() { chat_req = chat_req.header("Authorization", format!("Bearer {}", config.api_key)); }

            match chat_req.send().await {
                Ok(chat_resp) => {
                    let cs = chat_resp.status().as_u16();
                    steps.push(serde_json::json!({"endpoint": &chat_url, "method": "POST", "status": cs, "latencyMs": t2.elapsed().as_millis() as u64}));
                    if chat_resp.status().is_success() || cs == 400 || cs == 404 {
                        Ok(build_result!(true, if cs == 200 { "连接成功" } else { "认证成功（模型名需调整）" }, &chat_url, cs, "Bearer"))
                    } else if cs == 401 || cs == 403 {
                        let detail = chat_resp.text().await.unwrap_or_default();
                        Err(serde_json::to_string(&serde_json::json!({
                            "success": false, "message": format!("认证失败 ({})", cs), "detail": detail,
                            "url": &chat_url, "status": cs, "latencyMs": total_start.elapsed().as_millis() as u64, "steps": steps,
                        })).unwrap_or_default())
                    } else {
                        Ok(build_result!(true, "连接成功（不支持模型列表查询）", &chat_url, cs, "Bearer"))
                    }
                }
                Err(e) => {
                    steps.push(serde_json::json!({"endpoint": &chat_url, "method": "POST", "error": format!("{}", e), "latencyMs": t2.elapsed().as_millis() as u64}));
                    Err(build_result!(false, format!("连接失败: {}", e), &chat_url, 0, ""))
                }
            }
        }
        ApiType::Codex => {
            Ok(build_result!(true, "Codex 使用 OAuth 认证，无需测试", "", 0, "OAuth"))
        }
    }
}

#[tauri::command]
async fn test_model(
    config: ProviderConfig,
    model_id: String,
) -> Result<String, String> {
    use std::time::{Duration, Instant};
    use agent::build_api_url;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(&config.user_agent)
        .build()
        .map_err(|e| format!("Client error: {}", e))?;

    let base = config.base_url.trim_end_matches('/');
    let start = Instant::now();

    match config.api_type {
        ApiType::Anthropic => {
            let url = build_api_url(base, "/v1/messages");
            let body = serde_json::json!({
                "model": model_id,
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": "Hi" }]
            });
            let request_info = serde_json::json!({
                "url": &url, "method": "POST",
                "headers": { "x-api-key": "***", "anthropic-version": "2023-06-01", "content-type": "application/json" },
                "body": &body,
            });

            let resp = client.post(&url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body).send().await;

            let resp = match resp {
                Ok(r) => r,
                Err(_) => {
                    client.post(&url)
                        .header("Authorization", format!("Bearer {}", config.api_key))
                        .header("anthropic-version", "2023-06-01")
                        .header("content-type", "application/json")
                        .json(&body).send().await
                        .map_err(|e| serde_json::to_string(&serde_json::json!({
                            "success": false, "message": format!("连接失败: {}", e),
                            "model": model_id, "latencyMs": start.elapsed().as_millis() as u64,
                            "request": request_info,
                        })).unwrap_or_default())?
                }
            };

            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let response_body: serde_json::Value = serde_json::from_str(&body_text).unwrap_or(serde_json::json!(body_text));

            if status == 200 {
                let reply = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| v["content"].as_array()?.first()?.get("text")?.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let preview = if reply.len() > 100 { format!("{}...", &reply[..100]) } else { reply.clone() };
                Ok(serde_json::to_string(&serde_json::json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                })).unwrap_or_default())
            } else {
                Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("模型测试失败 ({})", status),
                    "model": model_id, "status": status, "latencyMs": latency,
                    "request": request_info, "response": response_body,
                })).unwrap_or_default())
            }
        }
        ApiType::OpenaiChat | ApiType::OpenaiResponses => {
            let url = build_api_url(base, "/v1/chat/completions");
            let body = serde_json::json!({
                "model": model_id,
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": "Hi" }]
            });
            let auth_header = if !config.api_key.is_empty() { "Bearer ***" } else { "(none)" };
            let request_info = serde_json::json!({
                "url": &url, "method": "POST",
                "headers": { "Authorization": auth_header, "content-type": "application/json" },
                "body": &body,
            });

            let mut req = client.post(&url)
                .header("content-type", "application/json")
                .json(&body);
            if !config.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", config.api_key));
            }
            let resp = req.send().await
                .map_err(|e| serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("连接失败: {}", e),
                    "model": model_id, "latencyMs": start.elapsed().as_millis() as u64,
                    "request": request_info,
                })).unwrap_or_default())?;

            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let response_body: serde_json::Value = serde_json::from_str(&body_text).unwrap_or(serde_json::json!(body_text));

            if status == 200 {
                let reply = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| v["choices"].as_array()?.first()?.get("message")?.get("content")?.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let preview = if reply.len() > 100 { format!("{}...", &reply[..100]) } else { reply.clone() };
                Ok(serde_json::to_string(&serde_json::json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                })).unwrap_or_default())
            } else {
                Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("模型测试失败 ({})", status),
                    "model": model_id, "status": status, "latencyMs": latency,
                    "request": request_info, "response": response_body,
                })).unwrap_or_default())
            }
        }
        ApiType::Codex => {
            Ok(serde_json::to_string(&serde_json::json!({
                "success": true, "message": "Codex 模型无需单独测试",
                "model": model_id, "latencyMs": 0,
            })).unwrap_or_default())
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

// ── Command Approval ──────────────────────────────────────────────

#[tauri::command]
async fn respond_to_approval(
    request_id: String,
    response: String,
) -> Result<(), String> {
    let approval_response = match response.as_str() {
        "allow_once" => tools::ApprovalResponse::AllowOnce,
        "allow_always" => tools::ApprovalResponse::AllowAlways,
        "deny" => tools::ApprovalResponse::Deny,
        _ => return Err(format!("Invalid approval response: {}", response)),
    };
    tools::submit_approval_response(&request_id, approval_response)
        .await
        .map_err(|e| e.to_string())
}

// ── Skills Management Commands ────────────────────────────────────

#[tauri::command]
async fn get_skills(
    state: State<'_, AppState>,
) -> Result<Vec<skills::SkillSummary>, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_extra(&store.extra_skills_dirs);
    let disabled = &store.disabled_skills;
    Ok(entries
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            skills::SkillSummary {
                name: e.name,
                description: e.description,
                source: e.source,
                base_dir: e.base_dir,
                enabled,
            }
        })
        .collect())
}

#[tauri::command]
async fn get_skill_detail(
    name: String,
    state: State<'_, AppState>,
) -> Result<skills::SkillDetail, String> {
    let store = state.provider_store.lock().await;
    skills::get_skill_content(&name, &store.extra_skills_dirs, &store.disabled_skills)
        .ok_or_else(|| format!("Skill not found: {}", name))
}

#[tauri::command]
async fn get_extra_skills_dirs(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.extra_skills_dirs.clone())
}

#[tauri::command]
async fn add_extra_skills_dir(
    dir: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    // Avoid duplicates
    if !store.extra_skills_dirs.contains(&dir) {
        store.extra_skills_dirs.push(dir);
        provider::save_store(&store).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn remove_extra_skills_dir(
    dir: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    store.extra_skills_dirs.retain(|d| d != &dir);
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn toggle_skill(
    name: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    if enabled {
        store.disabled_skills.retain(|n| n != &name);
    } else if !store.disabled_skills.contains(&name) {
        store.disabled_skills.push(name);
    }
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn open_directory(path: String) -> Result<(), String> {
    // Resolve ~ to home directory
    let resolved = if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..]).to_string_lossy().to_string()
        } else {
            path
        }
    } else {
        path
    };
    open::that(&resolved).map_err(|e| format!("Failed to open directory: {}", e))
}

// ── Agent Management Commands ────────────────────────────────────

#[tauri::command]
async fn list_agents() -> Result<Vec<agent_config::AgentSummary>, String> {
    agent_loader::list_agents().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_agent_config(id: String) -> Result<agent_config::AgentConfig, String> {
    let def = agent_loader::load_agent(&id).map_err(|e| e.to_string())?;
    Ok(def.config)
}

#[tauri::command]
async fn get_agent_markdown(id: String, file: String) -> Result<Option<String>, String> {
    agent_loader::get_agent_markdown(&id, &file).map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_agent_config_cmd(id: String, config: agent_config::AgentConfig) -> Result<(), String> {
    agent_loader::save_agent_config(&id, &config).map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_agent_markdown(id: String, file: String, content: String) -> Result<(), String> {
    agent_loader::save_agent_markdown(&id, &file, &content).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_agent(id: String) -> Result<(), String> {
    agent_loader::delete_agent(&id).map_err(|e| e.to_string())
}

// ── User Config Commands ─────────────────────────────────────────

#[tauri::command]
async fn get_user_config() -> Result<user_config::UserConfig, String> {
    user_config::load_user_config().map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_user_config(config: user_config::UserConfig) -> Result<(), String> {
    user_config::save_user_config_to_disk(&config).map_err(|e| e.to_string())
}

/// Get the system's IANA timezone name
#[tauri::command]
async fn get_system_timezone() -> Result<String, String> {
    // Try reading /etc/localtime symlink (macOS/Linux)
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let path_str = link.to_string_lossy().to_string();
        // Extract timezone from path like /var/db/timezone/zoneinfo/Asia/Shanghai
        if let Some(pos) = path_str.find("zoneinfo/") {
            return Ok(path_str[pos + 9..].to_string());
        }
    }
    // Fallback: TZ env var
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return Ok(tz);
        }
    }
    Ok("UTC".to_string())
}

// ── App Entry ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize directory structure
    if let Err(e) = paths::ensure_dirs() {
        log::error!("Failed to initialize data directories: {}", e);
    }

    // Ensure default agent exists
    if let Err(e) = agent_loader::ensure_default_agent() {
        log::error!("Failed to ensure default agent: {}", e);
    }

    // Load provider store at startup
    let initial_store = provider::load_store().unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Store global AppHandle for event emission
            let _ = APP_HANDLE.set(app.handle().clone());
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
            current_agent_id: Mutex::new("default".to_string()),
        })
        .invoke_handler(tauri::generate_handler![
            // Provider management
            get_providers,
            add_provider,
            update_provider,
            delete_provider,
            test_provider,
            test_model,
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
            // Command approval
            respond_to_approval,
            // Skills
            get_skills,
            get_skill_detail,
            get_extra_skills_dirs,
            add_extra_skills_dir,
            remove_extra_skills_dir,
            toggle_skill,
            open_directory,
            // Agent management
            list_agents,
            get_agent_config,
            get_agent_markdown,
            save_agent_config_cmd,
            save_agent_markdown,
            delete_agent,
            // User config
            get_user_config,
            save_user_config,
            get_system_timezone,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
