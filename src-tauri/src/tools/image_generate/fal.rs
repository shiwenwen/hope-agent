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

const DEFAULT_BASE_URL: &str = "https://fal.run";
const DEFAULT_MODEL: &str = "fal-ai/flux/dev";
const EDIT_SUBPATH: &str = "image-to-image";

#[derive(Deserialize)]
struct FalResponse {
    images: Option<Vec<FalImage>>,
}

#[derive(Deserialize)]
struct FalImage {
    url: Option<String>,
    content_type: Option<String>,
}

/// Parse size string "1024x1024" into (width, height).
fn parse_size(size: &str) -> (u32, u32) {
    let parts: Vec<&str> = size.split('x').collect();
    if parts.len() == 2 {
        let w = parts[0].parse().unwrap_or(1024);
        let h = parts[1].parse().unwrap_or(1024);
        (w, h)
    } else {
        (1024, 1024)
    }
}

/// Map aspect ratio to Fal enum string.
fn aspect_ratio_to_fal_enum(ar: &str) -> Option<&'static str> {
    match ar {
        "1:1" => Some("square_hd"),
        "4:3" => Some("landscape_4_3"),
        "3:4" => Some("portrait_4_3"),
        "16:9" => Some("landscape_16_9"),
        "9:16" => Some("portrait_16_9"),
        _ => None,
    }
}

/// Convert aspect ratio + resolution edge to width/height dimensions.
fn aspect_ratio_to_dimensions(ar: &str, edge: u32) -> Option<(u32, u32)> {
    let parts: Vec<&str> = ar.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let w_ratio: u32 = parts[0].parse().ok()?;
    let h_ratio: u32 = parts[1].parse().ok()?;
    if w_ratio == 0 || h_ratio == 0 {
        return None;
    }

    if w_ratio >= h_ratio {
        Some((edge, (edge * h_ratio).div_ceil(w_ratio)))
    } else {
        Some(((edge * w_ratio).div_ceil(h_ratio), edge))
    }
}

/// Map resolution string to edge pixel count.
fn resolution_to_edge(res: &str) -> u32 {
    match res {
        "4K" => 4096,
        "2K" => 2048,
        _ => 1024,
    }
}

/// Resolve the effective image_size for the Fal API request.
fn resolve_fal_image_size(
    size: &str,
    aspect_ratio: Option<&str>,
    resolution: Option<&str>,
    has_input_images: bool,
) -> serde_json::Value {
    // Explicit size takes precedence
    let (w, h) = parse_size(size);
    let is_default_size = w == 1024 && h == 1024;

    // If explicit non-default size, use it directly
    if !is_default_size {
        return serde_json::json!({ "width": w, "height": h });
    }

    // aspectRatio + resolution → calculate dimensions
    if let Some(ar) = aspect_ratio {
        if has_input_images {
            // Fal edit mode doesn't support aspectRatio, skip
            let edge = resolution.map(resolution_to_edge).unwrap_or(1024);
            return serde_json::json!({ "width": edge, "height": edge });
        }
        let edge = resolution.map(resolution_to_edge).unwrap_or(1024);
        if let Some((w, h)) = aspect_ratio_to_dimensions(ar, edge) {
            return serde_json::json!({ "width": w, "height": h });
        }
        // Fallback to enum
        if let Some(fal_enum) = aspect_ratio_to_fal_enum(ar) {
            return serde_json::json!(fal_enum);
        }
    }

    // Resolution only → square at that resolution
    if let Some(res) = resolution {
        let edge = resolution_to_edge(res);
        return serde_json::json!({ "width": edge, "height": edge });
    }

    // Default
    serde_json::json!({ "width": w, "height": h })
}

pub(crate) struct FalProvider;

impl ImageGenProviderImpl for FalProvider {
    fn id(&self) -> &str {
        "fal"
    }

    fn display_name(&self) -> &str {
        "Fal"
    }

    fn default_model(&self) -> &str {
        DEFAULT_MODEL
    }

    fn capabilities(&self) -> ImageGenCapabilities {
        ImageGenCapabilities {
            generate: ImageGenModeCapabilities {
                max_count: 4,
                supports_size: true,
                supports_aspect_ratio: true,
                supports_resolution: true,
            },
            edit: ImageGenEditCapabilities {
                enabled: true,
                max_count: 4,
                max_input_images: 1,
                supports_size: true,
                supports_aspect_ratio: false, // Fal edit doesn't support aspectRatio
                supports_resolution: true,
            },
            geometry: Some(ImageGenGeometry {
                sizes: vec![
                    "1024x1024", "1024x1536", "1536x1024", "1024x1792", "1792x1024",
                ],
                aspect_ratios: vec!["1:1", "4:3", "3:4", "16:9", "9:16"],
                resolutions: vec!["1K", "2K", "4K"],
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

    let has_input_images = !params.input_images.is_empty();

    // Auto-append /image-to-image for edit mode
    let model_path = if has_input_images {
        let m = params.model;
        if m.ends_with(&format!("/{}", EDIT_SUBPATH))
            || m.contains("/image-to-image/")
            || m.ends_with("/edit")
        {
            m.to_string()
        } else {
            format!("{}/{}", m, EDIT_SUBPATH)
        }
    } else {
        params.model.to_string()
    };

    let url = format!("{}/{}", base, model_path);

    let image_size = resolve_fal_image_size(
        params.size,
        params.aspect_ratio,
        params.resolution,
        has_input_images,
    );

    // Build request body
    let mut request_body = serde_json::json!({
        "prompt": params.prompt,
        "num_images": params.n,
        "output_format": "png",
        "image_size": image_size,
    });

    // Add reference image for edit mode
    if has_input_images {
        let input = &params.input_images[0];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&input.data);
        let data_uri = format!("data:{};base64,{}", input.mime, b64);
        request_body.as_object_mut().unwrap().insert(
            "image_url".to_string(),
            serde_json::json!(data_uri),
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
            "image_generate::fal::request",
            &format!(
                "Fal image gen request: model={}, n={}, edit={}, url={}",
                model_path, params.n, has_input_images, url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": &model_path,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "size": params.size,
                    "image_size": &image_size,
                    "n": params.n,
                    "timeout_secs": params.timeout_secs,
                    "has_input_images": has_input_images,
                    "aspect_ratio": params.aspect_ratio,
                    "resolution": params.resolution,
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
        .header("Authorization", format!("Key {}", params.api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    // Log response status
    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() { "debug" } else { "error" },
            "tool",
            "image_generate::fal::response",
            &format!(
                "Fal image gen response: status={}, ttfb={}ms",
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
        let body = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                "image_generate::fal::error",
                &format!(
                    "Fal image gen error ({}): {}",
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
        anyhow::bail!("Fal image generation failed ({}): {}", status, preview);
    }

    let body: FalResponse = resp.json().await?;
    let items = body.images.unwrap_or_default();
    if items.is_empty() {
        anyhow::bail!("Fal returned no images");
    }

    // Log API response metadata (image URLs and content types)
    if let Some(logger) = crate::get_logger() {
        let urls: Vec<&str> = items.iter().filter_map(|i| i.url.as_deref()).collect();
        let types: Vec<&str> = items.iter().filter_map(|i| i.content_type.as_deref()).collect();
        logger.log(
            "debug",
            "tool",
            "image_generate::fal::api_result",
            &format!("Fal API returned {} image URL(s)", urls.len()),
            Some(
                serde_json::json!({
                    "image_urls": urls,
                    "content_types": types,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    let mut images = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if let Some(img_url) = item.url {
            // Download image from CDN URL
            let dl_start = std::time::Instant::now();
            let img_resp = client
                .get(&img_url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Failed to download Fal image from {}: {}", img_url, e)
                })?;

            let dl_status = img_resp.status();
            if !dl_status.is_success() {
                anyhow::bail!(
                    "Fal image download failed ({}): {}",
                    dl_status,
                    img_url
                );
            }

            let mime = item
                .content_type
                .unwrap_or_else(|| "image/png".to_string());
            let data = img_resp.bytes().await?.to_vec();
            let dl_ms = dl_start.elapsed().as_millis() as u64;

            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "debug",
                    "tool",
                    "image_generate::fal::download",
                    &format!(
                        "Fal image #{} downloaded: {} bytes, {}ms, mime={}",
                        i, data.len(), dl_ms, mime
                    ),
                    Some(
                        serde_json::json!({
                            "index": i,
                            "url": &img_url,
                            "status": dl_status.as_u16(),
                            "size_bytes": data.len(),
                            "download_ms": dl_ms,
                            "mime": &mime,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }

            images.push(GeneratedImage {
                data,
                mime,
                revised_prompt: None,
            });
        }
    }

    if images.is_empty() {
        anyhow::bail!("Fal returned no downloadable images");
    }

    Ok(ImageGenResult { images, text: None })
}
