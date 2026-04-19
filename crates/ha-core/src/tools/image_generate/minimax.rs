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

const DEFAULT_BASE_URL: &str = "https://api.minimax.io";
const DEFAULT_MODEL: &str = "image-01";

#[derive(Deserialize)]
struct MiniMaxImageResponse {
    data: Option<MiniMaxImageData>,
    metadata: Option<MiniMaxMetadata>,
    base_resp: Option<MiniMaxBaseResp>,
}

#[derive(Deserialize)]
struct MiniMaxImageData {
    image_base64: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct MiniMaxMetadata {
    #[allow(dead_code)]
    success_count: Option<u32>,
    failed_count: Option<u32>,
}

#[derive(Deserialize)]
struct MiniMaxBaseResp {
    status_code: Option<i32>,
    status_msg: Option<String>,
}

pub(crate) struct MiniMaxProvider;

impl ImageGenProviderImpl for MiniMaxProvider {
    fn id(&self) -> &str {
        "minimax"
    }

    fn display_name(&self) -> &str {
        "MiniMax"
    }

    fn default_model(&self) -> &str {
        DEFAULT_MODEL
    }

    fn capabilities(&self) -> ImageGenCapabilities {
        ImageGenCapabilities {
            generate: ImageGenModeCapabilities {
                max_count: 9,
                supports_size: false,
                supports_aspect_ratio: true,
                supports_resolution: false,
            },
            edit: ImageGenEditCapabilities {
                enabled: true,
                max_count: 9,
                max_input_images: 1,
                supports_size: false,
                supports_aspect_ratio: true,
                supports_resolution: false,
            },
            geometry: Some(ImageGenGeometry {
                sizes: vec![],
                aspect_ratios: vec!["1:1", "16:9", "4:3", "3:2", "2:3", "3:4", "9:16", "21:9"],
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
        .map(|s| {
            // Extract origin from base URL (which may include path like /anthropic)
            if let Ok(url) = url::Url::parse(s) {
                format!("{}://{}", url.scheme(), url.host_str().unwrap_or(s))
            } else {
                s.trim_end_matches('/').to_string()
            }
        })
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let url = format!("{}/v1/image_generation", base);

    // Build request body
    let mut body = serde_json::json!({
        "model": params.model,
        "prompt": params.prompt,
        "response_format": "base64",
        "n": params.n,
    });

    // Add aspect_ratio if specified
    if let Some(ar) = params.aspect_ratio {
        body.as_object_mut()
            .unwrap()
            .insert("aspect_ratio".to_string(), serde_json::json!(ar));
    }

    // Add reference image as subject_reference for editing
    if !params.input_images.is_empty() {
        let input = &params.input_images[0];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&input.data);
        let data_uri = format!("data:{};base64,{}", input.mime, b64);
        body.as_object_mut().unwrap().insert(
            "subject_reference".to_string(),
            serde_json::json!([{
                "type": "character",
                "image_file": data_uri
            }]),
        );
    }

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
            "image_generate::minimax::request",
            &format!(
                "MiniMax image gen request: model={}, n={}, edit={}, url={}",
                params.model,
                params.n,
                !params.input_images.is_empty(),
                url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": params.model,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "n": params.n,
                    "timeout_secs": params.timeout_secs,
                    "has_input_images": !params.input_images.is_empty(),
                    "aspect_ratio": params.aspect_ratio,
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
        .json(&body)
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
            "image_generate::minimax::response",
            &format!(
                "MiniMax image gen response: status={}, ttfb={}ms",
                status.as_u16(),
                ttfb_ms
            ),
            Some(
                serde_json::json!({
                    "status": status.as_u16(),
                    "ttfb_ms": ttfb_ms,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                "image_generate::minimax::error",
                &format!(
                    "MiniMax image gen error ({}): {}",
                    status.as_u16(),
                    crate::truncate_utf8(&body_text, 500)
                ),
                Some(
                    serde_json::json!({
                        "status": status.as_u16(),
                        "error_body": &body_text,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }
        let preview = if body_text.len() > 300 {
            format!("{}...", crate::truncate_utf8(&body_text, 300))
        } else {
            body_text
        };
        anyhow::bail!("MiniMax image generation failed ({}): {}", status, preview);
    }

    let response: MiniMaxImageResponse = resp.json().await?;

    // Check API-level error
    if let Some(ref base_resp) = response.base_resp {
        if let Some(code) = base_resp.status_code {
            if code != 0 {
                let msg = base_resp.status_msg.as_deref().unwrap_or("");
                anyhow::bail!("MiniMax image generation API error ({}): {}", code, msg);
            }
        }
    }

    let base64_images = response
        .data
        .and_then(|d| d.image_base64)
        .unwrap_or_default();

    if base64_images.is_empty() {
        let failed_count = response.metadata.and_then(|m| m.failed_count).unwrap_or(0);
        let reason = if failed_count > 0 {
            format!("{} image(s) failed to generate", failed_count)
        } else {
            "no images returned".to_string()
        };
        anyhow::bail!("MiniMax image generation returned no images: {}", reason);
    }

    let mut images = Vec::new();
    for b64 in base64_images.iter() {
        if b64.is_empty() {
            continue;
        }
        let data = base64::engine::general_purpose::STANDARD.decode(b64)?;
        images.push(GeneratedImage {
            data,
            mime: "image/png".to_string(),
            revised_prompt: None,
        });
    }

    if images.is_empty() {
        anyhow::bail!("MiniMax returned no valid image data");
    }

    // Log successful result
    if let Some(logger) = crate::get_logger() {
        let image_sizes: Vec<usize> = images.iter().map(|img| img.data.len()).collect();
        logger.log(
            "debug",
            "tool",
            "image_generate::minimax::result",
            &format!(
                "MiniMax image gen result: {} image(s), sizes={:?}",
                images.len(),
                image_sizes
            ),
            Some(
                serde_json::json!({
                    "image_count": images.len(),
                    "image_sizes_bytes": image_sizes,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok(ImageGenResult { images, text: None })
}
