use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use super::{GeneratedImage, ImageGenParams, ImageGenProviderImpl, ImageGenResult};

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

pub(crate) struct GoogleProvider;

impl ImageGenProviderImpl for GoogleProvider {
    fn id(&self) -> &str {
        "google"
    }

    fn display_name(&self) -> &str {
        "Google"
    }

    fn default_model(&self) -> &str {
        DEFAULT_MODEL
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
    let url = format!("{}/v1beta/models/{}:generateContent", base, params.model);

    let thinking_level = params.extra.thinking_level.as_deref().unwrap_or("MINIMAL");

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
            "image_generate::google::request",
            &format!(
                "Google image gen request: model={}, thinking={}, url={}",
                params.model, thinking_level, url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": params.model,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "timeout_secs": params.timeout_secs,
                    "thinking_level": thinking_level,
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
        .header("x-goog-api-key", params.api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": params.prompt }]
            }],
            "generationConfig": {
                "responseModalities": ["IMAGE", "TEXT"],
                "thinkingConfig": {
                    "thinkingLevel": thinking_level
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
            "tool",
            "image_generate::google::response",
            &format!(
                "Google image gen response: status={}, ttfb={}ms",
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
                "image_generate::google::error",
                &format!(
                    "Google image gen error ({}): {}",
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

    // Log successful result details (everything except raw image bytes)
    if let Some(logger) = crate::get_logger() {
        let image_sizes: Vec<usize> = images.iter().map(|img| img.data.len()).collect();
        let text_preview = text.as_deref().map(|t| {
            if t.len() > 500 {
                format!("{}...", crate::truncate_utf8(t, 500))
            } else {
                t.to_string()
            }
        });
        logger.log(
            "debug",
            "tool",
            "image_generate::google::result",
            &format!(
                "Google image gen result: {} image(s), {} text part(s), sizes={:?}",
                images.len(),
                text_parts.len(),
                image_sizes
            ),
            Some(
                serde_json::json!({
                    "image_count": images.len(),
                    "image_sizes_bytes": image_sizes,
                    "mime_types": images.iter().map(|img| &img.mime).collect::<Vec<_>>(),
                    "text_parts_count": text_parts.len(),
                    "text_preview": text_preview,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok(ImageGenResult { images, text })
}
