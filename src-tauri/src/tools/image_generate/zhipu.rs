use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

use super::{
    GeneratedImage, ImageGenCapabilities, ImageGenEditCapabilities, ImageGenGeometry,
    ImageGenModeCapabilities, ImageGenParams, ImageGenProviderImpl, ImageGenResult,
};

const DEFAULT_BASE_URL: &str = "https://open.bigmodel.cn/api/paas";
const DEFAULT_MODEL: &str = "cogView-4-250304";

#[derive(Deserialize)]
struct ZhipuResponse {
    data: Option<Vec<ZhipuImageData>>,
}

#[derive(Deserialize)]
struct ZhipuImageData {
    url: Option<String>,
}

pub(crate) struct ZhipuProvider;

impl ImageGenProviderImpl for ZhipuProvider {
    fn id(&self) -> &str {
        "zhipu"
    }

    fn display_name(&self) -> &str {
        "ZhipuAI"
    }

    fn default_model(&self) -> &str {
        DEFAULT_MODEL
    }

    fn capabilities(&self) -> ImageGenCapabilities {
        ImageGenCapabilities {
            generate: ImageGenModeCapabilities {
                max_count: 1,
                supports_size: true,
                supports_aspect_ratio: false,
                supports_resolution: false,
            },
            edit: ImageGenEditCapabilities {
                enabled: false,
                max_count: 0,
                max_input_images: 0,
                supports_size: false,
                supports_aspect_ratio: false,
                supports_resolution: false,
            },
            geometry: Some(ImageGenGeometry {
                sizes: vec![
                    "1024x1024", "1024x1536", "1536x1024",
                    "1024x1792", "1792x1024", "2048x2048",
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

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}/v4/images/generations", base);

    let request_body = serde_json::json!({
        "model": params.model,
        "prompt": params.prompt,
        "size": params.size,
    });

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
            "image_generate::zhipu::request",
            &format!(
                "ZhipuAI image gen request: model={}, size={}, url={}",
                params.model, params.size, url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": params.model,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "size": params.size,
                    "timeout_secs": params.timeout_secs,
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
            .timeout(std::time::Duration::from_secs(params.timeout_secs)),
    )
    .build()?;
    let request_start = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() { "debug" } else { "error" },
            "tool",
            "image_generate::zhipu::response",
            &format!("ZhipuAI response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
            Some(serde_json::json!({"status": status.as_u16(), "ttfb_ms": ttfb_ms}).to_string()),
            None,
            None,
        );
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                "image_generate::zhipu::error",
                &format!("ZhipuAI error ({}): {}", status.as_u16(), crate::truncate_utf8(&body, 500)),
                Some(serde_json::json!({"status": status.as_u16(), "error_body": &body}).to_string()),
                None,
                None,
            );
        }
        let preview = if body.len() > 300 {
            format!("{}...", crate::truncate_utf8(&body, 300))
        } else {
            body
        };
        anyhow::bail!("ZhipuAI image generation failed ({}): {}", status, preview);
    }

    let body: ZhipuResponse = resp.json().await?;
    let items = body.data.unwrap_or_default();
    if items.is_empty() {
        anyhow::bail!("ZhipuAI returned no images");
    }

    // Download images from URLs
    let mut images = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if let Some(img_url) = item.url {
            let dl_start = std::time::Instant::now();
            let img_resp = client
                .get(&img_url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to download ZhipuAI image: {}", e))?;

            if !img_resp.status().is_success() {
                anyhow::bail!("ZhipuAI image download failed ({})", img_resp.status());
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
                    "image_generate::zhipu::download",
                    &format!("ZhipuAI image #{} downloaded: {} bytes, {}ms", i, data.len(), dl_ms),
                    Some(serde_json::json!({"index": i, "size_bytes": data.len(), "download_ms": dl_ms}).to_string()),
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
        anyhow::bail!("ZhipuAI returned no downloadable images");
    }

    Ok(ImageGenResult { images, text: None })
}
