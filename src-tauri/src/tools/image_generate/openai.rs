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

    let request_body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "n": n,
        "size": size,
        "response_format": "b64_json",
    });

    // Log image generation request
    if let Some(logger) = crate::get_logger() {
        let prompt_preview = if prompt.len() > 500 {
            format!("{}...", crate::truncate_utf8(prompt, 500))
        } else {
            prompt.to_string()
        };
        logger.log("debug", "tool", "image_generate::openai::request",
            &format!("OpenAI image gen request: model={}, size={}, n={}, url={}", model, size, n, url),
            Some(serde_json::json!({
                "api_url": &url,
                "model": model,
                "prompt_preview": prompt_preview,
                "prompt_length": prompt.len(),
                "size": size,
                "n": n,
                "timeout_secs": timeout_secs,
            }).to_string()),
            None, None);
    }

    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()?;
    let request_start = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
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
            "tool", "image_generate::openai::response",
            &format!("OpenAI image gen response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
            Some(serde_json::json!({
                "status": status.as_u16(),
                "ttfb_ms": ttfb_ms,
                "request_id": resp.headers().get("x-request-id").and_then(|v| v.to_str().ok()),
            }).to_string()),
            None, None);
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // Log full error response
        if let Some(logger) = crate::get_logger() {
            logger.log("error", "tool", "image_generate::openai::error",
                &format!("OpenAI image gen error ({}): {}",
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
