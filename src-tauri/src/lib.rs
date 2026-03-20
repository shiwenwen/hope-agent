mod agent;
mod agent_config;
mod agent_loader;
mod failover;
mod file_extract;
mod oauth;
mod paths;
mod process_registry;
mod provider;
mod sandbox;
mod session;
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
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tauri::State;
use serde::Serialize;
use session::SessionDB;

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
    /// Session database
    session_db: Arc<SessionDB>,
    /// Cancel flag for stopping ongoing chat
    chat_cancel: Arc<AtomicBool>,
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
async fn reorder_providers(
    provider_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
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
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
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
async fn get_fallback_models(
    state: State<'_, AppState>,
) -> Result<Vec<ActiveModel>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.fallback_models.clone())
}

#[tauri::command]
async fn set_fallback_models(
    models: Vec<ActiveModel>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    store.fallback_models = models;
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
                            let active_provider = store.providers.iter().find(|p| p.id == active.provider_id);
                            if let Some(provider) = active_provider {
                                if provider.api_type == ApiType::Codex {
                                    let agent = AssistantAgent::new_openai(&token.access_token, &id, &active.model_id);
                                    *state.agent.lock().await = Some(agent);
                                } else {
                                    let agent = AssistantAgent::new_from_provider(provider, &active.model_id);
                                    *state.agent.lock().await = Some(agent);
                                }
                            }
                        }
                    }
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

/// Build an AssistantAgent for a given ActiveModel.
/// Handles Codex (OAuth) vs regular API key providers.
async fn build_agent_for_model(
    model: &ActiveModel,
    state: &State<'_, AppState>,
) -> Option<AssistantAgent> {
    let store = state.provider_store.lock().await;
    let prov = provider::find_provider(&store.providers, &model.provider_id)?;

    if prov.api_type == ApiType::Codex {
        let token_info = state.codex_token.lock().await.clone();
        let (access_token, account_id) = token_info?;
        Some(AssistantAgent::new_openai(&access_token, &account_id, &model.model_id))
    } else {
        Some(AssistantAgent::new_from_provider(prov, &model.model_id))
    }
}

/// Find the provider name + model name for display in fallback notifications.
async fn model_display_name(
    model: &ActiveModel,
    state: &State<'_, AppState>,
) -> String {
    let store = state.provider_store.lock().await;
    if let Some(prov) = store.providers.iter().find(|p| p.id == model.provider_id) {
        let model_name = prov.models.iter()
            .find(|m| m.id == model.model_id)
            .map(|m| m.name.as_str())
            .unwrap_or(&model.model_id);
        format!("{} / {}", prov.name, model_name)
    } else {
        format!("{}::{}", model.provider_id, model.model_id)
    }
}

// ── Session Management Commands ───────────────────────────────────

#[tauri::command]
async fn create_session_cmd(
    agent_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<session::SessionMeta, String> {
    let agent_id = agent_id.unwrap_or_else(|| "default".to_string());
    state.session_db.create_session(&agent_id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_sessions_cmd(
    agent_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionMeta>, String> {
    state.session_db.list_sessions(agent_id.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn load_session_messages_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionMessage>, String> {
    state.session_db.load_session_messages(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<session::SessionMeta>, String> {
    state.session_db.get_session(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.delete_session(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn rename_session_cmd(
    session_id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.update_session_title(&session_id, &title).map_err(|e| e.to_string())
}

/// Save an attachment file to disk. Uses a temp directory when session_id is empty.
/// Returns the absolute path to the saved file.
#[tauri::command]
async fn save_attachment(
    session_id: Option<String>,
    file_name: String,
    _mime_type: String,
    data: Vec<u8>,
) -> Result<String, String> {
    // Use temp directory if no session ID yet (new chat)
    let att_dir = match &session_id {
        Some(sid) if !sid.is_empty() => {
            crate::paths::attachments_dir(sid).map_err(|e| e.to_string())?
        }
        _ => {
            let root = crate::paths::root_dir().map_err(|e| e.to_string())?;
            root.join("attachments").join("_temp")
        }
    };
    std::fs::create_dir_all(&att_dir)
        .map_err(|e| format!("Failed to create attachments dir: {}", e))?;

    // Generate unique filename with timestamp to avoid collisions
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name = file_name.replace(['/', '\\', ':'], "_");
    let filename = format!("{}_{}", ts, safe_name);
    let file_path = att_dir.join(&filename);

    std::fs::write(&file_path, &data)
        .map_err(|e| format!("Failed to write attachment {}: {}", file_name, e))?;

    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
async fn chat(
    message: String,
    mut attachments: Vec<Attachment>,
    session_id: Option<String>,
    on_event: tauri::ipc::Channel<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let effort = state.reasoning_effort.lock().await.clone();
    let effort_ref_str = effort.clone();
    let db = state.session_db.clone();
    let cancel = state.chat_cancel.clone();
    cancel.store(false, Ordering::SeqCst); // Reset cancel flag

    // Resolve or create session
    let current_agent_id = state.current_agent_id.lock().await.clone();
    let sid = match session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            // Auto-create a new session
            let meta = db.create_session(&current_agent_id).map_err(|e| e.to_string())?;
            // Emit session_created event so frontend knows
            let event = serde_json::json!({
                "type": "session_created",
                "session_id": &meta.id,
            });
            if let Ok(json_str) = serde_json::to_string(&event) {
                let _ = on_event.send(json_str);
            }
            meta.id
        }
    };

    // Build attachments metadata from file paths (files already saved by save_attachment)
    let attachments_meta = if !attachments.is_empty() {
        // Ensure session attachments directory exists and move temp files if needed
        let att_dir = crate::paths::attachments_dir(&sid).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&att_dir).map_err(|e| format!("Failed to create attachments dir: {}", e))?;

        let temp_dir = crate::paths::root_dir()
            .map(|r| r.join("attachments").join("_temp"))
            .unwrap_or_default();

        let mut meta_list = Vec::new();
        for att in attachments.iter_mut() {
            // Images: have base64 data directly, save to disk for persistence
            if let Some(ref b64_data) = att.data {
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(b64_data)
                    .unwrap_or_default();
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let safe_name = att.name.replace(['/', '\\', ':'], "_");
                let filename = format!("{}_{}", ts, safe_name);
                let file_path = att_dir.join(&filename);
                if let Err(e) = std::fs::write(&file_path, &decoded) {
                    log::warn!("Failed to save image attachment {}: {}", att.name, e);
                    continue;
                }
                meta_list.push(serde_json::json!({
                    "name": att.name,
                    "mime_type": att.mime_type,
                    "size": decoded.len(),
                    "path": file_path.to_string_lossy(),
                }));
                continue;
            }

            // Non-image files: have file_path, move from temp dir if needed
            if let Some(ref fp) = att.file_path {
                let src_path = std::path::Path::new(fp);

                let final_path = if src_path.starts_with(&temp_dir) {
                    if let Some(fname) = src_path.file_name() {
                        let dest = att_dir.join(fname);
                        if let Err(e) = std::fs::rename(src_path, &dest) {
                            if let Err(e2) = std::fs::copy(src_path, &dest) {
                                log::warn!("Failed to move attachment {}: rename={}, copy={}", att.name, e, e2);
                                continue;
                            }
                            let _ = std::fs::remove_file(src_path);
                        }
                        dest
                    } else {
                        src_path.to_path_buf()
                    }
                } else {
                    src_path.to_path_buf()
                };

                // Update the attachment's file_path to the final location
                att.file_path = Some(final_path.to_string_lossy().to_string());

                let size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
                meta_list.push(serde_json::json!({
                    "name": att.name,
                    "mime_type": att.mime_type,
                    "size": size,
                    "path": final_path.to_string_lossy(),
                }));
            }
        }
        Some(serde_json::to_string(&meta_list).unwrap_or_default())
    } else {
        None
    };

    // Save user message to DB
    let mut user_msg = session::NewMessage::user(&message);
    user_msg.attachments_meta = attachments_meta;
    let _ = db.append_message(&sid, &user_msg);

    // Auto-generate title from first user message if session has no title
    if let Ok(Some(meta)) = db.get_session(&sid) {
        if meta.title.is_none() && meta.message_count <= 1 {
            let title = session::auto_title(&message);
            let _ = db.update_session_title(&sid, &title);
        }
    }

    // Resolve model chain from current agent config
    let agent_model_config = agent_loader::load_agent(&current_agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();

    let (primary, fallbacks) = {
        let store = state.provider_store.lock().await;
        provider::resolve_model_chain(&agent_model_config, &store)
    };

    // Build ordered model chain: [primary, ...fallbacks]
    let mut model_chain: Vec<ActiveModel> = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        // Avoid duplicates
        if !model_chain.iter().any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        // No model chain resolved — fall back to existing agent instance
        let agent_lock = state.agent.lock().await;
        return match agent_lock.as_ref() {
            Some(agent) => {
                // Restore conversation history from DB for this session
                restore_agent_context(&db, &sid, agent);

                let effort_ref = if effort_ref_str == "none" { None } else { Some(effort_ref_str.as_str()) };
                let db_for_cb = db.clone();
                let sid_for_cb = sid.clone();
                let cancel_clone = cancel.clone();
                let chat_start = std::time::Instant::now();
                let on_event_clone = on_event.clone();
                // Shared state to capture token usage from on_delta callback
                let captured_tokens: Arc<std::sync::Mutex<(Option<i64>, Option<i64>)>> = Arc::new(std::sync::Mutex::new((None, None)));
                let captured_tokens_clone = captured_tokens.clone();
                let result = match agent.chat(&message, &attachments, effort_ref, cancel_clone, move |delta| {
                    // Intercept usage events to capture token counts
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                        if event.get("type").and_then(|t| t.as_str()) == Some("usage") {
                            if let Ok(mut tokens) = captured_tokens_clone.lock() {
                                if let Some(it) = event.get("input_tokens").and_then(|v| v.as_i64()) {
                                    tokens.0 = Some(it);
                                }
                                if let Some(ot) = event.get("output_tokens").and_then(|v| v.as_i64()) {
                                    tokens.1 = Some(ot);
                                }
                            }
                        }
                    }
                    persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                    let _ = on_event_clone.send(delta.to_string());
                }).await {
                    Ok(r) => r,
                    Err(e) => {
                        let err = e.to_string();
                        let _ = db.append_message(&sid, &session::NewMessage::event(&err));
                        return Err(err);
                    }
                };
                let duration_ms = chat_start.elapsed().as_millis() as u64;
                // Emit usage event with duration
                emit_usage_event(&on_event, duration_ms);
                // Save assistant reply with duration and tokens
                let mut assistant_msg = session::NewMessage::assistant(&result);
                assistant_msg.tool_duration_ms = Some(duration_ms as i64);
                if let Ok(tokens) = captured_tokens.lock() {
                    assistant_msg.tokens_in = tokens.0;
                    assistant_msg.tokens_out = tokens.1;
                }
                let _ = db.append_message(&sid, &assistant_msg);
                // Persist conversation context for future restoration
                save_agent_context(&db, &sid, agent);
                Ok(result)
            }
            None => {
                let err = "Agent not initialized. Please sign in first.".to_string();
                let _ = db.append_message(&sid, &session::NewMessage::event(&err));
                Err(err)
            }
        };
    }

    let mut last_error: Option<String> = None;
    let total_models = model_chain.len();
    // Track first model for "from_model" in fallback events
    let primary_display = {
        let first = &model_chain[0];
        model_display_name(first, &state).await
    };

    for (idx, model_ref) in model_chain.iter().enumerate() {
        let agent = match build_agent_for_model(model_ref, &state).await {
            Some(a) => a,
            None => {
                last_error = Some(format!("Cannot build agent for {}::{}", model_ref.provider_id, model_ref.model_id));
                continue;
            }
        };

        // Restore conversation history from DB for this session
        restore_agent_context(&db, &sid, &agent);

        // Determine max retries for this model
        const MAX_RETRIES: u32 = 2;
        const RETRY_BASE_MS: u64 = 1000;
        const RETRY_MAX_MS: u64 = 10000;

        let mut retry_count: u32 = 0;

        loop {
            // If this is a fallback (not the first model) and first attempt, notify frontend
            if idx > 0 && retry_count == 0 {
                let display = model_display_name(model_ref, &state).await;
                let reason_str = last_error.as_deref()
                    .map(|e| failover::classify_error(e))
                    .unwrap_or(failover::FailoverReason::Unknown);
                let event = serde_json::json!({
                    "type": "model_fallback",
                    "model": display,
                    "from_model": primary_display,
                    "provider_id": model_ref.provider_id,
                    "model_id": model_ref.model_id,
                    "reason": reason_str,
                    "attempt": idx + 1,
                    "total": total_models,
                    "error": last_error.as_deref().unwrap_or(""),
                });
                if let Ok(json_str) = serde_json::to_string(&event) {
                    let _ = on_event.send(json_str.clone());
                    // Persist fallback event to session DB
                    let _ = db.append_message(&sid, &session::NewMessage::event(&json_str));
                }
            }

            // Update session with current model info
            if retry_count == 0 {
                let store = state.provider_store.lock().await;
                let provider_name = store.providers.iter()
                    .find(|p| p.id == model_ref.provider_id)
                    .map(|p| p.name.as_str());
                let _ = db.update_session_model(&sid, provider_name, Some(&model_ref.model_id));
            }

            let effort_ref = if effort_ref_str == "none" { None } else { Some(effort_ref_str.as_str()) };
            let on_event_clone = on_event.clone();
            let db_for_cb = db.clone();
            let sid_for_cb = sid.clone();
            let cancel_clone = cancel.clone();

            // Shared state to capture token usage from on_delta callback
            let captured_tokens: Arc<std::sync::Mutex<(Option<i64>, Option<i64>)>> = Arc::new(std::sync::Mutex::new((None, None)));
            let captured_tokens_clone = captured_tokens.clone();

            let chat_start = std::time::Instant::now();
            match agent.chat(&message, &attachments, effort_ref, cancel_clone, move |delta| {
                // Intercept usage events to capture token counts
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                    if event.get("type").and_then(|t| t.as_str()) == Some("usage") {
                        if let Ok(mut tokens) = captured_tokens_clone.lock() {
                            if let Some(it) = event.get("input_tokens").and_then(|v| v.as_i64()) {
                                tokens.0 = Some(it);
                            }
                            if let Some(ot) = event.get("output_tokens").and_then(|v| v.as_i64()) {
                                tokens.1 = Some(ot);
                            }
                        }
                    }
                }
                persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                let _ = on_event_clone.send(delta.to_string());
            }).await {
                Ok(result) => {
                    let duration_ms = chat_start.elapsed().as_millis() as u64;
                    // Emit usage event with duration
                    emit_usage_event(&on_event, duration_ms);
                    // Save assistant reply to DB with duration and tokens
                    let mut assistant_msg = session::NewMessage::assistant(&result);
                    assistant_msg.tool_duration_ms = Some(duration_ms as i64);
                    if let Ok(tokens) = captured_tokens.lock() {
                        assistant_msg.tokens_in = tokens.0;
                        assistant_msg.tokens_out = tokens.1;
                    }
                    let _ = db.append_message(&sid, &assistant_msg);
                    // Persist conversation context for future restoration
                    save_agent_context(&db, &sid, &agent);
                    // Update the active agent instance for conversation continuity
                    *state.agent.lock().await = Some(agent);
                    return Ok(result);
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let reason = failover::classify_error(&error_msg);

                    log::warn!(
                        "Model {}::{} failed (attempt {}/{}, retry {}, reason {:?}): {}",
                        model_ref.provider_id, model_ref.model_id,
                        idx + 1, total_models, retry_count, reason, error_msg
                    );

                    // Terminal errors — surface immediately, no fallback
                    if reason.is_terminal() {
                        let _ = db.append_message(&sid, &session::NewMessage::event(&error_msg));
                        return Err(error_msg);
                    }

                    // Retryable errors — retry on same model with backoff
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay = failover::retry_delay_ms(retry_count - 1, RETRY_BASE_MS, RETRY_MAX_MS);
                        log::info!(
                            "Retrying {}::{} in {}ms (retry {}/{}, reason {:?})",
                            model_ref.provider_id, model_ref.model_id,
                            delay, retry_count, MAX_RETRIES, reason
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue; // Retry same model
                    }

                    // Non-retryable or retries exhausted — move to next model
                    last_error = Some(error_msg);
                    break; // Break inner retry loop, continue outer model loop
                }
            }
        }
    }

    let final_error = last_error.unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    let _ = db.append_message(&sid, &session::NewMessage::event(&final_error));
    Err(final_error)
}

#[tauri::command]
async fn stop_chat(state: State<'_, AppState>) -> Result<(), String> {
    state.chat_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// Restore conversation history from DB into the agent (if the session has saved context).
fn restore_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &crate::agent::AssistantAgent) {
    if let Ok(Some(json_str)) = db.load_context(session_id) {
        if let Ok(history) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            if !history.is_empty() {
                agent.set_conversation_history(history);
            }
        }
    }
}

/// Save the agent's conversation history to DB for future restoration.
fn save_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &crate::agent::AssistantAgent) {
    let history = agent.get_conversation_history();
    if let Ok(json_str) = serde_json::to_string(&history) {
        let _ = db.save_context(session_id, &json_str);
    }
}

/// Emit a usage event (with duration) to the frontend via the Tauri Channel.
fn emit_usage_event(on_event: &tauri::ipc::Channel<String>, duration_ms: u64) {
    let event = serde_json::json!({
        "type": "usage",
        "duration_ms": duration_ms,
    });
    if let Ok(json_str) = serde_json::to_string(&event) {
        let _ = on_event.send(json_str);
    }
}

/// Parse tool_call and tool_result events from the streaming callback and persist to DB.
fn persist_tool_event(db: &Arc<SessionDB>, session_id: &str, delta: &str) {
    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
        match event.get("type").and_then(|t| t.as_str()) {
            Some("tool_result") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let result = event.get("result").and_then(|v| v.as_str()).unwrap_or("");
                // We need the tool name, but tool_result events may not have it.
                // Use call_id as fallback for now.
                let tool_msg = session::NewMessage::tool(
                    call_id,
                    "", // name filled from tool_call event
                    "",
                    result,
                    None,
                    false,
                );
                let _ = db.append_message(session_id, &tool_msg);
            }
            Some("tool_call") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let name = event.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = event.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                let tool_msg = session::NewMessage::tool(
                    call_id,
                    name,
                    arguments,
                    "", // result will be filled in tool_result event
                    None,
                    false,
                );
                let _ = db.append_message(session_id, &tool_msg);
            }
            _ => {
                // text_delta events are not persisted as separate messages.
                // text_delta is accumulated into the final assistant message.
            }
        }
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

// ── Tools Info Commands ───────────────────────────────────────────

#[tauri::command]
async fn list_builtin_tools() -> Result<Vec<serde_json::Value>, String> {
    Ok(tools::get_available_tools()
        .into_iter()
        .map(|t| serde_json::json!({ "name": t.name, "description": t.description }))
        .collect())
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

#[tauri::command]
async fn get_agent_template(name: String, locale: String) -> Result<String, String> {
    agent_loader::get_template(&name, &locale)
        .ok_or_else(|| format!("Template not found: {}", name))
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

/// Save a cropped avatar image (base64-encoded) to ~/.opencomputer/avatars/
/// Returns the absolute path to the saved file.
#[tauri::command]
async fn save_avatar(image_data: String, file_name: String) -> Result<String, String> {
    use base64::Engine;

    let dir = paths::avatars_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image_data)
        .map_err(|e| format!("Base64 decode error: {}", e))?;

    let path = dir.join(&file_name);
    std::fs::write(&path, &bytes).map_err(|e| format!("Failed to write avatar: {}", e))?;

    Ok(path.to_string_lossy().to_string())
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

// ── Window Theme Command ──────────────────────────────────────────

#[tauri::command]
async fn set_window_theme(
    is_dark: bool,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        if let Some(window) = app_handle.get_webview_window("main") {
            let _ = window.with_webview(move |webview| unsafe {
                let ns_window: &objc2_app_kit::NSWindow =
                    &*webview.ns_window().cast();
                let (r, g, b) = if is_dark {
                    (15.0 / 255.0, 15.0 / 255.0, 15.0 / 255.0)
                } else {
                    (1.0, 1.0, 1.0)
                };
                let bg_color =
                    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
                ns_window.setBackgroundColor(Some(&bg_color));
            });
        }
    }
    Ok(())
}

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

            // Fix macOS theme-aware background to prevent flash on window resize
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| unsafe {
                        let ns_window: &objc2_app_kit::NSWindow =
                            &*webview.ns_window().cast();
                        // Detect system dark mode via appearance name
                        let is_dark = {
                            use objc2_app_kit::NSAppearanceCustomization;
                            let appearance = ns_window.effectiveAppearance();
                            let name = appearance.name();
                            name.to_string().contains("Dark")
                        };
                        let (r, g, b) = if is_dark {
                            (15.0 / 255.0, 15.0 / 255.0, 15.0 / 255.0)
                        } else {
                            (1.0, 1.0, 1.0)
                        };
                        let bg_color =
                            objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
                        ns_window.setBackgroundColor(Some(&bg_color));
                    });
                }
            }

            Ok(())
        })
        .manage({
            // Initialize the SessionDB
            let db_path = session::db_path().expect("Failed to resolve database path");
            let session_db = Arc::new(
                SessionDB::open(&db_path).expect("Failed to open session database")
            );

            AppState {
                agent: Mutex::new(None),
                auth_result: Arc::new(Mutex::new(None)),
                provider_store: Mutex::new(initial_store),
                reasoning_effort: Mutex::new("medium".to_string()),
                codex_token: Mutex::new(None),
                current_agent_id: Mutex::new("default".to_string()),
                session_db,
                chat_cancel: Arc::new(AtomicBool::new(false)),
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Provider management
            get_providers,
            add_provider,
            update_provider,
            reorder_providers,
            delete_provider,
            test_provider,
            test_model,
            get_available_models,
            get_active_model,
            set_active_model,
            get_fallback_models,
            set_fallback_models,
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
            save_attachment,
            chat,
            stop_chat,
            // Command approval
            respond_to_approval,
            // Tools info
            list_builtin_tools,
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
            get_agent_template,
            // User config
            get_user_config,
            save_user_config,
            save_avatar,
            get_system_timezone,
            // Session management
            create_session_cmd,
            list_sessions_cmd,
            load_session_messages_cmd,
            get_session_cmd,
            delete_session_cmd,
            rename_session_cmd,
            // Window theme
            set_window_theme,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
