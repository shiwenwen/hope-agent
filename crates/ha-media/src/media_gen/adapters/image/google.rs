use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use crate::media_gen::adapters::{GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

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

impl ImageGenAdapter for GoogleProvider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

/// Map size string to Google imageConfig fields.
fn map_size_to_image_config(size: &str) -> Option<(&'static str, Option<&'static str>)> {
    // Returns (aspectRatio, imageSize) where imageSize is "2K" or "4K" if needed
    match size {
        "1024x1024" => Some(("1:1", None)),
        "1024x1536" => Some(("2:3", None)),
        "1536x1024" => Some(("3:2", None)),
        "1024x1792" => Some(("9:16", None)),
        "1792x1024" => Some(("16:9", None)),
        _ => None,
    }
}

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}/v1beta/models/{}:generateContent", base, params.model);

    let thinking_level = params
        .extra
        .get("thinking_level")
        .map(String::as_str)
        .unwrap_or("MINIMAL");

    // Build content parts: input images first, then text prompt
    let mut parts = Vec::new();

    // Add reference/input images as inlineData parts
    for img in params.input_images {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&img.data);
        parts.push(serde_json::json!({
            "inlineData": {
                "mimeType": img.mime,
                "data": b64
            }
        }));
    }

    // Add text prompt
    parts.push(serde_json::json!({ "text": params.prompt }));

    // Build imageConfig from aspectRatio/resolution/size
    let mut image_config = serde_json::Map::new();

    // Explicit aspectRatio takes precedence
    if let Some(ar) = params.aspect_ratio {
        image_config.insert("aspectRatio".to_string(), serde_json::json!(ar));
    } else if let Some((ar, _)) = map_size_to_image_config(params.size) {
        // Map size to aspectRatio for Google API
        image_config.insert("aspectRatio".to_string(), serde_json::json!(ar));
    }

    // Resolution → imageSize
    if let Some(res) = params.resolution {
        match res {
            "2K" | "4K" => {
                image_config.insert("imageSize".to_string(), serde_json::json!(res));
            }
            _ => {} // 1K is default, don't send
        }
    } else if let Some((_, Some(img_size))) = map_size_to_image_config(params.size) {
        image_config.insert("imageSize".to_string(), serde_json::json!(img_size));
    }

    // Build generationConfig
    let mut gen_config = serde_json::json!({
        "responseModalities": ["IMAGE", "TEXT"],
        "thinkingConfig": {
            "thinkingLevel": thinking_level
        }
    });

    if !image_config.is_empty() {
        gen_config.as_object_mut().unwrap().insert(
            "imageConfig".to_string(),
            serde_json::Value::Object(image_config),
        );
    }

    // Log image generation request
    if let Some(logger) = ha_core::get_logger() {
        let prompt_preview = if params.prompt.len() > 500 {
            format!("{}...", ha_core::truncate_utf8(params.prompt, 500))
        } else {
            params.prompt.to_string()
        };
        logger.log(
            "debug",
            "tool",
            "image_generate::google::request",
            &format!(
                "Google image gen request: model={}, thinking={}, edit={}, url={}",
                params.model,
                thinking_level,
                !params.input_images.is_empty(),
                url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": params.model,
                    "prompt_preview": prompt_preview,
                    "prompt_length": params.prompt.len(),
                    "timeout_secs": params.timeout_secs,
                    "thinking_level": thinking_level,
                    "input_images_count": params.input_images.len(),
                    "aspect_ratio": params.aspect_ratio,
                    "resolution": params.resolution,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    let client = ha_core::provider::apply_model_proxy(
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
                "parts": parts
            }],
            "generationConfig": gen_config
        }))
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    // Log response status
    if let Some(logger) = ha_core::get_logger() {
        logger.log(
            if status.is_success() {
                "debug"
            } else {
                "error"
            },
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
        if let Some(logger) = ha_core::get_logger() {
            logger.log(
                "error",
                "tool",
                "image_generate::google::error",
                &format!(
                    "Google image gen error ({}): {}",
                    status.as_u16(),
                    ha_core::truncate_utf8(&body, 500)
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
            format!("{}...", ha_core::truncate_utf8(&body, 300))
        } else {
            body
        };
        anyhow::bail!("Google image generation failed ({}): {}", status, preview);
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
                                let mime =
                                    inline.mime_type.unwrap_or_else(|| "image/png".to_string());
                                let data =
                                    base64::engine::general_purpose::STANDARD.decode(&b64_data)?;
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
    if let Some(logger) = ha_core::get_logger() {
        let image_sizes: Vec<usize> = images.iter().map(|img| img.data.len()).collect();
        let text_preview = text.as_deref().map(|t| {
            if t.len() > 500 {
                format!("{}...", ha_core::truncate_utf8(t, 500))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_gen::adapters::InputImage;
    use ha_core::security::ssrf::SsrfPolicy;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn nano_banana_template_reaches_generate_content_with_panorama_and_references() {
        let _config =
            ha_core::test_support::replace_config_cache(ha_core::config::AppConfig::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let header_end = loop {
                let mut buf = [0; 4096];
                let len = socket.read(&mut buf).await.unwrap();
                assert_ne!(len, 0);
                raw.extend_from_slice(&buf[..len]);
                if let Some(end) = raw.windows(4).position(|b| b == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(raw[..header_end].to_vec()).unwrap();
            let lower = headers.to_ascii_lowercase();
            assert!(headers.starts_with(
                "POST /v1beta/models/gemini-nano-banana-2.1:generateContent HTTP/1.1"
            ));
            assert!(lower.contains("x-goog-api-key: synthetic-key\r\n"));
            let len: usize = lower
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            while raw.len() < header_end + len {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                raw.extend_from_slice(&buf[..n]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&raw[header_end..header_end + len]).unwrap();
            let parts = body["contents"][0]["parts"].as_array().unwrap();
            assert_eq!(parts.len(), 15);
            for (index, part) in parts[..14].iter().enumerate() {
                assert_eq!(part["inlineData"]["mimeType"], "image/png");
                assert_eq!(
                    part["inlineData"]["data"],
                    base64::engine::general_purpose::STANDARD.encode([index as u8])
                );
            }
            assert_eq!(parts[14]["text"], "Change the sky, keep the subjects");
            assert_eq!(
                body["generationConfig"]["imageConfig"],
                serde_json::json!({"aspectRatio":"8:1","imageSize":"4K"})
            );
            assert_eq!(
                body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
                "MINIMAL"
            );
            let response = serde_json::json!({"candidates":[{"content":{"parts":[{"text":"synthetic caption"},{"inlineData":{"mimeType":"image/png","data":"c3ludGhldGljLWltYWdl"}}]}}]}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
        });
        let template = crate::media_gen::catalog::media_provider_templates()
            .into_iter()
            .find(|t| t.key == "google")
            .unwrap();
        let model = &template.models[0];
        let inputs: Vec<_> = (0..14)
            .map(|i| InputImage {
                data: vec![i],
                mime: "image/png".into(),
            })
            .collect();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            GoogleProvider.generate(ImageGenParams {
                api_key: "synthetic-key",
                base_url: Some(&base),
                model: &model.id,
                prompt: "Change the sky, keep the subjects",
                size: "1024x1024",
                n: 1,
                timeout_secs: 3,
                extra: &model.extra,
                aspect_ratio: Some("8:1"),
                resolution: Some("4K"),
                input_images: &inputs,
                mask: None,
                ssrf: SsrfPolicy::AllowPrivate,
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].data, b"synthetic-image");
        assert_eq!(result.text.as_deref(), Some("synthetic caption"));
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}
