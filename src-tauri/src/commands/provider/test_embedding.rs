use crate::memory;

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
