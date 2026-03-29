use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use super::{
    GeneratedImage, ImageGenCapabilities, ImageGenEditCapabilities, ImageGenGeometry,
    ImageGenModeCapabilities, ImageGenParams, ImageGenProviderImpl, ImageGenResult,
};

const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com";
const DEFAULT_MODEL: &str = "wanx-v1";
const EDIT_MODEL: &str = "wanx2.1-imageedit";
const TEXT2IMAGE_PATH: &str = "/api/v1/services/aigc/text2image/image-synthesis";
const IMAGE2IMAGE_PATH: &str = "/api/v1/services/aigc/image2image/image-synthesis";
const TASK_PATH: &str = "/api/v1/tasks";

// ── Response Types ──────────────────────────────────────────────

#[derive(Deserialize)]
struct TongyiSubmitResponse {
    output: Option<TongyiTaskOutput>,
}

#[derive(Deserialize)]
struct TongyiTaskResponse {
    output: Option<TongyiTaskOutput>,
}

#[derive(Deserialize)]
struct TongyiTaskOutput {
    task_id: Option<String>,
    task_status: Option<String>,
    results: Option<Vec<TongyiResult>>,
    message: Option<String>,
    code: Option<String>,
}

#[derive(Deserialize)]
struct TongyiResult {
    url: Option<String>,
}

pub(crate) struct TongyiProvider;

impl ImageGenProviderImpl for TongyiProvider {
    fn id(&self) -> &str {
        "tongyi"
    }

    fn display_name(&self) -> &str {
        "Tongyi Wanxiang"
    }

    fn default_model(&self) -> &str {
        DEFAULT_MODEL
    }

    fn capabilities(&self) -> ImageGenCapabilities {
        ImageGenCapabilities {
            generate: ImageGenModeCapabilities {
                max_count: 4,
                supports_size: true,
                supports_aspect_ratio: false,
                supports_resolution: false,
            },
            edit: ImageGenEditCapabilities {
                enabled: true,
                max_count: 1,
                max_input_images: 1,
                supports_size: false,
                supports_aspect_ratio: false,
                supports_resolution: false,
            },
            geometry: Some(ImageGenGeometry {
                sizes: vec![
                    "1024x1024", "720x1280", "1280x720",
                ],
                aspect_ratios: vec![],
                resolutions: vec![],
            }),
        }
    }

    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

/// Convert size format from "1024x1024" to "1024*1024" for Tongyi API.
fn convert_size_format(size: &str) -> String {
    size.replace('x', "*")
}

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let has_input_images = !params.input_images.is_empty();

    // Build request based on mode (generate vs edit)
    let (submit_url, request_body) = if has_input_images {
        // Image editing mode
        let url = format!("{}{}", base, IMAGE2IMAGE_PATH);
        let input = &params.input_images[0];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&input.data);
        let data_uri = format!("data:{};base64,{}", input.mime, b64);

        let body = serde_json::json!({
            "model": EDIT_MODEL,
            "input": {
                "function": "description_edit",
                "prompt": params.prompt,
                "base_image_url": data_uri
            },
            "parameters": {
                "n": 1
            }
        });
        (url, body)
    } else {
        // Text-to-image mode
        let url = format!("{}{}", base, TEXT2IMAGE_PATH);
        let tongyi_size = convert_size_format(params.size);

        let body = serde_json::json!({
            "model": params.model,
            "input": {
                "prompt": params.prompt
            },
            "parameters": {
                "size": tongyi_size,
                "n": params.n
            }
        });
        (url, body)
    };

    // Log request
    if let Some(logger) = crate::get_logger() {
        let prompt_preview = if params.prompt.len() > 500 {
            format!("{}...", crate::truncate_utf8(params.prompt, 500))
        } else {
            params.prompt.to_string()
        };
        logger.log(
            "debug",
            "tool",
            "image_generate::tongyi::request",
            &format!(
                "Tongyi image gen request: model={}, edit={}, url={}",
                if has_input_images { EDIT_MODEL } else { params.model },
                has_input_images,
                submit_url
            ),
            Some(
                serde_json::json!({
                    "api_url": &submit_url,
                    "model": if has_input_images { EDIT_MODEL } else { params.model },
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "size": params.size,
                    "n": params.n,
                    "timeout_secs": params.timeout_secs,
                    "has_input_images": has_input_images,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(60)),
    )
    .build()?;

    // Step A: Submit async task
    let request_start = std::time::Instant::now();
    let resp = client
        .post(&submit_url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .header("X-DashScope-Async", "enable")
        .json(&request_body)
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() { "debug" } else { "error" },
            "tool",
            "image_generate::tongyi::submit_response",
            &format!("Tongyi submit response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
            Some(serde_json::json!({"status": status.as_u16(), "ttfb_ms": ttfb_ms}).to_string()),
            None,
            None,
        );
    }

    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        let preview = if body_text.len() > 300 {
            format!("{}...", crate::truncate_utf8(&body_text, 300))
        } else {
            body_text
        };
        anyhow::bail!("Tongyi Wanxiang task submit failed ({}): {}", status, preview);
    }

    let submit_resp: TongyiSubmitResponse = resp.json().await?;
    let output = submit_resp.output.ok_or_else(|| anyhow::anyhow!("Tongyi Wanxiang: missing output in submit response"))?;
    let task_id = output.task_id.ok_or_else(|| anyhow::anyhow!("Tongyi Wanxiang: missing task_id in submit response"))?;

    app_info!(
        "tool",
        "image_generate",
        "Tongyi task submitted: task_id={}",
        task_id
    );

    // Step B: Poll for results
    let poll_url = format!("{}{}/{}", base, TASK_PATH, task_id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(params.timeout_secs);
    let mut poll_interval_ms: u64 = 1000; // Start at 1s, increase gradually

    loop {
        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "Tongyi Wanxiang task timed out after {}s (task_id={})",
                params.timeout_secs,
                task_id
            );
        }

        tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;

        let poll_resp = client
            .get(&poll_url)
            .header("Authorization", format!("Bearer {}", params.api_key))
            .send()
            .await?;

        if !poll_resp.status().is_success() {
            let body_text = poll_resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Tongyi Wanxiang task poll failed (task_id={}): {}",
                task_id,
                crate::truncate_utf8(&body_text, 300)
            );
        }

        let task_resp: TongyiTaskResponse = poll_resp.json().await?;
        let task_output = task_resp.output.ok_or_else(|| {
            anyhow::anyhow!("Tongyi Wanxiang: missing output in poll response")
        })?;

        let task_status = task_output.task_status.as_deref().unwrap_or("UNKNOWN");

        match task_status {
            "SUCCEEDED" => {
                let total_ms = request_start.elapsed().as_millis() as u64;
                app_info!(
                    "tool",
                    "image_generate",
                    "Tongyi task completed in {}ms (task_id={})",
                    total_ms,
                    task_id
                );

                let results = task_output.results.unwrap_or_default();
                if results.is_empty() {
                    anyhow::bail!("Tongyi Wanxiang task succeeded but returned no images");
                }

                // Download images from URLs
                let mut images = Vec::new();
                for (i, result) in results.into_iter().enumerate() {
                    if let Some(img_url) = result.url {
                        let dl_start = std::time::Instant::now();
                        let img_resp = client
                            .get(&img_url)
                            .timeout(std::time::Duration::from_secs(30))
                            .send()
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to download Tongyi image: {}", e)
                            })?;

                        if !img_resp.status().is_success() {
                            anyhow::bail!(
                                "Tongyi image download failed ({})",
                                img_resp.status()
                            );
                        }

                        let content_type = img_resp
                            .headers()
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("image/png")
                            .split(';')
                            .next()
                            .unwrap_or("image/png")
                            .trim()
                            .to_string();
                        let data = img_resp.bytes().await?.to_vec();
                        let dl_ms = dl_start.elapsed().as_millis() as u64;

                        if let Some(logger) = crate::get_logger() {
                            logger.log(
                                "debug",
                                "tool",
                                "image_generate::tongyi::download",
                                &format!(
                                    "Tongyi image #{} downloaded: {} bytes, {}ms",
                                    i, data.len(), dl_ms
                                ),
                                Some(
                                    serde_json::json!({
                                        "index": i,
                                        "size_bytes": data.len(),
                                        "download_ms": dl_ms,
                                    })
                                    .to_string(),
                                ),
                                None,
                                None,
                            );
                        }

                        images.push(GeneratedImage {
                            data,
                            mime: content_type,
                            revised_prompt: None,
                        });
                    }
                }

                if images.is_empty() {
                    anyhow::bail!("Tongyi Wanxiang: no downloadable images");
                }

                return Ok(ImageGenResult { images, text: None });
            }
            "FAILED" => {
                let msg = task_output.message.unwrap_or_default();
                let code = task_output.code.unwrap_or_default();
                anyhow::bail!(
                    "Tongyi Wanxiang task failed (task_id={}, code={}): {}",
                    task_id,
                    code,
                    msg
                );
            }
            "PENDING" | "RUNNING" => {
                // Increase poll interval gradually: 1s → 2s → 3s (max 3s)
                poll_interval_ms = (poll_interval_ms + 1000).min(3000);
            }
            _ => {
                app_warn!(
                    "tool",
                    "image_generate",
                    "Tongyi unknown task status: {} (task_id={})",
                    task_status,
                    task_id
                );
                poll_interval_ms = (poll_interval_ms + 1000).min(3000);
            }
        }
    }
}
