use anyhow::{Context, Result};

use super::config::{EmbeddingConfig, EmbeddingProviderType};
use super::utils::{l2_normalize, truncate_for_model};
use crate::memory::traits::{EmbeddingProvider, MultimodalInput};

// ── API Embedding Provider ───────────────────────────────────────

/// OpenAI-compatible /v1/embeddings API provider.
pub struct ApiEmbeddingProvider {
    client: reqwest::blocking::Client,
    base_url: String,
    api_key: String,
    model: String,
    dimensions: u32,
    provider_type: EmbeddingProviderType,
}

impl ApiEmbeddingProvider {
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

        let client = crate::provider::apply_proxy_blocking(
            reqwest::blocking::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30)),
        )
        .build()
        .context("Failed to build embedding HTTP client")?;

        Ok(Self {
            client,
            base_url,
            api_key,
            model,
            dimensions,
            provider_type: config.provider_type.clone(),
        })
    }

    fn call_openai_compatible(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
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

        // Voyage AI asymmetric embedding: query (single text search) vs document (batch indexing)
        if self.base_url.contains("voyageai.com") {
            body["input_type"] = serde_json::json!(if texts.len() == 1 {
                "query"
            } else {
                "document"
            });
        }

        // Log embedding API request
        if let Some(logger) = crate::get_logger() {
            let body_str = serde_json::to_string(&body).unwrap_or_default();
            let body_size = body_str.len();
            let body_preview = if body_size > 4096 {
                format!(
                    "{}...(truncated, total {}B)",
                    crate::truncate_utf8(&body_str, 4096),
                    body_size
                )
            } else {
                body_str
            };
            logger.log(
                "debug",
                "memory",
                "embedding::openai_compatible::request",
                &format!(
                    "Embedding API request: {} texts, model={}, url={}, body {}B",
                    texts.len(),
                    self.model,
                    url,
                    body_size
                ),
                Some(
                    serde_json::json!({
                        "api_url": &url,
                        "model": &self.model,
                        "text_count": texts.len(),
                        "dimensions": self.dimensions,
                        "body_size_bytes": body_size,
                        "request_body": body_preview,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        // Log embedding API response
        if let Some(logger) = crate::get_logger() {
            let resp_preview = if resp_text.len() > 2048 {
                format!(
                    "{}...(truncated, total {}B)",
                    crate::truncate_utf8(&resp_text, 2048),
                    resp_text.len()
                )
            } else {
                resp_text.clone()
            };
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
                        "response_body": resp_preview,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !status.is_success() {
            anyhow::bail!("Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
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

        Ok(results)
    }

    /// Batch embed via Google Gemini `batchEmbedContents` API (up to 100 texts per request).
    /// Falls back to single `embedContent` if batch fails.
    fn call_google(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let texts = truncate_for_model(texts, &self.model);
        const BATCH_SIZE: usize = 100; // Gemini batch limit

        let mut all_results = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(BATCH_SIZE) {
            match self.call_google_batch(chunk) {
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
                        let result = self.call_google_single(text)?;
                        all_results.push(result);
                    }
                }
            }
        }

        Ok(all_results)
    }

    /// Batch embed via `batchEmbedContents` endpoint.
    fn call_google_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
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
                req
            })
            .collect();

        let body = serde_json::json!({ "requests": requests });

        // Log batch request
        if let Some(logger) = crate::get_logger() {
            let safe_url = format!(
                "{}/v1beta/models/{}:batchEmbedContents?key=[REDACTED]",
                self.base_url.trim_end_matches('/'),
                self.model,
            );
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
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call Google batch embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        // Log batch response
        if let Some(logger) = crate::get_logger() {
            let resp_preview = if resp_text.len() > 2048 {
                format!(
                    "{}...(truncated, total {}B)",
                    crate::truncate_utf8(&resp_text, 2048),
                    resp_text.len()
                )
            } else {
                resp_text.clone()
            };
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
                        "response_body": resp_preview,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !status.is_success() {
            anyhow::bail!("Google Batch Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
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

        Ok(results)
    }

    /// Single text embed via `embedContent` endpoint (fallback).
    fn call_google_single(&self, text: &str) -> Result<Vec<f32>> {
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

        if let Some(logger) = crate::get_logger() {
            let text_preview = if text.len() > 200 {
                format!("{}...", crate::truncate_utf8(text, 200))
            } else {
                text.to_string()
            };
            let safe_url = format!(
                "{}/v1beta/models/{}:embedContent?key=[REDACTED]",
                self.base_url.trim_end_matches('/'),
                self.model,
            );
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
                        "text_preview": text_preview,
                        "dimensions": self.dimensions,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call Google embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

        if let Some(logger) = crate::get_logger() {
            let resp_preview = if resp_text.len() > 2048 {
                format!(
                    "{}...(truncated, total {}B)",
                    crate::truncate_utf8(&resp_text, 2048),
                    resp_text.len()
                )
            } else {
                resp_text.clone()
            };
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
                        "response_body": resp_preview,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !status.is_success() {
            anyhow::bail!("Google Embedding API error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let values = resp_json["embedding"]["values"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google embedding response"))?;

        Ok(values
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect())
    }

    /// Multimodal embed via Gemini `embedContent` with inline data (image/audio).
    /// Only works with gemini-embedding-2.
    fn call_google_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        use base64::Engine;

        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        );

        let b64_data = base64::engine::general_purpose::STANDARD.encode(&input.file_data);

        let mut body = serde_json::json!({
            "content": {
                "parts": [
                    { "text": &input.label },
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
            let safe_url = format!(
                "{}/v1beta/models/{}:embedContent?key=[REDACTED]",
                self.base_url.trim_end_matches('/'),
                self.model,
            );
            logger.log(
                "info",
                "memory",
                "embedding::google::multimodal_request",
                &format!(
                    "Multimodal embedding: model={}, mime={}, file_size={}B, label={}",
                    self.model,
                    input.mime_type,
                    input.file_data.len(),
                    crate::truncate_utf8(&input.label, 100)
                ),
                Some(
                    serde_json::json!({
                        "api_url": safe_url,
                        "model": &self.model,
                        "mime_type": &input.mime_type,
                        "file_size_bytes": input.file_data.len(),
                        "base64_size_bytes": b64_data.len(),
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let request_start = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to call Google multimodal embedding API")?;

        let status = resp.status();
        let ttfb_ms = request_start.elapsed().as_millis() as u64;
        let resp_text = resp.text()?;

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
            anyhow::bail!(
                "Google Multimodal Embedding API error {}: {}",
                status,
                resp_text
            );
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        let values = resp_json["embedding"]["values"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid Google multimodal embedding response"))?;

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

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(body)
            .send()
            .context("Failed to upload batch JSONL file")?;

        let status = resp.status();
        let resp_text = resp.text()?;
        if !status.is_success() {
            anyhow::bail!("Batch file upload error {}: {}", status, resp_text);
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
        resp_json["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Missing file id in upload response"))
    }

    /// Create a batch job.
    fn batch_create(&self, input_file_id: &str) -> Result<String> {
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
                "input_type": "document",
            });
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to create batch job")?;

        let status = resp.status();
        let resp_text = resp.text()?;
        if !status.is_success() {
            anyhow::bail!("Batch create error {}: {}", status, resp_text);
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
            let resp = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .send()
                .context("Failed to poll batch status")?;

            let resp_text = resp.text()?;
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
                    anyhow::bail!(
                        "Batch {} {}: {}",
                        batch_id,
                        state,
                        resp_json["error"].as_str().unwrap_or("unknown error")
                    );
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

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .context("Failed to download batch output")?;

        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            anyhow::bail!("Batch output download error {}: {}", status, text);
        }
        Ok(text)
    }

    /// Run the complete async Batch API flow: upload JSONL -> create batch -> poll -> download -> parse.
    fn run_batch_api(
        &self,
        items: &[(String, String)],
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

            let batch_id = self.batch_create(&file_id)?;
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
                let parsed: serde_json::Value = serde_json::from_str(line).with_context(|| {
                    format!(
                        "Invalid batch output line: {}",
                        crate::truncate_utf8(line, 200)
                    )
                })?;

                let custom_id = parsed["custom_id"].as_str().unwrap_or("").to_string();
                if custom_id.is_empty() {
                    continue;
                }

                let status_code = parsed["response"]["status_code"].as_u64().unwrap_or(0);
                if status_code >= 400 {
                    let err_msg = parsed["response"]["body"]["error"]["message"]
                        .as_str()
                        .unwrap_or("unknown error");
                    if let Some(logger) = crate::get_logger() {
                        logger.log(
                            "warn",
                            "memory",
                            "embedding::batch_api",
                            &format!("Batch item {} failed: {}", custom_id, err_msg),
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
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(&[text.to_string()])?,
            _ => self.call_openai_compatible(&[text.to_string()])?,
        };
        let mut vec = results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = match self.provider_type {
            EmbeddingProviderType::Google => self.call_google(texts)?,
            _ => self.call_openai_compatible(texts)?,
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

    fn embed_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        if !self.supports_multimodal() {
            return self.embed(&input.label);
        }
        let mut vec = self.call_google_multimodal(input)?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn supports_batch_api(&self) -> bool {
        self.batch_api_supported()
    }

    fn embed_batch_async(
        &self,
        texts: &[(String, String)],
    ) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        if !self.batch_api_supported() {
            // Fallback to synchronous
            let text_strs: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
            let results = self.embed_batch(&text_strs)?;
            let mut map = std::collections::HashMap::new();
            for ((id, _), emb) in texts.iter().zip(results) {
                map.insert(id.clone(), emb);
            }
            return Ok(map);
        }
        self.run_batch_api(texts)
    }
}
