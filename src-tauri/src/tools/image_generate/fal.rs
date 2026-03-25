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
) -> Result<Vec<GeneratedImage>> {
    let base = base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let model = model.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_MODEL);
    let url = format!("{}/{}", base, model);
    let (w, h) = parse_size(size);

    let client = Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Key {}", api_key))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .json(&serde_json::json!({
            "prompt": prompt,
            "num_images": n,
            "output_format": "png",
            "image_size": { "width": w, "height": h },
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let preview = if body.len() > 300 {
            format!("{}...", &body[..300])
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

    Ok(images)
}
