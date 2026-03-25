use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use super::GeneratedImage;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_MODEL: &str = "gemini-2.0-flash-preview-image-generation";

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
    #[allow(dead_code)]
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
    timeout_secs: u64,
) -> Result<Vec<GeneratedImage>> {
    let base = base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let model = model.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_MODEL);
    let url = format!("{}/v1beta/models/{}:generateContent", base, model);

    let client = Client::new();
    let resp = client
        .post(&url)
        .header("x-goog-api-key", api_key)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .json(&serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"]
            }
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
            "Google image generation failed ({}): {}",
            status,
            preview
        );
    }

    let body: GoogleResponse = resp.json().await?;
    let mut images = Vec::new();

    if let Some(candidates) = body.candidates {
        for candidate in candidates {
            if let Some(content) = candidate.content {
                if let Some(parts) = content.parts {
                    for part in parts {
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

    Ok(images)
}
