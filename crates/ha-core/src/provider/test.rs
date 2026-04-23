//! Connectivity / credential test helpers for providers.
//!
//! Shared by both the Tauri `test_embedding` / `test_image_generate` /
//! `test_model` / `test_proxy` commands and the matching HTTP routes.

use std::time::{Duration, Instant};

use crate::agent::{build_api_url, is_complete_endpoint_url};
use crate::memory;
use crate::provider::{apply_proxy, apply_proxy_from_config, ApiType, ProviderConfig, ProxyConfig};
use crate::truncate_utf8;
use serde_json::{json, Value};

/// Ping an embedding provider with a single "test" document and return a JSON
/// string describing success/dimensions/latency. Never panics — on transport
/// or API errors returns `Err(json_string)` with the same shape.
pub async fn test_embedding(config: memory::EmbeddingConfig) -> Result<String, String> {
    let start = Instant::now();

    match config.provider_type {
        memory::EmbeddingProviderType::Local => {
            let model_id = config
                .local_model_id
                .clone()
                .unwrap_or_else(|| "bge-small-en-v1.5".to_string());
            let model_id_clone = model_id.clone();
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

            let client = apply_proxy(reqwest::Client::builder().timeout(Duration::from_secs(15)))
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
                            "detail": truncate_utf8(&resp_text, 500),
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

            let client = apply_proxy(reqwest::Client::builder().timeout(Duration::from_secs(15)))
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
                        let detail = truncate_utf8(&resp_text, 500);
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
                        let detail = truncate_utf8(&resp_text, 500);
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

/// Ping an image-generation provider with a lightweight GET probe.
pub async fn test_image_generate(
    provider_id: String,
    api_key: String,
    base_url: Option<String>,
) -> Result<String, String> {
    let start = Instant::now();
    let client = apply_proxy(
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

fn probe_model_id(config: &ProviderConfig) -> String {
    config
        .models
        .first()
        .map(|model| model.id.clone())
        .unwrap_or_else(|| "test".to_string())
}

fn build_chat_probe_body(model_id: &str, max_tokens: u32) -> Value {
    json!({
        "model": model_id,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": "Hi" }]
    })
}

fn build_responses_probe_body(model_id: &str, max_output_tokens: u32) -> Value {
    json!({
        "model": model_id,
        "store": false,
        "stream": false,
        "instructions": "Reply briefly.",
        "input": [{ "role": "user", "content": "Hi" }],
        "max_output_tokens": max_output_tokens,
    })
}

fn extract_chat_reply(response_body: &Value) -> String {
    response_body
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .unwrap_or_default()
        .to_string()
}

fn extract_responses_reply(response_body: &Value) -> String {
    response_body
        .get("output")
        .and_then(|output| output.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("message"))
                .filter_map(|item| item.get("content").and_then(|content| content.as_array()))
                .flat_map(|content| content.iter())
                .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some("output_text"))
                .filter_map(|block| block.get("text").and_then(|text| text.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn preview_reply(reply: String) -> String {
    if reply.len() > 100 {
        format!("{}...", truncate_utf8(&reply, 100))
    } else {
        reply
    }
}

fn should_skip_models_preflight(base_url: &str) -> bool {
    is_complete_endpoint_url(base_url)
}

/// Probe the provider's configured endpoint/auth combination and return a JSON
/// string with the same shape consumed by the Settings page.
///
/// Shared by both the Tauri `test_provider` command and the HTTP
/// `POST /api/providers/test` route. On failure returns `Err(json_string)` so
/// callers can surface the payload verbatim.
pub async fn test_provider(config: ProviderConfig) -> Result<String, String> {
    let client = apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(&config.user_agent),
    )
    .build()
    .map_err(|e| format!("Client error: {}", e))?;

    let base = config.base_url.trim_end_matches('/');
    let probe_model = probe_model_id(&config);
    let mut steps: Vec<Value> = Vec::new();
    let total_start = Instant::now();

    macro_rules! build_result {
        ($success:expr, $msg:expr, $url:expr, $status:expr, $auth:expr) => {
            serde_json::to_string(&json!({
                "success": $success,
                "message": $msg,
                "url": $url,
                "status": $status,
                "latencyMs": total_start.elapsed().as_millis() as u64,
                "auth": $auth,
                "steps": steps,
            }))
            .unwrap_or_default()
        };
    }

    match config.api_type {
        ApiType::Anthropic => {
            let url = build_api_url(base, "/v1/messages");
            let body = build_chat_probe_body(&probe_model, 1);

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
            steps.push(json!({
                "endpoint": &url,
                "method": "POST",
                "auth": "x-api-key",
                "status": status,
                "latencyMs": t.elapsed().as_millis() as u64
            }));

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
                let status2 = resp2.status().as_u16();
                steps.push(json!({
                    "endpoint": &url,
                    "method": "POST",
                    "auth": "Bearer",
                    "status": status2,
                    "latencyMs": t2.elapsed().as_millis() as u64
                }));

                if resp2.status().is_success() || status2 == 400 || status2 == 404 {
                    return Ok(build_result!(
                        true,
                        "连接成功（Bearer 认证）",
                        &url,
                        status2,
                        "Bearer"
                    ));
                }

                let detail = resp2.text().await.unwrap_or_default();
                return Err(serde_json::to_string(&json!({
                    "success": false,
                    "message": format!("认证失败 ({})", status2),
                    "detail": detail,
                    "url": &url,
                    "status": status2,
                    "latencyMs": total_start.elapsed().as_millis() as u64,
                    "steps": steps,
                }))
                .unwrap_or_default());
            }

            let detail = resp.text().await.unwrap_or_default();
            Err(serde_json::to_string(&json!({
                "success": false,
                "message": format!("API 错误 ({})", status),
                "detail": detail,
                "url": &url,
                "status": status,
                "latencyMs": total_start.elapsed().as_millis() as u64,
                "steps": steps,
            }))
            .unwrap_or_default())
        }
        ApiType::OpenaiChat => {
            if !should_skip_models_preflight(base) {
                let models_url = build_api_url(base, "/v1/models");
                let t = Instant::now();
                let mut req = client.get(&models_url);
                if !config.api_key.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", config.api_key));
                }
                let resp = req.send().await.map_err(|e| {
                    build_result!(false, format!("连接失败: {}", e), &models_url, 0, "Bearer")
                })?;
                let status = resp.status().as_u16();
                steps.push(json!({
                    "endpoint": &models_url,
                    "method": "GET",
                    "status": status,
                    "latencyMs": t.elapsed().as_millis() as u64
                }));

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
                    return Err(serde_json::to_string(&json!({
                        "success": false,
                        "message": format!("认证失败 ({})", status),
                        "detail": detail,
                        "url": &models_url,
                        "status": status,
                        "latencyMs": total_start.elapsed().as_millis() as u64,
                        "steps": steps,
                    }))
                    .unwrap_or_default());
                }
            }

            let chat_url = build_api_url(base, "/v1/chat/completions");
            let body = build_chat_probe_body(&probe_model, 1);
            let t2 = Instant::now();
            let mut chat_req = client
                .post(&chat_url)
                .header("content-type", "application/json")
                .json(&body);
            if !config.api_key.is_empty() {
                chat_req = chat_req.header("Authorization", format!("Bearer {}", config.api_key));
            }

            match chat_req.send().await {
                Ok(chat_resp) => {
                    let status = chat_resp.status().as_u16();
                    steps.push(json!({
                        "endpoint": &chat_url,
                        "method": "POST",
                        "status": status,
                        "latencyMs": t2.elapsed().as_millis() as u64
                    }));
                    if chat_resp.status().is_success() || status == 400 || status == 404 {
                        Ok(build_result!(
                            true,
                            if status == 200 {
                                "连接成功"
                            } else {
                                "认证成功（模型名需调整）"
                            },
                            &chat_url,
                            status,
                            "Bearer"
                        ))
                    } else if status == 401 || status == 403 {
                        let detail = chat_resp.text().await.unwrap_or_default();
                        Err(serde_json::to_string(&json!({
                            "success": false,
                            "message": format!("认证失败 ({})", status),
                            "detail": detail,
                            "url": &chat_url,
                            "status": status,
                            "latencyMs": total_start.elapsed().as_millis() as u64,
                            "steps": steps,
                        }))
                        .unwrap_or_default())
                    } else {
                        Ok(build_result!(
                            true,
                            "连接成功（不支持模型列表查询）",
                            &chat_url,
                            status,
                            "Bearer"
                        ))
                    }
                }
                Err(e) => {
                    steps.push(json!({
                        "endpoint": &chat_url,
                        "method": "POST",
                        "error": format!("{}", e),
                        "latencyMs": t2.elapsed().as_millis() as u64
                    }));
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
        ApiType::OpenaiResponses => {
            if !should_skip_models_preflight(base) {
                let models_url = build_api_url(base, "/v1/models");
                let t = Instant::now();
                let mut req = client.get(&models_url);
                if !config.api_key.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", config.api_key));
                }
                let resp = req.send().await.map_err(|e| {
                    build_result!(false, format!("连接失败: {}", e), &models_url, 0, "Bearer")
                })?;
                let status = resp.status().as_u16();
                steps.push(json!({
                    "endpoint": &models_url,
                    "method": "GET",
                    "status": status,
                    "latencyMs": t.elapsed().as_millis() as u64
                }));

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
                    return Err(serde_json::to_string(&json!({
                        "success": false,
                        "message": format!("认证失败 ({})", status),
                        "detail": detail,
                        "url": &models_url,
                        "status": status,
                        "latencyMs": total_start.elapsed().as_millis() as u64,
                        "steps": steps,
                    }))
                    .unwrap_or_default());
                }
            }

            let responses_url = build_api_url(base, "/v1/responses");
            let body = build_responses_probe_body(&probe_model, 1);
            let t2 = Instant::now();
            let mut responses_req = client
                .post(&responses_url)
                .header("content-type", "application/json")
                .json(&body);
            if !config.api_key.is_empty() {
                responses_req =
                    responses_req.header("Authorization", format!("Bearer {}", config.api_key));
            }

            match responses_req.send().await {
                Ok(responses_resp) => {
                    let status = responses_resp.status().as_u16();
                    steps.push(json!({
                        "endpoint": &responses_url,
                        "method": "POST",
                        "status": status,
                        "latencyMs": t2.elapsed().as_millis() as u64
                    }));
                    if responses_resp.status().is_success() || status == 400 || status == 404 {
                        Ok(build_result!(
                            true,
                            if status == 200 {
                                "连接成功"
                            } else {
                                "认证成功（模型名需调整）"
                            },
                            &responses_url,
                            status,
                            "Bearer"
                        ))
                    } else if status == 401 || status == 403 {
                        let detail = responses_resp.text().await.unwrap_or_default();
                        Err(serde_json::to_string(&json!({
                            "success": false,
                            "message": format!("认证失败 ({})", status),
                            "detail": detail,
                            "url": &responses_url,
                            "status": status,
                            "latencyMs": total_start.elapsed().as_millis() as u64,
                            "steps": steps,
                        }))
                        .unwrap_or_default())
                    } else {
                        Ok(build_result!(
                            true,
                            "连接成功（不支持模型列表查询）",
                            &responses_url,
                            status,
                            "Bearer"
                        ))
                    }
                }
                Err(e) => {
                    steps.push(json!({
                        "endpoint": &responses_url,
                        "method": "POST",
                        "error": format!("{}", e),
                        "latencyMs": t2.elapsed().as_millis() as u64
                    }));
                    Err(build_result!(
                        false,
                        format!("连接失败: {}", e),
                        &responses_url,
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

// ── Model test (single-turn chat roundtrip) ────────────────────────

/// Issue a minimal chat request against the given provider/model and return
/// a JSON string describing success, latency, and a preview of the reply.
///
/// Shared body behind the Tauri `test_model` command and the HTTP
/// `POST /api/providers/test-model` route. On failure returns
/// `Err(json_string)` with the same shape, so callers can surface details
/// verbatim without re-stringifying.
pub async fn test_model(config: ProviderConfig, model_id: String) -> Result<String, String> {
    let client = apply_proxy(
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
            let body = build_chat_probe_body(&model_id, 32);
            let request_info = json!({
                "url": &url, "method": "POST",
                "headers": { "x-api-key": "***", "anthropic-version": "2023-06-01", "content-type": "application/json" },
                "body": &body,
            });

            // Try `x-api-key` header first; some Anthropic-compatible gateways
            // want `Authorization: Bearer` instead, so we fall back on network
            // errors (not on API errors — the former are the ones that signal
            // "wrong auth scheme" in practice).
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
            let response_body: Value = serde_json::from_str(&body_text).unwrap_or(json!(body_text));

            if status == 200 {
                let preview = preview_reply(
                    response_body
                        .get("content")
                        .and_then(|content| content.as_array())
                        .and_then(|content| content.first())
                        .and_then(|block| block.get("text"))
                        .and_then(|text| text.as_str())
                        .unwrap_or_default()
                        .to_string(),
                );
                Ok(serde_json::to_string(&json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            } else {
                Err(serde_json::to_string(&json!({
                    "success": false, "message": format!("模型测试失败 ({})", status),
                    "model": model_id, "status": status, "latencyMs": latency,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            }
        }
        ApiType::OpenaiChat => {
            let url = build_api_url(base, "/v1/chat/completions");
            let body = build_chat_probe_body(&model_id, 32);
            let auth_header = if !config.api_key.is_empty() {
                "Bearer ***"
            } else {
                "(none)"
            };
            let request_info = json!({
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
            let response_body: Value = serde_json::from_str(&body_text).unwrap_or(json!(body_text));

            if status == 200 {
                let preview = preview_reply(extract_chat_reply(&response_body));
                Ok(serde_json::to_string(&json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            } else {
                Err(serde_json::to_string(&json!({
                    "success": false, "message": format!("模型测试失败 ({})", status),
                    "model": model_id, "status": status, "latencyMs": latency,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            }
        }
        ApiType::OpenaiResponses => {
            let url = build_api_url(base, "/v1/responses");
            let body = build_responses_probe_body(&model_id, 32);
            let auth_header = if !config.api_key.is_empty() {
                "Bearer ***"
            } else {
                "(none)"
            };
            let request_info = json!({
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
                serde_json::to_string(&json!({
                    "success": false, "message": format!("连接失败: {}", e),
                    "model": model_id, "latencyMs": start.elapsed().as_millis() as u64,
                    "request": request_info,
                }))
                .unwrap_or_default()
            })?;

            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let response_body: Value = serde_json::from_str(&body_text).unwrap_or(json!(body_text));

            if status == 200 {
                let preview = preview_reply(extract_responses_reply(&response_body));
                Ok(serde_json::to_string(&json!({
                    "success": true, "message": "模型响应正常",
                    "model": model_id, "status": status, "latencyMs": latency,
                    "reply": preview,
                    "request": request_info, "response": response_body,
                }))
                .unwrap_or_default())
            } else {
                Err(serde_json::to_string(&json!({
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

// ── Proxy test (generic outbound probe) ────────────────────────────

/// Send a single GET against `https://httpbin.org/ip` using the given
/// proxy configuration, returning the human-readable status line.
/// Used by both Tauri `test_proxy` and the HTTP `/api/config/proxy/test`
/// route for the settings-panel "Test proxy" button.
pub async fn test_proxy(config: ProxyConfig) -> Result<String, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15));
    builder = apply_proxy_from_config(builder, &config);
    let client = builder
        .build()
        .map_err(|e| format!("Failed to build client: {}", e))?;

    let start = Instant::now();
    let resp = client
        .get("https://httpbin.org/ip")
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let elapsed = start.elapsed().as_millis();
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status));
    }
    let body = resp.text().await.unwrap_or_default();
    Ok(format!("OK ({}ms)\n{}", elapsed, body))
}

#[cfg(test)]
mod tests {
    use super::{
        build_responses_probe_body, extract_responses_reply, should_skip_models_preflight,
    };
    use serde_json::json;

    #[test]
    fn responses_probe_body_uses_responses_fields() {
        let body = build_responses_probe_body("gpt-5.4", 32);

        assert_eq!(body.get("model").and_then(|v| v.as_str()), Some("gpt-5.4"));
        assert_eq!(
            body.get("max_output_tokens").and_then(|v| v.as_u64()),
            Some(32)
        );
        assert!(body.get("input").is_some());
        assert!(body.get("messages").is_none());
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn extract_responses_reply_concatenates_output_text_blocks() {
        let response = json!({
            "output": [{
                "type": "message",
                "content": [
                    { "type": "output_text", "text": "hello" },
                    { "type": "output_text", "text": " world" }
                ]
            }]
        });

        assert_eq!(extract_responses_reply(&response), "hello world");
    }

    #[test]
    fn complete_endpoint_urls_skip_models_preflight() {
        assert!(should_skip_models_preflight(
            "https://gateway/v1/openai/native/chat/completions"
        ));
        assert!(should_skip_models_preflight(
            "https://gateway/v1/openai/native/responses"
        ));
        assert!(!should_skip_models_preflight("https://gateway/v1"));
    }
}
