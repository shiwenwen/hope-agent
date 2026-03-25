use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use super::{GeneratedImage, ImageGenResult};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_MODEL: &str = "gemini-3.1-flash-image-preview";

#[derive(Deserialize)]
struct GoogleResponse {
    candidates: Option<Vec<GoogleCandidate>>,
}

#[derive(Deserialize)]
struct GoogleCandidate {
    content: Option<GoogleContent>,
}

#[derive(Deserialize)]
struct GoogleContent {
    parts: Option<Vec<GooglePart>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GooglePart {
    text: Option<String>,
    inline_data: Option<GoogleInlineData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleInlineData {
    mime_type: Option<String>,
    data: Option<String>,
}

pub(super) async fn generate(
    api_key: &str,
    base_url: Option<&str>,
    model: Option<&str>,
    prompt: &str,
    thinking_level: Option<&str>,
    timeout_secs: u64,
) -> Result<ImageGenResult> {
    let base = base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let model = model.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_MODEL);
    let url = format!("{}/v1beta/models/{}:generateContent", base, model);

    // Log image generation request
    if let Some(logger) = crate::get_logger() {
        let prompt_preview = if prompt.len() > 500 {
            format!("{}...", crate::truncate_utf8(prompt, 500))
        } else {
            prompt.to_string()
        };
        logger.log("debug", "tool", "image_generate::google::request",
            &format!("Google image gen request: model={}, url={}", model, url),
            Some(serde_json::json!({
                "api_url": &url,
                "model": model,
                "prompt_preview": prompt_preview,
                "prompt_length": prompt.len(),
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
        .header("x-goog-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": {
                "responseModalities": ["IMAGE", "TEXT"],
                "thinkingConfig": {
                    "thinkingLevel": thinking_level.unwrap_or("MINIMAL")
                }
            }
        }))
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    // Log response status
    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() { "debug" } else { "error" },
            "tool", "image_generate::google::response",
            &format!("Google image gen response: status={}, ttfb={}ms", status.as_u16(), ttfb_ms),
            Some(serde_json::json!({
                "status": status.as_u16(),
                "ttfb_ms": ttfb_ms,
            }).to_string()),
            None, None);
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log("error", "tool", "image_generate::google::error",
                &format!("Google image gen error ({}): {}",
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
            "Google image generation failed ({}): {}",
            status,
            preview
        );
    }

    let body: GoogleResponse = resp.json().await?;
    let mut images = Vec::new();
    let mut text_parts = Vec::new();

    if let Some(candidates) = body.candidates {
        for candidate in candidates {
            if let Some(content) = candidate.content {
                if let Some(parts) = content.parts {
                    for part in parts {
                        if let Some(text) = part.text {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                text_parts.push(trimmed.to_string());
                            }
                        }
                        if let Some(inline) = part.inline_data {
                            if let Some(b64_data) = inline.data {
                                let mime = inline
                                    .mime_type
                                    .unwrap_or_else(|| "image/png".to_string());
                                let data = base64::engine::general_purpose::STANDARD
                                    .decode(&b64_data)?;
                                images.push(GeneratedImage {
                                    data,
                                    mime,
                                    revised_prompt: None,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if images.is_empty() {
        anyhow::bail!("Google returned no image data");
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    };

    Ok(ImageGenResult { images, text })
}
