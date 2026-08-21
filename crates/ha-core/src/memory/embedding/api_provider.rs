use std::mem::ManuallyDrop;

use anyhow::{Context, Result};

use super::config::{EmbeddingConfig, EmbeddingProviderType, EmbeddingPurpose};
use super::utils::{l2_normalize, truncate_for_model};
use crate::memory::traits::{EmbeddingProvider, MultimodalInput};

// ── API Embedding Provider ───────────────────────────────────────

fn sanitized_api_endpoint(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }

    raw.find(['?', '#'])
        .map(|index| raw[..index].to_string())
        .unwrap_or_else(|| raw.to_string())
}

fn transport_error_class(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_decode() {
        "decode"
    } else if error.is_body() {
        "body"
    } else if error.is_request() {
        "request"
    } else {
        "transport"
    }
}

fn send_embedding_request(
    request: reqwest::blocking::RequestBuilder,
    operation: &'static str,
) -> Result<(reqwest::StatusCode, String)> {
    let response = request.send().map_err(|error| {
        anyhow::anyhow!(
            "Embedding provider request failed: operation={}, class={}",
            operation,
            transport_error_class(&error)
        )
    })?;
    let status = response.status();
    let response_text = response.text().map_err(|error| {
        anyhow::anyhow!(
            "Embedding provider response read failed: operation={}, class={}",
            operation,
            transport_error_class(&error)
        )
    })?;
    Ok((status, response_text))
}

fn http_error(operation: &'static str, status: reqwest::StatusCode) -> anyhow::Error {
    let class = match status.as_u16() {
        401 | 403 => "authentication",
        408 => "timeout",
        409 => "conflict",
        429 => "rate_limited",
        400..=499 => "client",
        500..=599 => "server",
        _ => "http",
    };
    anyhow::anyhow!(
        "Embedding provider HTTP error: operation={}, status={}, class={}",
        operation,
        status.as_u16(),
        class
    )
}

/// OpenAI-compatible /v1/embeddings API provider.
pub struct ApiEmbeddingProvider {
    // `reqwest::blocking::Client` owns a tokio runtime; dropping it inside a
    // tokio worker panics. ManuallyDrop lets the [`Drop`] impl below move the
    // client onto a non-tokio OS thread when needed.
    client: ManuallyDrop<reqwest::blocking::Client>,
    base_url: String,
    api_key: String,
    model: String,
    dimensions: u32,
    provider_type: EmbeddingProviderType,
}

impl Drop for ApiEmbeddingProvider {
    fn drop(&mut self) {
        // SAFETY: ManuallyDrop::take 在此处 Drop 路径里调用且仅一次。
        let client = unsafe { ManuallyDrop::take(&mut self.client) };
        // 仅当 caller 处于 tokio runtime 内（drop runtime 会触发 panic）时才
        // 把销毁挪到独立 OS 线程。非 tokio 上下文（测试 / CLI 同步路径 / 进程
        // 退出阶段）直接 inline drop——避免在 hot path 无界 spawn detached 线程。
        // 不能用 `tokio::task::spawn_blocking`：blocking pool 仍属 tokio runtime。
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::spawn(move || drop(client));
        } else {
            drop(client);
        }
    }
}

impl ApiEmbeddingProvider {
    fn provider_label(&self) -> &'static str {
        match self.provider_type {
            EmbeddingProviderType::OpenaiCompatible => "openai_compatible",
            EmbeddingProviderType::Google => "google",
        }
    }

    fn record_embedding_usage(
        &self,
        operation: &'static str,
        purpose: EmbeddingPurpose,
        text_count: usize,
        duration_ms: u64,
        input_tokens: Option<u64>,
        success: bool,
        error: Option<String>,
    ) {
        let mut event =
            crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_EMBEDDING);
        event.operation = Some(operation.to_string());
        event.source = Some("embedding".to_string());
        event.provider_name = Some(self.provider_label().to_string());
        event.model_id = Some(self.model.clone());
        event.input_tokens = input_tokens;
        event.duration_ms = Some(duration_ms);
        event.success = success;
        event.error = error;
        event.metadata = Some(serde_json::json!({
            "text_count": text_count,
            "dimensions": self.dimensions,
            "endpoint": sanitized_api_endpoint(&self.base_url),
            "purpose": purpose.as_str(),
        }));
        crate::model_usage::record_model_usage_best_effort(event);
    }

    /// Drive `self.client` on a fresh non-tokio OS thread.
    ///
    /// `reqwest::blocking::Client` 的方法（`.send()` / `.text()` / `.bytes()`）
    /// 在 debug build 下经 `wait::enter()` 构造并立即 drop 一个临时
    /// `current_thread` 运行时；当调用栈处于任何 tokio 运行时上下文中
    /// （worker 线程或 `spawn_blocking` 线程，二者默认线程名都叫
    /// `tokio-rt-worker`）时，那次 drop 会以
    /// "Cannot drop a runtime in a context where blocking is not allowed"
    /// 触发 panic。channel 侧的 `memory_extract` / active_memory 召回都
    /// 经 `tokio::spawn` / `spawn_blocking` 串到这里，所以每次 reqwest
    /// 调用都必须切到一根全新的 OS 线程。
    fn run_off_runtime<R, F>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&reqwest::blocking::Client) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let client: reqwest::blocking::Client = (*self.client).clone();
        std::thread::Builder::new()
            .name("embedding-api-call".into())
            .spawn(move || f(&client))
            .map_err(|e| anyhow::anyhow!("Failed to spawn embedding call thread: {}", e))?
            .join()
            .map_err(|_| anyhow::anyhow!("Embedding call thread panicked"))?
    }

    pub fn new(config: &EmbeddingConfig) -> Result<Self> {
        let base_url = config
            .api_base_url
            .as_deref()
            .unwrap_or("https://api.openai.com")
            .to_string();
        let api_key = config.api_key.as_deref().unwrap_or("").to_string();
        let model = config
            .api_model
            .as_deref()
            .unwrap_or("text-embedding-3-small")
            .to_string();
        let dimensions = config.api_dimensions.unwrap_or(1536);

        // reqwest 0.13 `blocking::Client::new` 在 debug build 下经
        // `wait::enter()` 创建+立即 drop 一个临时 current_thread runtime；在
        // tokio worker 上 drop 它会 panic（`tauri dev` 是 debug）。挪到一个
        // 无 tokio context 的独立 OS 线程构造规避。
        let client = std::thread::Builder::new()
            .name("embedding-client-build".into())
            .spawn(|| -> Result<reqwest::blocking::Client> {
                crate::provider::apply_proxy_blocking(
                    reqwest::blocking::Client::builder()
                        .connect_timeout(std::time::Duration::from_secs(10))
                        .timeout(std::time::Duration::from_secs(30)),
                )
                .build()
                .context("Failed to build embedding HTTP client")
            })
            .context("Failed to spawn embedding HTTP client builder thread")?
            .join()
            .map_err(|_| anyhow::anyhow!("Embedding HTTP client builder thread panicked"))??;

        Ok(Self {
            client: ManuallyDrop::new(client),
            base_url,
            api_key,
            model,
            dimensions,
            provider_type: config.provider_type.clone(),
        })
    }

    fn apply_openai_compatible_purpose(
        &self,
        body: &mut serde_json::Value,
        purpose: EmbeddingPurpose,
    ) {
        if self.base_url.contains("voyageai.com") {
            match purpose {
                EmbeddingPurpose::Query => body["input_type"] = serde_json::json!("query"),
                EmbeddingPurpose::Document => body["input_type"] = serde_json::json!("document"),
                EmbeddingPurpose::Symmetric => {}
            }
        } else if self.base_url.contains("jina.ai") {
            body["task"] = serde_json::json!(match purpose {
                EmbeddingPurpose::Query => "retrieval.query",
                EmbeddingPurpose::Document => "retrieval.passage",
                EmbeddingPurpose::Symmetric => "text-matching",
            });
        } else if self.base_url.contains("cohere") {
            body["input_type"] = serde_json::json!(match purpose {
                EmbeddingPurpose::Query => "search_query",
                EmbeddingPurpose::Document => "search_document",
                EmbeddingPurpose::Symmetric => "clustering",
            });
        }
    }

    fn prepare_google_text(&self, text: &str, purpose: EmbeddingPurpose) -> String {
        if !self.model.contains("embedding-2") {
            return text.to_string();
        }
        match purpose {
            EmbeddingPurpose::Query => format!("task: search result | query: {text}"),
            EmbeddingPurpose::Document => format!("title: none | text: {text}"),
            EmbeddingPurpose::Symmetric => {
                format!("task: sentence similarity | query: {text}")
            }
        }
    }

    fn apply_google_task_type(&self, request: &mut serde_json::Value, purpose: EmbeddingPurpose) {
        // Gemini Embedding 2 does not support taskType; it uses the prompt
        // structures prepared above. Embedding 1 uses the explicit enum.
        if !self.model.contains("embedding-2") {
            request["taskType"] = serde_json::json!(match purpose {
                EmbeddingPurpose::Query => "RETRIEVAL_QUERY",
                EmbeddingPurpose::Document => "RETRIEVAL_DOCUMENT",
                EmbeddingPurpose::Symmetric => "SEMANTIC_SIMILARITY",
            });
        }
    }

    fn call_openai_compatible(
        &self,
        texts: &[String],
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<Vec<f32>>> {
        let texts = truncate_for_model(texts, &self.model);
        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": self.model,
            "input": &texts,
        });

        // Some APIs support specifying dimensions
        if self.dimensions > 0 {
            body["dimensions"] = serde_json::json!(self.dimensions);
        }

        self.apply_openai_compatible_purpose(&mut body, purpose);

        // Embedding content and provider payloads are deliberately excluded
        // from logs. Only bounded operational metadata may be recorded here.
        if let Some(logger) = crate::get_logger() {
            let body_size = serde_json::to_vec(&body).map_or(0, |encoded| encoded.len());
            let safe_url = sanitized_api_endpoint(&url);
            logger.log(
                "debug",
                "memory",
                "embedding::openai_compatible::request",
                &format!(
                    "Embedding API request: {} texts, model={}, url={}, body {}B",
                    texts.len(),
                    self.model,
                    safe_url,
                    body_size
                ),
                Some(
                    serde_json::json!({
                        "api_url": safe_url,
                        "model": &self.model,
                        "text_count": texts.len(),
                        "dimensions": self.dimensions,
                        "purpose": purpose.as_str(),
                        "request_size_bytes": body_size,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let url_owned = url.clone();
        let api_key_owned = self.api_key.clone();
        let body_owned = body.clone();
        let (status, resp_text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .post(&url_owned)
                    .header("Authorization", format!("Bearer {}", api_key_owned))
                    .header("Content-Type", "application/json")
                    .json(&body_owned),
                "openai_compatible",
            )
        })?;
        let ttfb_ms = request_start.elapsed().as_millis() as u64;

        // Log embedding API response
        if let Some(logger) = crate::get_logger() {
            let level = if status.is_success() {
                "debug"
            } else {
                "error"
            };
            logger.log(
                level,
                "memory",
                "embedding::openai_compatible::response",
                &format!(
                    "Embedding API response: status={}, ttfb={}ms, body {}B",
                    status.as_u16(),
                    ttfb_ms,
                    resp_text.len()
                ),
                Some(
                    serde_json::json!({
                        "status": status.as_u16(),
                        "ttfb_ms": ttfb_ms,
                        "response_size_bytes": resp_text.len(),
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !status.is_success() {
            self.record_embedding_usage(
                "embedding.openai_compatible",
                purpose,
                texts.len(),
                ttfb_ms,
                None,
                false,
                Some(format!("HTTP {}", status.as_u16())),
            );
            return Err(http_error("openai_compatible", status));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let usage_tokens = resp_json
            .get("usage")
            .and_then(|u| {
                u.get("prompt_tokens")
                    .or_else(|| u.get("input_tokens"))
                    .or_else(|| u.get("total_tokens"))
            })
            .and_then(|v| v.as_u64());
        let data = resp_json["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding API response"))?;

        let mut results = Vec::new();
        for item in data {
            let embedding = item["embedding"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing embedding in response"))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }

        self.record_embedding_usage(
            "embedding.openai_compatible",
            purpose,
            texts.len(),
            ttfb_ms,
            usage_tokens,
            true,
            None,
        );
        Ok(results)
    }

    /// Batch embed via Google Gemini `batchEmbedContents` API (up to 100 texts per request).
    /// Falls back to single `embedContent` if batch fails.
    fn call_google(&self, texts: &[String], purpose: EmbeddingPurpose) -> Result<Vec<Vec<f32>>> {
        let texts = truncate_for_model(texts, &self.model)
            .into_iter()
            .map(|text| self.prepare_google_text(&text, purpose))
            .collect::<Vec<_>>();
        const BATCH_SIZE: usize = 100; // Gemini batch limit

        let mut all_results = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(BATCH_SIZE) {
            match self.call_google_batch(chunk, purpose) {
                Ok(mut batch_results) => {
                    all_results.append(&mut batch_results);
                }
                Err(batch_err) => {
                    // Fallback: single embedContent per text
                    if let Some(logger) = crate::get_logger() {
                        logger.log(
                            "warn",
                            "memory",
                            "embedding::google::batch_fallback",
                            &format!(
                                "Batch embedContent failed, falling back to single requests: {}",
                                batch_err
                            ),
                            None,
                            None,
                            None,
                        );
                    }
                    for text in chunk {
                        let result = self.call_google_single(text, purpose)?;
                        all_results.push(result);
                    }
                }
            }
        }

        Ok(all_results)
    }

    /// Batch embed via `batchEmbedContents` endpoint.
    fn call_google_batch(
        &self,
        texts: &[String],
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<Vec<f32>>> {
        let url = format!(
            "{}/v1beta/models/{}:batchEmbedContents?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let model_path = format!("models/{}", self.model);
        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|text| {
                let mut req = serde_json::json!({
                    "model": &model_path,
                    "content": { "parts": [{"text": text}] }
                });
                if self.dimensions > 0 {
                    req["outputDimensionality"] = serde_json::json!(self.dimensions);
                }
                self.apply_google_task_type(&mut req, purpose);
                req
            })
            .collect();

        let body = serde_json::json!({ "requests": requests });

        // Log batch request
        if let Some(logger) = crate::get_logger() {
            let safe_url = sanitized_api_endpoint(&url);
            let body_size = serde_json::to_vec(&body).map_or(0, |encoded| encoded.len());
            logger.log(
                "debug",
                "memory",
                "embedding::google::batch_request",
                &format!(
                    "Google Batch Embedding API: {} texts, model={}",
                    texts.len(),
                    self.model
                ),
                Some(
                    serde_json::json!({
                        "api_url": safe_url,
                        "model": &self.model,
                        "text_count": texts.len(),
                        "dimensions": self.dimensions,
                        "purpose": purpose.as_str(),
                        "request_size_bytes": body_size,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let url_owned = url.clone();
        let body_owned = body.clone();
        let (status, resp_text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .post(&url_owned)
                    .header("Content-Type", "application/json")
                    .json(&body_owned),
                "google_batch",
            )
        })?;
        let ttfb_ms = request_start.elapsed().as_millis() as u64;

        // Log batch response
        if let Some(logger) = crate::get_logger() {
            let level = if status.is_success() {
                "debug"
            } else {
                "error"
            };
            logger.log(
                level,
                "memory",
                "embedding::google::batch_response",
                &format!(
                    "Google Batch Embedding API response: status={}, ttfb={}ms, body {}B",
                    status.as_u16(),
                    ttfb_ms,
                    resp_text.len()
                ),
                Some(
                    serde_json::json!({
                        "status": status.as_u16(),
                        "ttfb_ms": ttfb_ms,
                        "text_count": texts.len(),
                        "response_size_bytes": resp_text.len(),
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !status.is_success() {
            self.record_embedding_usage(
                "embedding.google_batch",
                purpose,
                texts.len(),
                ttfb_ms,
                None,
                false,
                Some(format!("HTTP {}", status.as_u16())),
            );
            return Err(http_error("google_batch", status));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let usage_tokens = resp_json
            .get("usageMetadata")
            .and_then(|u| {
                u.get("totalTokenCount")
                    .or_else(|| u.get("promptTokenCount"))
                    .or_else(|| u.get("inputTokenCount"))
            })
            .and_then(|v| v.as_u64());
        let embeddings = resp_json["embeddings"].as_array().ok_or_else(|| {
            anyhow::anyhow!("Invalid Google batch embedding response: missing 'embeddings' array")
        })?;

        let mut results = Vec::with_capacity(embeddings.len());
        for emb in embeddings {
            let values = emb["values"].as_array().ok_or_else(|| {
                anyhow::anyhow!("Invalid Google batch embedding response: missing 'values'")
            })?;
            let embedding: Vec<f32> = values
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }

        self.record_embedding_usage(
            "embedding.google_batch",
            purpose,
            texts.len(),
            ttfb_ms,
            usage_tokens,
            true,
            None,
        );
        Ok(results)
    }

    /// Single text embed via `embedContent` endpoint (fallback).
    fn call_google_single(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>> {
        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let mut body = serde_json::json!({
            "content": { "parts": [{"text": text}] }
        });
        if self.dimensions > 0 {
            body["outputDimensionality"] = serde_json::json!(self.dimensions);
        }
        self.apply_google_task_type(&mut body, purpose);

        if let Some(logger) = crate::get_logger() {
            let safe_url = sanitized_api_endpoint(&url);
            let body_size = serde_json::to_vec(&body).map_or(0, |encoded| encoded.len());
            logger.log(
                "debug",
                "memory",
                "embedding::google::single_request",
                &format!(
                    "Google Embedding API single request: model={}, text_len={}",
                    self.model,
                    text.len()
                ),
                Some(
                    serde_json::json!({
                        "api_url": safe_url,
                        "model": &self.model,
                        "text_length": text.len(),
                        "dimensions": self.dimensions,
                        "purpose": purpose.as_str(),
                        "request_size_bytes": body_size,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let url_owned = url.clone();
        let body_owned = body.clone();
        let (status, resp_text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .post(&url_owned)
                    .header("Content-Type", "application/json")
                    .json(&body_owned),
                "google_single",
            )
        })?;
        let ttfb_ms = request_start.elapsed().as_millis() as u64;

        if let Some(logger) = crate::get_logger() {
            let level = if status.is_success() {
                "debug"
            } else {
                "error"
            };
            logger.log(
                level,
                "memory",
                "embedding::google::single_response",
                &format!(
                    "Google Embedding API single response: status={}, ttfb={}ms",
                    status.as_u16(),
                    ttfb_ms
                ),
                Some(
                    serde_json::json!({
                        "status": status.as_u16(),
                        "ttfb_ms": ttfb_ms,
                        "response_size_bytes": resp_text.len(),
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !status.is_success() {
            self.record_embedding_usage(
                "embedding.google_single",
                purpose,
                1,
                ttfb_ms,
                None,
                false,
                Some(format!("HTTP {}", status.as_u16())),
            );
            return Err(http_error("google_single", status));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let usage_tokens = resp_json
            .get("usageMetadata")
            .and_then(|u| {
                u.get("totalTokenCount")
                    .or_else(|| u.get("promptTokenCount"))
                    .or_else(|| u.get("inputTokenCount"))
            })
            .and_then(|v| v.as_u64());
        let values = resp_json["embedding"]["values"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google embedding response"))?;

        self.record_embedding_usage(
            "embedding.google_single",
            purpose,
            1,
            ttfb_ms,
            usage_tokens,
            true,
            None,
        );
        Ok(values
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect())
    }

    /// Multimodal embed via Gemini `embedContent` with inline data (image/audio).
    /// Only works with gemini-embedding-2.
    fn call_google_multimodal(
        &self,
        input: &MultimodalInput,
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<f32>> {
        use base64::Engine;

        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let b64_data = base64::engine::general_purpose::STANDARD.encode(&input.file_data);

        let prepared_label = self.prepare_google_text(&input.label, purpose);
        let mut body = serde_json::json!({
            "content": {
                "parts": [
                    { "text": prepared_label },
                    { "inlineData": {
                        "mimeType": &input.mime_type,
                        "data": &b64_data,
                    }}
                ]
            }
        });
        if self.dimensions > 0 {
            body["outputDimensionality"] = serde_json::json!(self.dimensions);
        }

        if let Some(logger) = crate::get_logger() {
            let safe_url = sanitized_api_endpoint(&url);
            logger.log(
                "info",
                "memory",
                "embedding::google::multimodal_request",
                &format!(
                    "Multimodal embedding: model={}, mime={}, file_size={}B, label_size={}B",
                    self.model,
                    input.mime_type,
                    input.file_data.len(),
                    input.label.len()
                ),
                Some(
                    serde_json::json!({
                        "api_url": safe_url,
                        "model": &self.model,
                        "mime_type": &input.mime_type,
                        "file_size_bytes": input.file_data.len(),
                        "base64_size_bytes": b64_data.len(),
                        "purpose": purpose.as_str(),
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let url_owned = url.clone();
        let body_owned = body.clone();
        let (status, resp_text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .post(&url_owned)
                    .header("Content-Type", "application/json")
                    .json(&body_owned),
                "google_multimodal",
            )
        })?;
        let ttfb_ms = request_start.elapsed().as_millis() as u64;

        if let Some(logger) = crate::get_logger() {
            let level = if status.is_success() { "info" } else { "error" };
            logger.log(
                level,
                "memory",
                "embedding::google::multimodal_response",
                &format!(
                    "Multimodal embedding response: status={}, ttfb={}ms",
                    status.as_u16(),
                    ttfb_ms
                ),
                None,
                None,
                None,
            );
        }

        if !status.is_success() {
            self.record_embedding_usage(
                "embedding.google_multimodal",
                purpose,
                1,
                ttfb_ms,
                None,
                false,
                Some(format!("HTTP {}", status.as_u16())),
            );
            return Err(http_error("google_multimodal", status));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let usage_tokens = resp_json
            .get("usageMetadata")
            .and_then(|u| {
                u.get("totalTokenCount")
                    .or_else(|| u.get("promptTokenCount"))
                    .or_else(|| u.get("inputTokenCount"))
            })
            .and_then(|v| v.as_u64());
        let values = resp_json["embedding"]["values"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google multimodal embedding response"))?;

        self.record_embedding_usage(
            "embedding.google_multimodal",
            purpose,
            1,
            ttfb_ms,
            usage_tokens,
            true,
            None,
        );
        Ok(values
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect())
    }

    // ── Async Batch API (OpenAI / Voyage compatible) ──

    /// Check if this provider supports the async Batch API.
    fn batch_api_supported(&self) -> bool {
        match self.provider_type {
            EmbeddingProviderType::OpenaiCompatible => {
                // OpenAI and Voyage support Batch API
                self.base_url.contains("openai.com") || self.base_url.contains("voyageai.com")
            }
            _ => false, // Gemini uses batchEmbedContents (already synchronous batch)
        }
    }

    /// Upload a JSONL file for batch processing.
    fn batch_upload_jsonl(&self, jsonl_content: &str) -> Result<String> {
        let url = format!("{}/v1/files", self.base_url.trim_end_matches('/'));

        let boundary = format!("----BatchBoundary{}", chrono::Utc::now().timestamp_millis());
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nbatch\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"memory-embeddings.jsonl\"\r\nContent-Type: application/jsonl\r\n\r\n{jsonl_content}\r\n--{boundary}--\r\n",
        );

        let url_owned = url.clone();
        let api_key_owned = self.api_key.clone();
        let boundary_owned = boundary.clone();
        let body_owned = body;
        let (status, resp_text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .post(&url_owned)
                    .header("Authorization", format!("Bearer {}", api_key_owned))
                    .header(
                        "Content-Type",
                        format!("multipart/form-data; boundary={}", boundary_owned),
                    )
                    .body(body_owned),
                "batch_upload",
            )
        })?;
        if !status.is_success() {
            return Err(http_error("batch_upload", status));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        resp_json["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Missing file id in upload response"))
    }

    /// Create a batch job.
    fn batch_create(&self, input_file_id: &str, purpose: EmbeddingPurpose) -> Result<String> {
        let url = format!("{}/v1/batches", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "input_file_id": input_file_id,
            "endpoint": "/v1/embeddings",
            "completion_window": "24h",
        });

        // Voyage needs request_params
        if self.base_url.contains("voyageai.com") {
            body["completion_window"] = serde_json::json!("12h");
            body["request_params"] = serde_json::json!({
                "model": &self.model,
            });
            self.apply_openai_compatible_purpose(&mut body["request_params"], purpose);
        }

        let url_owned = url.clone();
        let api_key_owned = self.api_key.clone();
        let body_owned = body.clone();
        let (status, resp_text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .post(&url_owned)
                    .header("Authorization", format!("Bearer {}", api_key_owned))
                    .header("Content-Type", "application/json")
                    .json(&body_owned),
                "batch_create",
            )
        })?;
        if !status.is_success() {
            return Err(http_error("batch_create", status));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        resp_json["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Missing batch id in create response"))
    }

    /// Poll batch status until completion or failure.
    fn batch_poll(&self, batch_id: &str, timeout_ms: u64, poll_interval_ms: u64) -> Result<String> {
        let url = format!(
            "{}/v1/batches/{}",
            self.base_url.trim_end_matches('/'),
            batch_id
        );
        let start = std::time::Instant::now();

        loop {
            let url_owned = url.clone();
            let api_key_owned = self.api_key.clone();
            let (status, resp_text) = self.run_off_runtime(move |client| {
                send_embedding_request(
                    client
                        .get(&url_owned)
                        .header("Authorization", format!("Bearer {}", api_key_owned)),
                    "batch_poll",
                )
            })?;
            if !status.is_success() {
                return Err(http_error("batch_poll", status));
            }
            let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
            let state = resp_json["status"].as_str().unwrap_or("unknown");

            match state {
                "completed" => {
                    return resp_json["output_file_id"]
                        .as_str()
                        .map(String::from)
                        .ok_or_else(|| anyhow::anyhow!("Batch completed but no output_file_id"));
                }
                "failed" | "expired" | "cancelled" | "canceled" => {
                    anyhow::bail!("Batch {} reached terminal state {}", batch_id, state);
                }
                _ => {
                    if start.elapsed().as_millis() as u64 > timeout_ms {
                        anyhow::bail!(
                            "Batch {} timed out after {}ms (state: {})",
                            batch_id,
                            timeout_ms,
                            state
                        );
                    }
                    if let Some(logger) = crate::get_logger() {
                        logger.log(
                            "debug",
                            "memory",
                            "embedding::batch_poll",
                            &format!(
                                "Batch {} state={}, waiting {}ms",
                                batch_id, state, poll_interval_ms
                            ),
                            None,
                            None,
                            None,
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(poll_interval_ms));
                }
            }
        }
    }

    /// Download batch output file content (JSONL).
    fn batch_download_output(&self, file_id: &str) -> Result<String> {
        let url = format!(
            "{}/v1/files/{}/content",
            self.base_url.trim_end_matches('/'),
            file_id
        );

        let url_owned = url.clone();
        let api_key_owned = self.api_key.clone();
        let (status, text) = self.run_off_runtime(move |client| {
            send_embedding_request(
                client
                    .get(&url_owned)
                    .header("Authorization", format!("Bearer {}", api_key_owned)),
                "batch_download",
            )
        })?;
        if !status.is_success() {
            return Err(http_error("batch_download", status));
        }
        Ok(text)
    }

    /// Run the complete async Batch API flow: upload JSONL -> create batch -> poll -> download -> parse.
    fn run_batch_api(
        &self,
        items: &[(String, String)],
        purpose: EmbeddingPurpose,
    ) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        use std::collections::HashMap;
        const MAX_BATCH_SIZE: usize = 50_000;
        const POLL_INTERVAL_MS: u64 = 5_000;
        const TIMEOUT_MS: u64 = 60 * 60 * 1_000; // 60 minutes

        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info",
                "memory",
                "embedding::batch_api",
                &format!(
                    "Starting async Batch API: {} items, model={}",
                    items.len(),
                    self.model
                ),
                None,
                None,
                None,
            );
        }

        let mut all_results: HashMap<String, Vec<f32>> = HashMap::new();

        for chunk in items.chunks(MAX_BATCH_SIZE) {
            // Build JSONL
            let jsonl: String = chunk
                .iter()
                .map(|(id, text)| {
                    let mut body = serde_json::json!({
                        "model": &self.model,
                        "input": text,
                    });
                    if self.dimensions > 0 {
                        body["dimensions"] = serde_json::json!(self.dimensions);
                    }
                    self.apply_openai_compatible_purpose(&mut body, purpose);
                    serde_json::json!({
                        "custom_id": id,
                        "method": "POST",
                        "url": "/v1/embeddings",
                        "body": body,
                    })
                    .to_string()
                })
                .collect::<Vec<_>>()
                .join("\n");

            // Upload -> Create -> Poll -> Download
            let file_id = self.batch_upload_jsonl(&jsonl)?;
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "info",
                    "memory",
                    "embedding::batch_api",
                    &format!(
                        "Batch JSONL uploaded: file_id={}, {} items",
                        file_id,
                        chunk.len()
                    ),
                    None,
                    None,
                    None,
                );
            }

            let batch_id = self.batch_create(&file_id, purpose)?;
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "info",
                    "memory",
                    "embedding::batch_api",
                    &format!("Batch created: batch_id={}", batch_id),
                    None,
                    None,
                    None,
                );
            }

            let output_file_id = self.batch_poll(&batch_id, TIMEOUT_MS, POLL_INTERVAL_MS)?;
            let output = self.batch_download_output(&output_file_id)?;

            // Parse JSONL output
            for line in output.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parsed: serde_json::Value =
                    serde_json::from_str(line).context("Invalid embedding batch output JSONL")?;

                let custom_id = parsed["custom_id"].as_str().unwrap_or("").to_string();
                if custom_id.is_empty() {
                    continue;
                }

                let status_code = parsed["response"]["status_code"].as_u64().unwrap_or(0);
                if status_code >= 400 {
                    if let Some(logger) = crate::get_logger() {
                        logger.log(
                            "warn",
                            "memory",
                            "embedding::batch_api",
                            &format!("Batch item {} failed: status={}", custom_id, status_code),
                            None,
                            None,
                            None,
                        );
                    }
                    continue;
                }

                if let Some(data) = parsed["response"]["body"]["data"].as_array() {
                    if let Some(first) = data.first() {
                        if let Some(emb_arr) = first["embedding"].as_array() {
                            let mut emb: Vec<f32> = emb_arr
                                .iter()
                                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                                .collect();
                            l2_normalize(&mut emb);
                            all_results.insert(custom_id, emb);
                        }
                    }
                }
            }
        }

        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info",
                "memory",
                "embedding::batch_api",
                &format!(
                    "Batch API completed: {}/{} embeddings generated",
                    all_results.len(),
                    items.len()
                ),
                None,
                None,
                None,
            );
        }

        Ok(all_results)
    }
}

impl EmbeddingProvider for ApiEmbeddingProvider {
    fn embed(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>> {
        let results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(&[text.to_string()], purpose)?,
            _ => self.call_openai_compatible(&[text.to_string()], purpose)?,
        };
        let mut vec = results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[String], purpose: EmbeddingPurpose) -> Result<Vec<Vec<f32>>> {
        let mut results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(texts, purpose)?,
            _ => self.call_openai_compatible(texts, purpose)?,
        };
        for vec in &mut results {
            l2_normalize(vec);
        }
        Ok(results)
    }

    fn dimensions(&self) -> u32 {
        self.dimensions
    }

    fn supports_multimodal(&self) -> bool {
        matches!(self.provider_type, EmbeddingProviderType::Google)
            && self.model.contains("embedding-2")
    }

    fn embed_multimodal(
        &self,
        input: &MultimodalInput,
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<f32>> {
        if !self.supports_multimodal() {
            return self.embed(&input.label, purpose);
        }
        let mut vec = self.call_google_multimodal(input, purpose)?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn supports_batch_api(&self) -> bool {
        self.batch_api_supported()
    }

    fn embed_batch_async(
        &self,
        texts: &[(String, String)],
        purpose: EmbeddingPurpose,
    ) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        if !self.batch_api_supported() {
            // Fallback to synchronous
            let text_strs: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
            let results = self.embed_batch(&text_strs, purpose)?;
            let mut map = std::collections::HashMap::new();
            for ((id, _), emb) in texts.iter().zip(results) {
                map.insert(id.clone(), emb);
            }
            return Ok(map);
        }
        self.run_batch_api(texts, purpose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(
        base_url: &str,
        model: &str,
        provider_type: EmbeddingProviderType,
    ) -> ApiEmbeddingProvider {
        ApiEmbeddingProvider::new(&EmbeddingConfig {
            enabled: true,
            provider_type,
            api_base_url: Some(base_url.to_string()),
            api_key: None,
            api_model: Some(model.to_string()),
            api_dimensions: Some(768),
        })
        .unwrap()
    }

    #[test]
    fn embedding_endpoint_metadata_removes_credentials_and_query() {
        let endpoint = sanitized_api_endpoint(
            "https://user:secret@example.com/v1/embeddings?api_key=SYNTHETIC_CANARY#fragment",
        );

        assert_eq!(endpoint, "https://example.com/v1/embeddings");
        assert!(!endpoint.contains("secret"));
        assert!(!endpoint.contains("SYNTHETIC_CANARY"));
    }

    #[test]
    fn embedding_log_canary_rejects_payload_fields() {
        let source = include_str!("api_provider.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source before test module");
        for forbidden in [
            "\"request_body\"",
            "\"response_body\"",
            "\"text_preview\"",
            "label={}",
        ] {
            assert!(
                !production.contains(forbidden),
                "embedding logs must not expose payload field {forbidden}"
            );
        }
    }

    #[test]
    fn embedding_http_errors_are_stable_and_body_free() {
        let error = http_error("canary", reqwest::StatusCode::TOO_MANY_REQUESTS).to_string();
        assert_eq!(
            error,
            "Embedding provider HTTP error: operation=canary, status=429, class=rate_limited"
        );
        assert!(!error.contains("SYNTHETIC_CANARY"));
    }

    #[test]
    fn asymmetric_openai_compatible_roles_are_explicit_not_cardinality_based() {
        let voyage = provider(
            "https://api.voyageai.com",
            "voyage-4",
            EmbeddingProviderType::OpenaiCompatible,
        );
        let mut one_document = serde_json::json!({"input": ["one"]});
        voyage.apply_openai_compatible_purpose(&mut one_document, EmbeddingPurpose::Document);
        assert_eq!(one_document["input_type"], "document");
        let mut many_queries = serde_json::json!({"input": ["one", "two"]});
        voyage.apply_openai_compatible_purpose(&mut many_queries, EmbeddingPurpose::Query);
        assert_eq!(many_queries["input_type"], "query");

        let jina = provider(
            "https://api.jina.ai",
            "jina-embeddings-v5-text-small",
            EmbeddingProviderType::OpenaiCompatible,
        );
        let mut symmetric = serde_json::json!({});
        jina.apply_openai_compatible_purpose(&mut symmetric, EmbeddingPurpose::Symmetric);
        assert_eq!(symmetric["task"], "text-matching");

        let cohere = provider(
            "https://api.cohere.ai/compatibility",
            "embed-v4.0",
            EmbeddingProviderType::OpenaiCompatible,
        );
        let mut document = serde_json::json!({});
        cohere.apply_openai_compatible_purpose(&mut document, EmbeddingPurpose::Document);
        assert_eq!(document["input_type"], "search_document");
    }

    #[test]
    fn google_role_contract_switches_between_v1_task_type_and_v2_prefixes() {
        let v1 = provider(
            "https://generativelanguage.googleapis.com",
            "gemini-embedding-001",
            EmbeddingProviderType::Google,
        );
        let mut v1_request = serde_json::json!({});
        v1.apply_google_task_type(&mut v1_request, EmbeddingPurpose::Query);
        assert_eq!(v1_request["taskType"], "RETRIEVAL_QUERY");
        assert_eq!(
            v1.prepare_google_text("needle", EmbeddingPurpose::Query),
            "needle"
        );

        let v2 = provider(
            "https://generativelanguage.googleapis.com",
            "gemini-embedding-2",
            EmbeddingProviderType::Google,
        );
        let mut v2_request = serde_json::json!({});
        v2.apply_google_task_type(&mut v2_request, EmbeddingPurpose::Document);
        assert!(v2_request.get("taskType").is_none());
        assert_eq!(
            v2.prepare_google_text("needle", EmbeddingPurpose::Document),
            "title: none | text: needle"
        );
    }
}
