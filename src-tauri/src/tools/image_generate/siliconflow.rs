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

const DEFAULT_BASE_URL: &str = "https://api.siliconflow.cn";
const DEFAULT_MODEL: &str = "Qwen/Qwen-Image";
const EDIT_MODEL: &str = "Qwen/Qwen-Image-Edit";

#[derive(Deserialize)]
struct SiliconFlowResponse {
    images: Option<Vec<SiliconFlowImage>>,
}

#[derive(Deserialize)]
struct SiliconFlowImage {
    url: Option<String>,
}

pub(crate) struct SiliconFlowProvider;

impl ImageGenProviderImpl for SiliconFlowProvider {
    fn id(&self) -> &str {
        "siliconflow"
    }

    fn display_name(&self) -> &str {
        "SiliconFlow"
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
                supports_size: true,
                supports_aspect_ratio: false,
                supports_resolution: false,
            },
            geometry: Some(ImageGenGeometry {
                sizes: vec![
                    "1024x1024", "1328x1328", "1664x928", "928x1664",
                    "1472x1140", "1140x1472", "1584x1056", "1056x1584",
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
    let url = format!("{}/v1/images/generations", base);

    let has_input_images = !params.input_images.is_empty();

    // Auto-switch to Qwen-Image-Edit model for editing
    let effective_model = if has_input_images {
        EDIT_MODEL
    } else {
        params.model
    };

    // Build request body
    let mut body = serde_json::json!({
        "model": effective_model,
        "prompt": params.prompt,
        "image_size": params.size,
        "batch_size": params.n,
    });

    // Add inference steps (different defaults for generate vs edit)
    let steps = if has_input_images { 20 } else { 50 };
    body.as_object_mut().unwrap().insert(
        "num_inference_steps".to_string(),
        serde_json::json!(steps),
    );

    // Add reference image for editing
    if has_input_images {
        let input = &params.input_images[0];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&input.data);
        let data_uri = format!("data:{};base64,{}", input.mime, b64);
        body.as_object_mut().unwrap().insert(
            "image".to_string(),
            serde_json::json!(data_uri),
        );
        body.as_object_mut().unwrap().insert(
            "guidance_scale".to_string(),
            serde_json::json!(7.5),
        );
    }

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
            "image_generate::siliconflow::request",
            &format!(
                "SiliconFlow image gen request: model={}, size={}, n={}, edit={}, url={}",
                effective_model, params.size, params.n, has_input_images, url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": effective_model,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "size": params.size,
                    "n": params.n,
                    "timeout_secs": params.timeout_secs,
                    "has_input_images": has_input_images,
                    "steps": steps,
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

    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() { "debug" } else { "error" },
            "tool",
            "image_generate::siliconflow::response",
            &format!(
                "SiliconFlow image gen response: status={}, ttfb={}ms",
                status.as_u16(), ttfb_ms
            ),
            Some(serde_json::json!({"status": status.as_u16(), "ttfb_ms": ttfb_ms}).to_string()),
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
                "image_generate::siliconflow::error",
                &format!("SiliconFlow error ({}): {}", status.as_u16(), crate::truncate_utf8(&body_text, 500)),
                Some(serde_json::json!({"status": status.as_u16(), "error_body": &body_text}).to_string()),
                None,
                None,
            );
        }
        let preview = if body_text.len() > 300 {
            format!("{}...", crate::truncate_utf8(&body_text, 300))
        } else {
            body_text
        };
        anyhow::bail!("SiliconFlow image generation failed ({}): {}", status, preview);
    }

    let response: SiliconFlowResponse = resp.json().await?;
    let items = response.images.unwrap_or_default();
    if items.is_empty() {
        anyhow::bail!("SiliconFlow returned no images");
    }

    // Download images from URLs (URLs expire after 1 hour)
    let mut images = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if let Some(img_url) = item.url {
            let dl_start = std::time::Instant::now();
            let img_resp = client
                .get(&img_url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to download SiliconFlow image: {}", e))?;

            if !img_resp.status().is_success() {
                anyhow::bail!("SiliconFlow image download failed ({})", img_resp.status());
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
                    "image_generate::siliconflow::download",
                    &format!("SiliconFlow image #{} downloaded: {} bytes, {}ms", i, data.len(), dl_ms),
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
        anyhow::bail!("SiliconFlow returned no downloadable images");
    }

    Ok(ImageGenResult { images, text: None })
}
