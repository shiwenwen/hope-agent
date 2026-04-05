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

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_MODEL: &str = "gpt-image-1";

#[derive(Deserialize)]
struct OpenAIImageResponse {
    data: Option<Vec<OpenAIImageData>>,
}

#[derive(Deserialize)]
struct OpenAIImageData {
    b64_json: Option<String>,
    revised_prompt: Option<String>,
}

pub(crate) struct OpenAIProvider;

impl ImageGenProviderImpl for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn display_name(&self) -> &str {
        "OpenAI"
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
                enabled: false,
                max_count: 0,
                max_input_images: 0,
                supports_size: false,
                supports_aspect_ratio: false,
                supports_resolution: false,
            },
            geometry: Some(ImageGenGeometry {
                sizes: vec!["1024x1024", "1024x1536", "1536x1024"],
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
    let url = format!("{}/v1/images/generations", base);

    let request_body = serde_json::json!({
        "model": params.model,
        "prompt": params.prompt,
        "n": params.n,
        "size": params.size,
        "response_format": "b64_json",
    });

    // Log image generation request
    if let Some(logger) = crate::get_logger() {
        let prompt_preview = if params.prompt.len() > 500 {
            format!("{}...", crate::truncate_utf8(params.prompt, 500))
        } else {
            params.prompt.to_string()
        };
        logger.log(
            "debug",
            "tool",
            "image_generate::openai::request",
            &format!(
                "OpenAI image gen request: model={}, size={}, n={}, url={}",
                params.model, params.size, params.n, url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": params.model,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "size": params.size,
                    "n": params.n,
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

    // Log response status
    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() {
                "debug"
            } else {
                "error"
            },
            "tool",
            "image_generate::openai::response",
            &format!(
                "OpenAI image gen response: status={}, ttfb={}ms",
                status.as_u16(),
                ttfb_ms
            ),
            Some(
                serde_json::json!({
                    "status": status.as_u16(),
                    "ttfb_ms": ttfb_ms,
                    "request_id": resp.headers().get("x-request-id").and_then(|v| v.to_str().ok()),
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // Log full error response
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                "image_generate::openai::error",
                &format!(
                    "OpenAI image gen error ({}): {}",
                    status.as_u16(),
                    crate::truncate_utf8(&body, 500)
                ),
                Some(
                    serde_json::json!({
                        "status": status.as_u16(),
                        "error_body": &body,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }
        let preview = if body.len() > 300 {
            format!("{}...", crate::truncate_utf8(&body, 300))
        } else {
            body
        };
        anyhow::bail!("OpenAI image generation failed ({}): {}", status, preview);
    }

    let body: OpenAIImageResponse = resp.json().await?;
    let items = body.data.unwrap_or_default();
    if items.is_empty() {
        anyhow::bail!("OpenAI returned no images");
    }

    let mut images = Vec::new();
    let mut revised_prompts: Vec<String> = Vec::new();
    for item in items {
        if let Some(ref rp) = item.revised_prompt {
            revised_prompts.push(rp.clone());
        }
        if let Some(b64) = item.b64_json {
            let data = base64::engine::general_purpose::STANDARD.decode(&b64)?;
            images.push(GeneratedImage {
                data,
                mime: "image/png".to_string(),
                revised_prompt: item.revised_prompt,
            });
        }
    }

    if images.is_empty() {
        anyhow::bail!("OpenAI returned no valid image data");
    }

    // Log successful result details (everything except raw image bytes)
    if let Some(logger) = crate::get_logger() {
        let image_sizes: Vec<usize> = images.iter().map(|img| img.data.len()).collect();
        logger.log(
            "debug",
            "tool",
            "image_generate::openai::result",
            &format!(
                "OpenAI image gen result: {} image(s), sizes={:?}",
                images.len(),
                image_sizes
            ),
            Some(
                serde_json::json!({
                    "image_count": images.len(),
                    "image_sizes_bytes": image_sizes,
                    "mime_types": images.iter().map(|img| &img.mime).collect::<Vec<_>>(),
                    "revised_prompts": revised_prompts,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok(ImageGenResult { images, text: None })
}
