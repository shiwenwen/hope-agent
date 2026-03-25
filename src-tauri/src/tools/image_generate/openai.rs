use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use super::GeneratedImage;

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
    let url = format!("{}/v1/images/generations", base);

    let client = Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "n": n,
            "size": size,
            "response_format": "b64_json",
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
        anyhow::bail!(
            "OpenAI image generation failed ({}): {}",
            status,
            preview
        );
    }

    let body: OpenAIImageResponse = resp.json().await?;
    let items = body.data.unwrap_or_default();
    if items.is_empty() {
        anyhow::bail!("OpenAI returned no images");
    }

    let mut images = Vec::new();
    for item in items {
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

    Ok(images)
}
