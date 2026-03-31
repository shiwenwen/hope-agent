use crate::agent::{build_api_url, AssistantAgent};
use crate::memory;
use crate::provider::{self, ActiveModel, ApiType, AvailableModel, ProviderConfig};
use crate::truncate_utf8;
use crate::AppState;
use tauri::State;

// ── Provider Management Commands ──────────────────────────────────

#[tauri::command]
pub async fn get_providers(state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.providers.clone())
}

#[tauri::command]
pub async fn add_provider(
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
pub async fn update_provider(
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
        existing.thinking_style = config.thinking_style;
        provider::save_store(&store).map_err(|e| e.to_string())?;
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
pub async fn delete_provider(
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
pub async fn test_provider(config: ProviderConfig) -> Result<String, String> {
    use std::time::{Duration, Instant};

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(&config.user_agent),
    )
    .build()
    .map_err(|e| format!("Client error: {}", e))?;

    let base = config.base_url.trim_end_matches('/');
    let has_version_suffix =
        base.ends_with("/v1") || base.ends_with("/v2") || base.ends_with("/v3");
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
            let resp = client
                .post(&url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    build_result!(false, format!("连接失败: {}", e), &url, 0, "x-api-key")
                })?;
            let status = resp.status().as_u16();
            steps.push(serde_json::json!({"endpoint": &url, "method": "POST", "auth": "x-api-key", "status": status, "latencyMs": t.elapsed().as_millis() as u64}));

            if resp.status().is_success() || status == 400 || status == 404 {
                return Ok(build_result!(
                    true,
                    if status == 200 {
                        "连接成功"
                    } else {
                        "认证成功（模型名需调整）"
                    },
                    &url,
                    status,
                    "x-api-key"
                ));
            }

            // Fallback: Bearer auth
            if status == 401 || status == 403 {
                let t2 = Instant::now();
                let resp2 = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", config.api_key))
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| {
                        build_result!(false, format!("连接失败: {}", e), &url, 0, "Bearer")
                    })?;
                let s2 = resp2.status().as_u16();
                steps.push(serde_json::json!({"endpoint": &url, "method": "POST", "auth": "Bearer", "status": s2, "latencyMs": t2.elapsed().as_millis() as u64}));

                if resp2.status().is_success() || s2 == 400 || s2 == 404 {
                    return Ok(build_result!(
                        true,
                        "连接成功（Bearer 认证）",
                        &url,
                        s2,
                        "Bearer"
                    ));
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
            let models_url = if has_version_suffix {
                format!("{}/models", base)
            } else {
                format!("{}/v1/models", base)
            };
            let t = Instant::now();
            let mut req = client.get(&models_url);
            if !config.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", config.api_key));
            }
            let resp = req.send().await.map_err(|e| {
                build_result!(false, format!("连接失败: {}", e), &models_url, 0, "Bearer")
            })?;
            let status = resp.status().as_u16();
            steps.push(serde_json::json!({"endpoint": &models_url, "method": "GET", "status": status, "latencyMs": t.elapsed().as_millis() as u64}));

            if resp.status().is_success() {
                return Ok(build_result!(
                    true,
                    "连接成功",
                    &models_url,
                    status,
                    "Bearer"
                ));
            }
            if status == 401 || status == 403 {
                let detail = resp.text().await.unwrap_or_default();
                return Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("认证失败 ({})", status), "detail": detail,
                    "url": &models_url, "status": status, "latencyMs": total_start.elapsed().as_millis() as u64, "steps": steps,
                })).unwrap_or_default());
            }

            // Fallback: chat/completions
            let chat_url = if has_version_suffix {
                format!("{}/chat/completions", base)
            } else {
                format!("{}/v1/chat/completions", base)
            };
            let t2 = Instant::now();
            let mut chat_req = client.post(&chat_url)
                .header("content-type", "application/json")
                .json(&serde_json::json!({"model": "test", "max_tokens": 1, "messages": [{"role": "user", "content": "Hi"}]}));
            if !config.api_key.is_empty() {
                chat_req = chat_req.header("Authorization", format!("Bearer {}", config.api_key));
            }

            match chat_req.send().await {
                Ok(chat_resp) => {
                    let cs = chat_resp.status().as_u16();
                    steps.push(serde_json::json!({"endpoint": &chat_url, "method": "POST", "status": cs, "latencyMs": t2.elapsed().as_millis() as u64}));
                    if chat_resp.status().is_success() || cs == 400 || cs == 404 {
                        Ok(build_result!(
                            true,
                            if cs == 200 {
                                "连接成功"
                            } else {
                                "认证成功（模型名需调整）"
                            },
                            &chat_url,
                            cs,
                            "Bearer"
                        ))
                    } else if cs == 401 || cs == 403 {
                        let detail = chat_resp.text().await.unwrap_or_default();
                        Err(serde_json::to_string(&serde_json::json!({
                            "success": false, "message": format!("认证失败 ({})", cs), "detail": detail,
                            "url": &chat_url, "status": cs, "latencyMs": total_start.elapsed().as_millis() as u64, "steps": steps,
                        })).unwrap_or_default())
                    } else {
                        Ok(build_result!(
                            true,
                            "连接成功（不支持模型列表查询）",
                            &chat_url,
                            cs,
                            "Bearer"
                        ))
                    }
                }
                Err(e) => {
                    steps.push(serde_json::json!({"endpoint": &chat_url, "method": "POST", "error": format!("{}", e), "latencyMs": t2.elapsed().as_millis() as u64}));
                    Err(build_result!(
                        false,
                        format!("连接失败: {}", e),
                        &chat_url,
                        0,
                        ""
                    ))
                }
            }
        }
        ApiType::Codex => Ok(build_result!(
            true,
            "Codex 使用 OAuth 认证，无需测试",
            "",
            0,
            "OAuth"
        )),
    }
}

#[tauri::command]
pub async fn test_model(config: ProviderConfig, model_id: String) -> Result<String, String> {
    use std::time::{Duration, Instant};

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(&config.user_agent),
    )
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

            let resp = client
                .post(&url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(_) => client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", config.api_key))
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| {
                        serde_json::to_string(&serde_json::json!({
                            "success": false, "message": format!("连接失败: {}", e),
                            "model": model_id, "latencyMs": start.elapsed().as_millis() as u64,
                            "request": request_info,
                        }))
                        .unwrap_or_default()
                    })?,
            };

            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let response_body: serde_json::Value =
                serde_json::from_str(&body_text).unwrap_or(serde_json::json!(body_text));

            if status == 200 {
                let reply = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| {
                        v["content"]
                            .as_array()?
                            .first()?
                            .get("text")?
                            .as_str()
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default();
                let preview = if reply.len() > 100 {
                    format!("{}...", truncate_utf8(&reply, 100))
                } else {
                    reply.clone()
                };
                Ok(serde_json::to_string(&serde_json::json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            } else {
                Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("模型测试失败 ({})", status),
                    "model": model_id, "status": status, "latencyMs": latency,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            }
        }
        ApiType::OpenaiChat | ApiType::OpenaiResponses => {
            let url = build_api_url(base, "/v1/chat/completions");
            let body = serde_json::json!({
                "model": model_id,
                "max_tokens": 32,
                "messages": [{ "role": "user", "content": "Hi" }]
            });
            let auth_header = if !config.api_key.is_empty() {
                "Bearer ***"
            } else {
                "(none)"
            };
            let request_info = serde_json::json!({
                "url": &url, "method": "POST",
                "headers": { "Authorization": auth_header, "content-type": "application/json" },
                "body": &body,
            });

            let mut req = client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body);
            if !config.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", config.api_key));
            }
            let resp = req.send().await.map_err(|e| {
                serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("连接失败: {}", e),
                    "model": model_id, "latencyMs": start.elapsed().as_millis() as u64,
                    "request": request_info,
                }))
                .unwrap_or_default()
            })?;

            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let response_body: serde_json::Value =
                serde_json::from_str(&body_text).unwrap_or(serde_json::json!(body_text));

            if status == 200 {
                let reply = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| {
                        v["choices"]
                            .as_array()?
                            .first()?
                            .get("message")?
                            .get("content")?
                            .as_str()
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default();
                let preview = if reply.len() > 100 {
                    format!("{}...", truncate_utf8(&reply, 100))
                } else {
                    reply.clone()
                };
                Ok(serde_json::to_string(&serde_json::json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            } else {
                Err(serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("模型测试失败 ({})", status),
                    "model": model_id, "status": status, "latencyMs": latency,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            }
        }
        ApiType::Codex => Ok(serde_json::to_string(&serde_json::json!({
            "success": true, "message": "Codex 模型无需单独测试",
            "model": model_id, "latencyMs": 0,
        }))
        .unwrap_or_default()),
    }
}

#[tauri::command]
pub async fn test_embedding(config: memory::EmbeddingConfig) -> Result<String, String> {
    use std::time::{Duration, Instant};

    let start = Instant::now();

    match config.provider_type {
        memory::EmbeddingProviderType::Local => {
            let model_id = config
                .local_model_id
                .clone()
                .unwrap_or_else(|| "bge-small-en-v1.5".to_string());
            let model_id_clone = model_id.clone();
            // Local model init + embed is blocking, run in spawn_blocking
            let result = tokio::task::spawn_blocking(move || -> Result<(usize, u64), String> {
                use memory::EmbeddingProvider;
                let t = Instant::now();
                let provider = memory::LocalEmbeddingProvider::new(&model_id_clone)
                    .map_err(|e| format!("{}", e))?;
                let vec = provider.embed("test").map_err(|e| format!("{}", e))?;
                Ok((vec.len(), t.elapsed().as_millis() as u64))
            })
            .await
            .map_err(|e| {
                serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("任务执行失败: {}", e),
                    "latencyMs": start.elapsed().as_millis() as u64,
                }))
                .unwrap_or_default()
            })?;

            match result {
                Ok((dims, latency)) => Ok(serde_json::to_string(&serde_json::json!({
                    "success": true,
                    "message": format!("本地模型测试成功（{}维）", dims),
                    "url": model_id,
                    "latencyMs": latency,
                }))
                .unwrap_or_default()),
                Err(e) => Err(serde_json::to_string(&serde_json::json!({
                    "success": false,
                    "message": format!("本地模型测试失败: {}", e),
                    "latencyMs": start.elapsed().as_millis() as u64,
                }))
                .unwrap_or_default()),
            }
        }
        memory::EmbeddingProviderType::Google => {
            let base_url = config
                .api_base_url
                .as_deref()
                .unwrap_or("https://generativelanguage.googleapis.com")
                .trim_end_matches('/')
                .to_string();
            let api_key = config.api_key.as_deref().unwrap_or("").to_string();
            let model = config
                .api_model
                .as_deref()
                .unwrap_or("gemini-embedding-001")
                .to_string();

            let url = format!(
                "{}/v1beta/models/{}:embedContent?key={}",
                base_url, model, api_key
            );

            let mut body = serde_json::json!({
                "content": { "parts": [{"text": "test"}] }
            });
            if let Some(dims) = config.api_dimensions {
                if dims > 0 {
                    body["outputDimensionality"] = serde_json::json!(dims);
                }
            }

            let client = crate::provider::apply_proxy(
                reqwest::Client::builder().timeout(Duration::from_secs(15)),
            )
            .build()
            .map_err(|e| {
                serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("Client error: {}", e),
                }))
                .unwrap_or_default()
            })?;

            let display_url = format!("{}/v1beta/models/{}:embedContent", base_url, model);

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let resp_text = resp.text().await.unwrap_or_default();
                    let latency = start.elapsed().as_millis() as u64;

                    if status == 200 {
                        let dims = serde_json::from_str::<serde_json::Value>(&resp_text)
                            .ok()
                            .and_then(|v| v["embedding"]["values"].as_array().map(|a| a.len()))
                            .unwrap_or(0);
                        Ok(serde_json::to_string(&serde_json::json!({
                            "success": true,
                            "message": format!("Embedding 连接成功（{}维）", dims),
                            "url": display_url,
                            "status": status,
                            "latencyMs": latency,
                            "auth": "API Key (query)",
                        }))
                        .unwrap_or_default())
                    } else {
                        Err(serde_json::to_string(&serde_json::json!({
                            "success": false,
                            "message": format!("API 错误 ({})", status),
                            "url": display_url,
                            "status": status,
                            "latencyMs": latency,
                            "detail": crate::truncate_utf8(&resp_text, 500),
                        }))
                        .unwrap_or_default())
                    }
                }
                Err(e) => Err(serde_json::to_string(&serde_json::json!({
                    "success": false,
                    "message": format!("连接失败: {}", e),
                    "url": display_url,
                    "latencyMs": start.elapsed().as_millis() as u64,
                }))
                .unwrap_or_default()),
            }
        }
        _ => {
            // OpenAI-compatible
            let base_url = config
                .api_base_url
                .as_deref()
                .unwrap_or("https://api.openai.com")
                .trim_end_matches('/')
                .to_string();
            let api_key = config.api_key.as_deref().unwrap_or("").to_string();
            let model = config
                .api_model
                .as_deref()
                .unwrap_or("text-embedding-3-small")
                .to_string();

            let url = format!("{}/v1/embeddings", base_url);

            let mut body = serde_json::json!({
                "model": model,
                "input": ["test"],
            });
            if let Some(dims) = config.api_dimensions {
                if dims > 0 {
                    body["dimensions"] = serde_json::json!(dims);
                }
            }

            let client = crate::provider::apply_proxy(
                reqwest::Client::builder().timeout(Duration::from_secs(15)),
            )
            .build()
            .map_err(|e| {
                serde_json::to_string(&serde_json::json!({
                    "success": false, "message": format!("Client error: {}", e),
                }))
                .unwrap_or_default()
            })?;

            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let resp_text = resp.text().await.unwrap_or_default();
                    let latency = start.elapsed().as_millis() as u64;

                    if status == 200 {
                        let dims = serde_json::from_str::<serde_json::Value>(&resp_text)
                            .ok()
                            .and_then(|v| {
                                v["data"].as_array()?.first()?["embedding"]
                                    .as_array()
                                    .map(|a| a.len())
                            })
                            .unwrap_or(0);
                        Ok(serde_json::to_string(&serde_json::json!({
                            "success": true,
                            "message": format!("Embedding 连接成功（{}维）", dims),
                            "url": url,
                            "status": status,
                            "latencyMs": latency,
                            "auth": "Bearer",
                        }))
                        .unwrap_or_default())
                    } else if status == 401 || status == 403 {
                        let detail = crate::truncate_utf8(&resp_text, 500);
                        Err(serde_json::to_string(&serde_json::json!({
                            "success": false,
                            "message": format!("认证失败 ({})", status),
                            "url": url,
                            "status": status,
                            "latencyMs": latency,
                            "auth": "Bearer",
                            "detail": detail,
                        }))
                        .unwrap_or_default())
                    } else {
                        let detail = crate::truncate_utf8(&resp_text, 500);
                        Err(serde_json::to_string(&serde_json::json!({
                            "success": false,
                            "message": format!("API 错误 ({})", status),
                            "url": url,
                            "status": status,
                            "latencyMs": latency,
                            "detail": detail,
                        }))
                        .unwrap_or_default())
                    }
                }
                Err(e) => Err(serde_json::to_string(&serde_json::json!({
                    "success": false,
                    "message": format!("连接失败: {}", e),
                    "url": url,
                    "latencyMs": start.elapsed().as_millis() as u64,
                }))
                .unwrap_or_default()),
            }
        }
    }
}

// ── Image Generation Provider Test ──────────────────────────────

#[tauri::command]
pub async fn test_image_generate(
    provider_id: String,
    api_key: String,
    base_url: Option<String>,
) -> Result<String, String> {
    use std::time::{Duration, Instant};

    let start = Instant::now();
    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15)),
    )
    .build()
    .map_err(|e| {
        serde_json::to_string(&serde_json::json!({
            "success": false, "message": format!("Client error: {}", e),
        }))
        .unwrap_or_default()
    })?;

    // Normalize provider_id (backward compat: "OpenAI" → "openai")
    let pid = provider_id.to_lowercase();
    let display_name = crate::tools::image_generate::resolve_provider(&pid)
        .map(|p| p.display_name().to_string())
        .unwrap_or_else(|| provider_id.clone());

    let (url, auth_header, auth_value) = match pid.as_str() {
        "openai" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("https://api.openai.com")
                .trim_end_matches('/');
            (
                format!("{}/v1/models", base),
                "Authorization",
                format!("Bearer {}", api_key),
            )
        }
        "google" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("https://generativelanguage.googleapis.com")
                .trim_end_matches('/');
            (
                format!("{}/v1beta/models?key={}", base, api_key),
                "",
                String::new(),
            )
        }
        "fal" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("https://fal.run")
                .trim_end_matches('/');
            // Fal doesn't have a lightweight list endpoint; check queue status
            (
                format!("{}/fal-ai/flux/dev", base),
                "Authorization",
                format!("Key {}", api_key),
            )
        }
        "minimax" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    if let Ok(parsed) = url::Url::parse(s) {
                        format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(s))
                    } else {
                        s.trim_end_matches('/').to_string()
                    }
                })
                .unwrap_or_else(|| "https://api.minimax.io".to_string());
            // MiniMax: GET on the endpoint will return 405 = API alive
            (
                format!("{}/v1/image_generation", base),
                "Authorization",
                format!("Bearer {}", api_key),
            )
        }
        "siliconflow" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("https://api.siliconflow.cn")
                .trim_end_matches('/');
            (
                format!("{}/v1/models", base),
                "Authorization",
                format!("Bearer {}", api_key),
            )
        }
        "zhipu" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("https://open.bigmodel.cn/api/paas")
                .trim_end_matches('/');
            // ZhipuAI: GET on generations endpoint returns 405 = alive
            (
                format!("{}/v4/images/generations", base),
                "Authorization",
                format!("Bearer {}", api_key),
            )
        }
        "tongyi" => {
            let base = base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("https://dashscope.aliyuncs.com")
                .trim_end_matches('/');
            // DashScope: GET on text2image endpoint returns 405 = alive
            (
                format!("{}/api/v1/services/aigc/text2image/image-synthesis", base),
                "Authorization",
                format!("Bearer {}", api_key),
            )
        }
        _ => {
            return Err(serde_json::to_string(&serde_json::json!({
                "success": false,
                "message": format!("Unknown provider: {}", provider_id),
            }))
            .unwrap_or_default());
        }
    };

    let mut req = client.get(&url);
    if !auth_header.is_empty() {
        req = req.header(auth_header, &auth_value);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency = start.elapsed().as_millis() as u64;
            // For Fal, a 405 (Method Not Allowed on GET) or 422 still means connectivity is fine
            let ok = status < 400 || (pid == "fal" && (status == 405 || status == 422));
            let msg = if ok {
                format!("{} 连接成功", display_name)
            } else if status == 401 || status == 403 {
                format!("{} 认证失败，请检查 API Key", display_name)
            } else {
                format!("{} 请求失败 ({})", display_name, status)
            };

            Ok(serde_json::to_string(&serde_json::json!({
                "success": ok,
                "message": msg,
                "url": url.replace(&api_key, "***"),
                "status": status,
                "latencyMs": latency,
                "auth": if auth_header.is_empty() { "Query Parameter" } else { auth_header },
            }))
            .unwrap_or_default())
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            let msg = if e.is_timeout() {
                format!("{} 连接超时，请检查网络或代理设置", display_name)
            } else if e.is_connect() {
                format!("{} 无法连接，请检查网络或 Base URL", display_name)
            } else {
                format!("{} 连接失败: {}", display_name, e)
            };

            Err(serde_json::to_string(&serde_json::json!({
                "success": false,
                "message": msg,
                "url": url.replace(&api_key, "***"),
                "latencyMs": latency,
            }))
            .unwrap_or_default())
        }
    }
}

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

#[tauri::command]
pub async fn set_active_model(
    provider_id: String,
    model_id: String,
    state: State<'_, AppState>,
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

#[tauri::command]
pub async fn has_providers(state: State<'_, AppState>) -> Result<bool, String> {
    let store = state.provider_store.lock().await;
    Ok(!store.providers.is_empty())
}
