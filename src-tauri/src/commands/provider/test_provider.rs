use crate::agent::build_api_url;
use crate::provider::{ApiType, ProviderConfig};
use crate::truncate_utf8;

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
