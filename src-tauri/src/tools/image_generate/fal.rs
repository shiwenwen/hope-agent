use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

use super::GeneratedImage;

const DEFAULT_BASE_URL: &str = "https://fal.run";
const DEFAULT_MODEL: &str = "fal-ai/flux/dev";

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

pub(super) async fn generate(
    api_key: &str,
    base_url: Option<&str>,
    model: Option<&str>,
    prompt: &str,
    size: &str,
    n: u32,
    timeout_secs: u64,
) -> Result<super::ImageGenResult> {
    let base = base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let model = model.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_MODEL);
    let url = format!("{}/{}", base, model);
    let (w, h) = parse_size(size);

    // Log image generation request
    if let Some(logger) = crate::get_logger() {
        let prompt_preview = if prompt.len() > 500 {
            format!("{}...", crate::truncate_utf8(prompt, 500))
        } else {
            prompt.to_string()
        };
        logger.log("debug", "tool", "image_generate::fal::request",
            &format!("Fal image gen request: model={}, size={}x{}, n={}, url={}", model, w, h, n, url),
            Some(serde_json::json!({
                "api_url": &url,
                "model": model,
                "prompt_preview": prompt_preview,
                "prompt_length": prompt.len(),
                "size": size,
                "width": w,
                "height": h,
                "n": n,
                "timeout_secs": timeout_secs,
            }).to_string()),
            None, None);
    }

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(timeout_secs))
    ).build()?;
    let request_start = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Key {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "prompt": prompt,
            "num_images": n,
            "output_format": "png",
            "image_size": { "width": w, "height": h },
        }))
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    // Log response status
    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() { "debug" } else { "error" },
            "tool", "image_generate::fal::response",
            &format!("Fal image gen response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
            Some(serde_json::json!({
                "status": status.as_u16(),
                "ttfb_ms": ttfb_ms,
            }).to_string()),
            None, None);
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log("error", "tool", "image_generate::fal::error",
                &format!("Fal image gen error ({}): {}",
                    status.as_u16(), crate::truncate_utf8(&body, 500)),
                Some(serde_json::json!({
                    "status": status.as_u16(),
                    "error_body": &body,
                }).to_string()),
                None, None);
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

    let mut images = Vec::new();
    for item in items {
        if let Some(img_url) = item.url {
            // Download image from CDN URL
            let img_resp = client
                .get(&img_url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to download Fal image from {}: {}", img_url, e))?;

            if !img_resp.status().is_success() {
                anyhow::bail!(
                    "Fal image download failed ({}): {}",
                    img_resp.status(),
                    img_url
                );
            }

            let mime = item
                .content_type
                .unwrap_or_else(|| "image/png".to_string());
            let data = img_resp.bytes().await?.to_vec();
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

    Ok(super::ImageGenResult { images, text: None })
}
