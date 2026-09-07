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

#[derive(Debug, Clone, Copy, Default)]
struct UsageParts {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

fn extract_anthropic_usage(response_body: &Value) -> UsageParts {
    let usage = response_body.get("usage");
    UsageParts {
        input_tokens: usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: usage
            .and_then(|u| u.get("cache_creation_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_read_input_tokens: usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    }
}

fn extract_openai_usage(response_body: &Value) -> UsageParts {
    let usage = response_body.get("usage");
    UsageParts {
        input_tokens: usage
            .and_then(|u| u.get("input_tokens").or_else(|| u.get("prompt_tokens")))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: usage
            .and_then(|u| {
                u.get("output_tokens")
                    .or_else(|| u.get("completion_tokens"))
            })
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: usage
            .and_then(|u| u.get("prompt_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .or_else(|| {
                usage
                    .and_then(|u| u.get("input_tokens_details"))
                    .and_then(|d| d.get("cached_tokens"))
            })
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    }
}

fn extract_embedding_input_tokens(response_body: &Value) -> Option<u64> {
    response_body
        .get("usage")
        .and_then(|u| {
            u.get("prompt_tokens")
                .or_else(|| u.get("input_tokens"))
                .or_else(|| u.get("total_tokens"))
        })
        .or_else(|| {
            response_body.get("usageMetadata").and_then(|u| {
                u.get("promptTokenCount")
                    .or_else(|| u.get("inputTokenCount"))
                    .or_else(|| u.get("totalTokenCount"))
            })
        })
        .and_then(|v| v.as_u64())
}

fn record_provider_test_usage(
    config: &ProviderConfig,
    model_id: &str,
    operation: &'static str,
    latency_ms: u64,
    success: bool,
    status: Option<u16>,
    usage: UsageParts,
    error: Option<String>,
) {
    let mut event =
        crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_PROVIDER_TEST)
            .with_usage(
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_creation_input_tokens,
                usage.cache_read_input_tokens,
            );
    event.operation = Some(operation.to_string());
    event.source = Some("provider_test".to_string());
    event.provider_id = Some(config.id.clone());
    event.provider_name = Some(config.name.clone());
    event.model_id = Some(model_id.to_string());
    event.duration_ms = Some(latency_ms);
    event.success = success;
    event.error = error;
    event.metadata = Some(json!({
        "api_type": config.api_type.display_name(),
        "status": status,
    }));
    crate::model_usage::record_model_usage_best_effort(event);
}

fn record_embedding_test_usage(
    provider_name: &str,
    model_id: &str,
    latency_ms: u64,
    success: bool,
    status: Option<u16>,
    input_tokens: Option<u64>,
    error: Option<String>,
) {
    let mut event = crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_EMBEDDING);
    event.operation = Some("provider.test_embedding".to_string());
    event.source = Some("provider_test".to_string());
    event.provider_name = Some(provider_name.to_string());
    event.model_id = Some(model_id.to_string());
    event.input_tokens = input_tokens;
    event.output_tokens = Some(0);
    event.duration_ms = Some(latency_ms);
    event.success = success;
    event.error = error;
    event.metadata = Some(json!({ "status": status }));
    crate::model_usage::record_model_usage_best_effort(event);
}

/// Ping an embedding provider with a single "test" document and return a JSON
/// string describing success/dimensions/latency. Never panics — on transport
/// or API errors returns `Err(json_string)` with the same shape.
pub async fn test_embedding(config: memory::EmbeddingConfig) -> Result<String, String> {
    let start = Instant::now();

    match config.provider_type {
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
                        let response_body: Value =
                            serde_json::from_str(&resp_text).unwrap_or(Value::Null);
                        let dims = response_body["embedding"]["values"]
                            .as_array()
                            .map(|a| a.len())
                            .unwrap_or(0);
                        record_embedding_test_usage(
                            "Google",
                            &model,
                            latency,
                            true,
                            Some(status),
                            extract_embedding_input_tokens(&response_body),
                            None,
                        );
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
                        record_embedding_test_usage(
                            "Google",
                            &model,
                            latency,
                            false,
                            Some(status),
                            None,
                            Some(format!("API 错误 ({})", status)),
                        );
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
                Err(e) => {
                    let latency = start.elapsed().as_millis() as u64;
                    record_embedding_test_usage(
                        "Google",
                        &model,
                        latency,
                        false,
                        None,
                        None,
                        Some(format!("连接失败: {}", e)),
                    );
                    Err(serde_json::to_string(&serde_json::json!({
                        "success": false,
                        "message": format!("连接失败: {}", e),
                        "url": display_url,
                        "latencyMs": latency,
                    }))
                    .unwrap_or_default())
                }
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
                        let response_body: Value =
                            serde_json::from_str(&resp_text).unwrap_or(Value::Null);
                        let dims = response_body["data"]
                            .as_array()
                            .and_then(|items| items.first())
                            .and_then(|item| item["embedding"].as_array().map(|a| a.len()))
                            .unwrap_or(0);
                        record_embedding_test_usage(
                            "OpenAI-compatible",
                            &model,
                            latency,
                            true,
                            Some(status),
                            extract_embedding_input_tokens(&response_body),
                            None,
                        );
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
                        record_embedding_test_usage(
                            "OpenAI-compatible",
                            &model,
                            latency,
                            false,
                            Some(status),
                            None,
                            Some(format!("认证失败 ({})", status)),
                        );
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
                        record_embedding_test_usage(
                            "OpenAI-compatible",
                            &model,
                            latency,
                            false,
                            Some(status),
                            None,
                            Some(format!("API 错误 ({})", status)),
                        );
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
                Err(e) => {
                    let latency = start.elapsed().as_millis() as u64;
                    record_embedding_test_usage(
                        "OpenAI-compatible",
                        &model,
                        latency,
                        false,
                        None,
                        None,
                        Some(format!("连接失败: {}", e)),
                    );
                    Err(serde_json::to_string(&serde_json::json!({
                        "success": false,
                        "message": format!("连接失败: {}", e),
                        "url": url,
                        "latencyMs": latency,
                    }))
                    .unwrap_or_default())
                }
            }
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

fn extract_anthropic_reply(response_body: &Value) -> String {
    response_body
        .get("content")
        .and_then(|content| content.as_array())
        .map(|blocks| {
            blocks
                .iter()
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

/// Build the `test_model` result for an HTTP 200, distinguishing a *genuinely*
/// empty reply from one merely cut off by the probe's tiny token budget.
///
/// - empty **and not** truncated → failure: a 2xx alone doesn't prove the model
///   is wired up; gateways and misconfigured deployments happily return empty
///   `content`, so the test only passes when there is text to show.
/// - empty **but** truncated (`stop_reason=max_tokens` / `finish_reason=length`
///   / Responses `status=incomplete`) → success: reasoning models routinely
///   spend the whole 32-token probe budget before emitting visible text, which
///   is a budget artifact, not a wiring problem — failing them here is a false
///   negative.
///
/// `request_info` is echoed back in both the success and the failure payload so
/// the Settings "完整日志 → 请求" panel renders for every outcome.
fn ok_or_empty_reply(
    reply: String,
    truncated: bool,
    model_id: &str,
    status: u16,
    latency: u64,
    request_info: &Value,
    response_body: &Value,
) -> Result<String, String> {
    let is_empty = reply.trim().is_empty();
    if is_empty && !truncated {
        return Err(serde_json::to_string(&json!({
            "success": false,
            "message": "模型返回成功但无回复内容",
            "model": model_id, "status": status, "latencyMs": latency,
            "request": request_info, "response": response_body,
        }))
        .unwrap_or_default());
    }
    let message = if is_empty {
        "模型连接正常（回复在测试 token 上限处截断）"
    } else {
        "模型响应正常"
    };
    Ok(serde_json::to_string(&json!({
        "success": true,
        "message": message,
        "model": model_id, "status": status, "latencyMs": latency,
        "reply": preview_reply(reply),
        "request": request_info, "response": response_body,
    }))
    .unwrap_or_default())
}

fn hydrate_probe_credentials(
    config: &mut ProviderConfig,
    stored_providers: &[ProviderConfig],
) -> Result<(), String> {
    let stored = stored_providers
        .iter()
        .find(|stored| stored.id == config.id);
    let missing_key = || {
        "The saved key for this authentication profile is unavailable. Enter a key before testing."
            .to_string()
    };
    if super::is_masked_key(&config.api_key) {
        config.api_key = stored.ok_or_else(missing_key)?.api_key.clone();
    }
    for profile in &mut config.auth_profiles {
        if super::is_masked_key(&profile.api_key) {
            profile.api_key = stored
                .and_then(|stored| {
                    stored
                        .auth_profiles
                        .iter()
                        .find(|saved| saved.id == profile.id)
                })
                .ok_or_else(missing_key)?
                .api_key
                .clone();
        }
    }
    if super::is_masked_key(&config.api_key)
        || config
            .auth_profiles
            .iter()
            .any(|profile| super::is_masked_key(&profile.api_key))
    {
        return Err(missing_key());
    }
    Ok(())
}

fn resolve_probe_profile(config: &mut ProviderConfig) -> Result<Option<String>, String> {
    // HTTP settings only hold masked keys. Restore secrets into this request's
    // draft by stable IDs; never replace unsaved profile settings or write back.
    if config.api_type != ApiType::Codex
        && (super::is_masked_key(&config.api_key)
            || config
                .auth_profiles
                .iter()
                .any(|profile| super::is_masked_key(&profile.api_key)))
    {
        let stored = crate::config::cached_config();
        hydrate_probe_credentials(config, &stored.providers)?;
    }
    super::validate_anthropic_profiles(config).map_err(|error| error.to_string())?;
    if let Some(profile) = config.effective_profiles().into_iter().next() {
        config.base_url = config.resolve_base_url(&profile).to_string();
        config.api_key = profile.api_key;
        return Ok(profile.anthropic_workspace_id);
    }
    if config.api_type != ApiType::Codex && !config.auth_profiles.is_empty() {
        return Err(
            "No enabled authentication profile. Enable a key before testing this provider."
                .to_string(),
        );
    }
    Ok(None)
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
pub async fn test_provider(mut config: ProviderConfig) -> Result<String, String> {
    // Trim stray whitespace from copy-pasted base URL / keys before probing, so
    // the test exercises exactly what `sanitize()` will persist on save.
    config.sanitize();
    let workspace_id = resolve_probe_profile(&mut config)?;
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
                .headers(
                    super::anthropic_headers(
                        &config.base_url,
                        &config.api_key,
                        workspace_id.as_deref(),
                    )
                    .map_err(|error| error.to_string())?,
                )
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    record_provider_test_usage(
                        &config,
                        &probe_model,
                        "provider.test_provider",
                        t.elapsed().as_millis() as u64,
                        false,
                        None,
                        UsageParts::default(),
                        Some(format!("连接失败: {}", e)),
                    );
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

            let is_success = resp.status().is_success();
            if status == 200 {
                let body_text = resp.text().await.unwrap_or_default();
                let response_body: Value =
                    serde_json::from_str(&body_text).unwrap_or(json!(body_text));
                record_provider_test_usage(
                    &config,
                    &probe_model,
                    "provider.test_provider",
                    t.elapsed().as_millis() as u64,
                    true,
                    Some(status),
                    extract_anthropic_usage(&response_body),
                    None,
                );
                return Ok(build_result!(true, "连接成功", &url, status, "x-api-key"));
            } else {
                record_provider_test_usage(
                    &config,
                    &probe_model,
                    "provider.test_provider",
                    t.elapsed().as_millis() as u64,
                    false,
                    Some(status),
                    UsageParts::default(),
                    Some(format!("模型探测返回 {}", status)),
                );
            }

            if workspace_id.is_some() && !is_success {
                return Err(build_result!(
                    false,
                    "工作区绑定请求失败，请检查工作区、密钥权限和模型",
                    &url,
                    status,
                    "x-api-key"
                ));
            }
            if is_success || status == 400 || status == 404 {
                return Ok(build_result!(
                    true,
                    "认证成功（模型名需调整）",
                    &url,
                    status,
                    "x-api-key"
                ));
            }

            if status == 401 || status == 403 {
                let t2 = Instant::now();
                let resp2 = client
                    .post(&url)
                    .headers(
                        super::anthropic_bearer_headers(
                            &config.base_url,
                            &config.api_key,
                            workspace_id.as_deref(),
                        )
                        .map_err(|error| error.to_string())?,
                    )
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| {
                        record_provider_test_usage(
                            &config,
                            &probe_model,
                            "provider.test_provider",
                            t2.elapsed().as_millis() as u64,
                            false,
                            None,
                            UsageParts::default(),
                            Some(format!("连接失败: {}", e)),
                        );
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

                let is_success2 = resp2.status().is_success();
                if status2 == 200 {
                    let body_text = resp2.text().await.unwrap_or_default();
                    let response_body: Value =
                        serde_json::from_str(&body_text).unwrap_or(json!(body_text));
                    record_provider_test_usage(
                        &config,
                        &probe_model,
                        "provider.test_provider",
                        t2.elapsed().as_millis() as u64,
                        true,
                        Some(status2),
                        extract_anthropic_usage(&response_body),
                        None,
                    );
                    return Ok(build_result!(
                        true,
                        "连接成功（Bearer 认证）",
                        &url,
                        status2,
                        "Bearer"
                    ));
                } else {
                    record_provider_test_usage(
                        &config,
                        &probe_model,
                        "provider.test_provider",
                        t2.elapsed().as_millis() as u64,
                        false,
                        Some(status2),
                        UsageParts::default(),
                        Some(format!("模型探测返回 {}", status2)),
                    );
                }

                if is_success2 || status2 == 400 || status2 == 404 {
                    return Ok(build_result!(
                        true,
                        "认证成功（模型名需调整）",
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
                    let is_success = chat_resp.status().is_success();
                    steps.push(json!({
                        "endpoint": &chat_url,
                        "method": "POST",
                        "status": status,
                        "latencyMs": t2.elapsed().as_millis() as u64
                    }));
                    if status == 200 {
                        let body_text = chat_resp.text().await.unwrap_or_default();
                        let response_body: Value =
                            serde_json::from_str(&body_text).unwrap_or(json!(body_text));
                        record_provider_test_usage(
                            &config,
                            &probe_model,
                            "provider.test_provider",
                            t2.elapsed().as_millis() as u64,
                            true,
                            Some(status),
                            extract_openai_usage(&response_body),
                            None,
                        );
                        return Ok(build_result!(true, "连接成功", &chat_url, status, "Bearer"));
                    } else {
                        record_provider_test_usage(
                            &config,
                            &probe_model,
                            "provider.test_provider",
                            t2.elapsed().as_millis() as u64,
                            false,
                            Some(status),
                            UsageParts::default(),
                            Some(format!("模型探测返回 {}", status)),
                        );
                    }

                    if is_success || status == 400 || status == 404 {
                        Ok(build_result!(
                            true,
                            "认证成功（模型名需调整）",
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
                    record_provider_test_usage(
                        &config,
                        &probe_model,
                        "provider.test_provider",
                        t2.elapsed().as_millis() as u64,
                        false,
                        None,
                        UsageParts::default(),
                        Some(format!("连接失败: {}", e)),
                    );
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
                    let is_success = responses_resp.status().is_success();
                    steps.push(json!({
                        "endpoint": &responses_url,
                        "method": "POST",
                        "status": status,
                        "latencyMs": t2.elapsed().as_millis() as u64
                    }));
                    if status == 200 {
                        let body_text = responses_resp.text().await.unwrap_or_default();
                        let response_body: Value =
                            serde_json::from_str(&body_text).unwrap_or(json!(body_text));
                        record_provider_test_usage(
                            &config,
                            &probe_model,
                            "provider.test_provider",
                            t2.elapsed().as_millis() as u64,
                            true,
                            Some(status),
                            extract_openai_usage(&response_body),
                            None,
                        );
                        return Ok(build_result!(
                            true,
                            "连接成功",
                            &responses_url,
                            status,
                            "Bearer"
                        ));
                    } else {
                        record_provider_test_usage(
                            &config,
                            &probe_model,
                            "provider.test_provider",
                            t2.elapsed().as_millis() as u64,
                            false,
                            Some(status),
                            UsageParts::default(),
                            Some(format!("模型探测返回 {}", status)),
                        );
                    }

                    if is_success || status == 400 || status == 404 {
                        Ok(build_result!(
                            true,
                            "认证成功（模型名需调整）",
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
                    record_provider_test_usage(
                        &config,
                        &probe_model,
                        "provider.test_provider",
                        t2.elapsed().as_millis() as u64,
                        false,
                        None,
                        UsageParts::default(),
                        Some(format!("连接失败: {}", e)),
                    );
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
pub async fn test_model(mut config: ProviderConfig, model_id: String) -> Result<String, String> {
    // Trim stray whitespace from copy-pasted base URL / keys / model id before
    // probing, so the test exercises exactly what `sanitize()` persists on save.
    config.sanitize();
    let workspace_id = resolve_probe_profile(&mut config)?;
    let model_id = model_id.trim().to_string();
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
                .headers(
                    super::anthropic_headers(
                        &config.base_url,
                        &config.api_key,
                        workspace_id.as_deref(),
                    )
                    .map_err(|error| error.to_string())?,
                )
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(_) => client
                    .post(&url)
                    .headers(
                        super::anthropic_bearer_headers(
                            &config.base_url,
                            &config.api_key,
                            workspace_id.as_deref(),
                        )
                        .map_err(|error| error.to_string())?,
                    )
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| {
                        record_provider_test_usage(
                            &config,
                            &model_id,
                            "provider.test_model",
                            start.elapsed().as_millis() as u64,
                            false,
                            None,
                            UsageParts::default(),
                            Some(format!("连接失败: {}", e)),
                        );
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
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    latency,
                    true,
                    Some(status),
                    extract_anthropic_usage(&response_body),
                    None,
                );
                let reply = extract_anthropic_reply(&response_body);
                let truncated =
                    response_body.get("stop_reason").and_then(|v| v.as_str()) == Some("max_tokens");
                ok_or_empty_reply(
                    reply,
                    truncated,
                    &model_id,
                    status,
                    latency,
                    &request_info,
                    &response_body,
                )
            } else {
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    latency,
                    false,
                    Some(status),
                    UsageParts::default(),
                    Some(format!("模型测试失败 ({})", status)),
                );
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
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    start.elapsed().as_millis() as u64,
                    false,
                    None,
                    UsageParts::default(),
                    Some(format!("连接失败: {}", e)),
                );
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
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    latency,
                    true,
                    Some(status),
                    extract_openai_usage(&response_body),
                    None,
                );
                let reply = extract_chat_reply(&response_body);
                let truncated = response_body
                    .get("choices")
                    .and_then(|choices| choices.as_array())
                    .and_then(|choices| choices.first())
                    .and_then(|choice| choice.get("finish_reason"))
                    .and_then(|reason| reason.as_str())
                    == Some("length");
                ok_or_empty_reply(
                    reply,
                    truncated,
                    &model_id,
                    status,
                    latency,
                    &request_info,
                    &response_body,
                )
            } else {
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    latency,
                    false,
                    Some(status),
                    UsageParts::default(),
                    Some(format!("模型测试失败 ({})", status)),
                );
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
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    start.elapsed().as_millis() as u64,
                    false,
                    None,
                    UsageParts::default(),
                    Some(format!("连接失败: {}", e)),
                );
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
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    latency,
                    true,
                    Some(status),
                    extract_openai_usage(&response_body),
                    None,
                );
                let reply = extract_responses_reply(&response_body);
                // Responses sets status="incomplete" when the output was cut off
                // (incomplete_details.reason="max_output_tokens").
                let truncated =
                    response_body.get("status").and_then(|v| v.as_str()) == Some("incomplete");
                ok_or_empty_reply(
                    reply,
                    truncated,
                    &model_id,
                    status,
                    latency,
                    &request_info,
                    &response_body,
                )
            } else {
                record_provider_test_usage(
                    &config,
                    &model_id,
                    "provider.test_model",
                    latency,
                    false,
                    Some(status),
                    UsageParts::default(),
                    Some(format!("模型测试失败 ({})", status)),
                );
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
        build_responses_probe_body, extract_anthropic_reply, extract_responses_reply,
        ok_or_empty_reply, should_skip_models_preflight,
    };
    use serde_json::{json, Value};

    #[test]
    fn probe_profiles_hydrate_masked_keys_by_id_and_preserve_draft_edits() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut stored = ProviderConfig::new(
            "stored".into(),
            ApiType::Anthropic,
            "https://api.anthropic.com".into(),
            "synthetic-legacy-secret".into(),
        );
        stored.auth_profiles = vec![
            AuthProfile::new("first".into(), "synthetic-first-secret".into(), None),
            AuthProfile::new("second".into(), "synthetic-second-secret".into(), None),
        ];
        let mut draft = stored.masked();
        draft.auth_profiles.swap(0, 1);
        draft.auth_profiles[0].anthropic_workspace_id = Some("wrkspc_Draft".into());
        draft.auth_profiles[1].enabled = false;
        draft.auth_profiles[1].api_key = "synthetic-unsaved-secret".into();
        super::hydrate_probe_credentials(&mut draft, std::slice::from_ref(&stored)).unwrap();
        assert_eq!(draft.api_key, "synthetic-legacy-secret");
        assert_eq!(draft.auth_profiles[0].api_key, "synthetic-second-secret");
        assert_eq!(draft.auth_profiles[1].api_key, "synthetic-unsaved-secret");
        assert!(!draft.auth_profiles[1].enabled);
        assert_eq!(
            super::resolve_probe_profile(&mut draft).unwrap().as_deref(),
            Some("wrkspc_Draft")
        );
        assert_eq!(draft.api_key, "synthetic-second-secret");
        assert_eq!(stored.auth_profiles[1].anthropic_workspace_id, None);
        assert_eq!(stored.auth_profiles[0].api_key, "synthetic-first-secret");
    }

    #[test]
    fn probe_profiles_reject_unresolvable_masks_without_substituting_another_key() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut stored = ProviderConfig::new(
            "stored".into(),
            ApiType::OpenaiChat,
            "https://example.invalid".into(),
            "synthetic-legacy-secret".into(),
        );
        stored.auth_profiles.push(AuthProfile::new(
            "same label".into(),
            "synthetic-secret".into(),
            None,
        ));
        let mut draft = stored.masked();
        draft.id = "missing-provider".into();
        assert!(
            super::hydrate_probe_credentials(&mut draft, std::slice::from_ref(&stored)).is_err()
        );
        draft = stored.masked();
        draft.auth_profiles[0].id = "missing-profile".into();
        assert!(
            super::hydrate_probe_credentials(&mut draft, std::slice::from_ref(&stored)).is_err()
        );
        // Legacy-only providers still recover their own key, and an explicit
        // empty draft key remains an intentional clear rather than a mask.
        draft = stored.masked();
        draft.auth_profiles.clear();
        super::hydrate_probe_credentials(&mut draft, std::slice::from_ref(&stored)).unwrap();
        assert_eq!(draft.api_key, "synthetic-legacy-secret");
        draft.api_key.clear();
        super::hydrate_probe_credentials(&mut draft, &[stored]).unwrap();
        assert!(draft.api_key.is_empty());
    }

    #[test]
    fn probe_profiles_do_not_fall_back_to_legacy_keys_when_all_are_disabled() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        for api_type in [
            ApiType::Anthropic,
            ApiType::OpenaiChat,
            ApiType::OpenaiResponses,
        ] {
            let mut config = ProviderConfig::new(
                "synthetic".into(),
                api_type,
                "https://api.anthropic.com".into(),
                "synthetic-legacy-key".into(),
            );
            let mut profile =
                AuthProfile::new("profile".into(), "synthetic-profile-key".into(), None);
            profile.enabled = false;
            config.auth_profiles.push(profile);
            assert!(super::resolve_probe_profile(&mut config).is_err());
            assert_eq!(config.api_key, "synthetic-legacy-key");

            config.auth_profiles[0].enabled = true;
            assert_eq!(super::resolve_probe_profile(&mut config).unwrap(), None);
            assert_eq!(config.api_key, "synthetic-profile-key");

            config.auth_profiles.clear();
            config.api_key.clear();
            assert_eq!(super::resolve_probe_profile(&mut config).unwrap(), None);
        }
    }

    #[test]
    fn probe_profiles_use_the_first_enabled_keys_own_workspace() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut config = ProviderConfig::new(
            "synthetic".into(),
            ApiType::Anthropic,
            "https://api.anthropic.com".into(),
            "synthetic-legacy-key".into(),
        );
        let mut disabled =
            AuthProfile::new("disabled".into(), "synthetic-disabled-key".into(), None);
        disabled.enabled = false;
        disabled.anthropic_workspace_id = Some("wrkspc_A".into());
        let mut active = AuthProfile::new("active".into(), "synthetic-active-key".into(), None);
        active.anthropic_workspace_id = Some("wrkspc_B".into());
        config.auth_profiles = vec![disabled, active];

        assert_eq!(
            super::resolve_probe_profile(&mut config)
                .unwrap()
                .as_deref(),
            Some("wrkspc_B")
        );
        assert_eq!(config.api_key, "synthetic-active-key");
    }

    #[test]
    fn probe_profiles_keep_codex_oauth_independent_of_api_key_profiles() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut config =
            ProviderConfig::new("Codex".into(), ApiType::Codex, String::new(), String::new());
        let mut disabled = AuthProfile::new("unused".into(), "synthetic-key".into(), None);
        disabled.enabled = false;
        config.auth_profiles.push(disabled);
        assert_eq!(super::resolve_probe_profile(&mut config).unwrap(), None);
        assert!(config.api_key.is_empty());
    }

    #[test]
    fn extract_anthropic_reply_joins_text_blocks_and_ignores_thinking() {
        let response = json!({
            "content": [
                { "type": "thinking", "thinking": "let me think" },
                { "type": "text", "text": "Hello" },
                { "type": "text", "text": " there" }
            ]
        });
        assert_eq!(extract_anthropic_reply(&response), "Hello there");
    }

    #[test]
    fn ok_or_empty_reply_fails_only_when_empty_and_not_truncated() {
        let request = json!({ "url": "https://api.example.com", "method": "POST" });
        let response = json!({ "raw": "body" });

        // Genuinely empty (not truncated) → failure, and request is echoed back.
        let err = ok_or_empty_reply(String::new(), false, "m", 200, 5, &request, &response)
            .expect_err("empty non-truncated reply must fail");
        let v: Value = serde_json::from_str(&err).unwrap();
        assert_eq!(v["success"], json!(false));
        assert!(
            v.get("request").is_some(),
            "failure payload must include request"
        );

        // Empty but truncated by the probe token budget → success (not a wiring bug).
        let ok = ok_or_empty_reply(String::new(), true, "m", 200, 5, &request, &response)
            .expect("empty truncated reply must pass");
        let v: Value = serde_json::from_str(&ok).unwrap();
        assert_eq!(v["success"], json!(true));
        assert!(v.get("request").is_some());

        // Non-empty → success with the reply echoed.
        let ok = ok_or_empty_reply("hi".to_string(), false, "m", 200, 5, &request, &response)
            .expect("non-empty reply must pass");
        let v: Value = serde_json::from_str(&ok).unwrap();
        assert_eq!(v["success"], json!(true));
        assert_eq!(v["reply"], json!("hi"));
    }

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
