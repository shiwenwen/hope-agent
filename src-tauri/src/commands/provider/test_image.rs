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
