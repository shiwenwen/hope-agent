use anyhow::Result;
use base64::Engine;

use crate::provider;
use super::types::*;

// ── Public Helpers ──────────────────────────────────────────────

/// Check if at least one provider is enabled with an API key.
#[allow(dead_code)]
pub fn has_configured_provider() -> bool {
    provider::load_store()
        .map(|s| has_configured_provider_from_config(&s.image_generate))
        .unwrap_or(false)
}

/// Check from a config reference (avoids re-loading store).
pub fn has_configured_provider_from_config(config: &ImageGenConfig) -> bool {
    config
        .providers
        .iter()
        .any(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
}

/// Get the display name for a provider entry.
pub fn provider_display_name(entry: &ImageGenProviderEntry) -> String {
    super::resolve_provider(&entry.id)
        .map(|p| p.display_name().to_string())
        .unwrap_or_else(|| entry.id.clone())
}

/// Get the effective model name for a provider entry.
pub fn effective_model(entry: &ImageGenProviderEntry) -> String {
    entry
        .model
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            super::resolve_provider(&entry.id)
                .map(|p| p.default_model().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        })
}

/// Find a provider entry by model name (for LLM tool `model` parameter routing).
pub(super) fn find_provider_by_model<'a>(
    model: &str,
    config: &'a ImageGenConfig,
) -> Option<&'a ImageGenProviderEntry> {
    let enabled_providers = config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()));

    // 1. Exact match on user-configured model
    for entry in config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
    {
        if entry.model.as_deref() == Some(model) {
            return Some(entry);
        }
    }

    // 2. Match against provider's default model
    for entry in enabled_providers {
        if let Some(impl_) = super::resolve_provider(&entry.id) {
            if impl_.default_model() == model {
                return Some(entry);
            }
        }
    }

    None
}

// ── Input Image Loading ─────────────────────────────────────────

/// Load an input image from a local file path or HTTP(S) URL.
pub(super) async fn load_input_image(path_or_url: &str) -> Result<InputImage> {
    let trimmed = path_or_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Empty image path/URL");
    }

    // Data URL
    if trimmed.starts_with("data:") {
        return decode_data_url(trimmed);
    }

    // HTTP(S) URL
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let resp = client.get(trimmed).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Failed to download image from {} ({})",
                trimmed,
                resp.status()
            );
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/png")
            .to_string();
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("image/png")
            .trim()
            .to_string();
        let data = resp.bytes().await?.to_vec();
        return Ok(InputImage { data, mime });
    }

    // Local file path (expand ~ to home dir)
    let resolved = if trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            home.join(&trimmed[2..])
        } else {
            std::path::PathBuf::from(trimmed)
        }
    } else if trimmed.starts_with("file://") {
        std::path::PathBuf::from(&trimmed[7..])
    } else {
        std::path::PathBuf::from(trimmed)
    };

    let data = tokio::fs::read(&resolved).await.map_err(|e| {
        anyhow::anyhow!("Failed to read image file '{}': {}", resolved.display(), e)
    })?;

    // Infer MIME from extension
    let mime = match resolved.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "image/png",
    };

    Ok(InputImage {
        data,
        mime: mime.to_string(),
    })
}

/// Decode a data URL into InputImage.
pub(super) fn decode_data_url(url: &str) -> Result<InputImage> {
    // data:image/png;base64,xxxx
    let after_data = url.strip_prefix("data:").unwrap_or(url);
    let (header, b64) = after_data
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("Invalid data URL format"))?;
    let mime = header.split(';').next().unwrap_or("image/png").to_string();
    let data = base64::engine::general_purpose::STANDARD.decode(b64.trim())?;
    Ok(InputImage { data, mime })
}

/// Infer resolution from input images using the `image` crate.
pub(super) fn infer_resolution(images: &[InputImage]) -> &'static str {
    let mut max_dim: u32 = 0;
    for img in images {
        if let Ok(reader) =
            image::ImageReader::new(std::io::Cursor::new(&img.data)).with_guessed_format()
        {
            if let Ok(dims) = reader.into_dimensions() {
                max_dim = max_dim.max(dims.0).max(dims.1);
            }
        }
    }
    if max_dim >= 3000 {
        "4K"
    } else if max_dim >= 1500 {
        "2K"
    } else {
        "1K"
    }
}

/// Validate tool parameters against provider capabilities.
pub(super) fn validate_capabilities(
    caps: &ImageGenCapabilities,
    provider_name: &str,
    is_edit: bool,
    count: u32,
    aspect_ratio: Option<&str>,
    resolution: Option<&str>,
    size: &str,
    input_count: usize,
) -> Result<()> {
    let mode_caps = if is_edit {
        &caps.edit_as_mode()
    } else {
        &caps.generate
    };

    if is_edit {
        if !caps.edit.enabled {
            anyhow::bail!(
                "{} does not support reference-image editing.",
                provider_name
            );
        }
        if input_count as u32 > caps.edit.max_input_images {
            anyhow::bail!(
                "{} edit supports at most {} reference image(s), got {}.",
                provider_name,
                caps.edit.max_input_images,
                input_count
            );
        }
    }

    let max_count = if is_edit {
        caps.edit.max_count
    } else {
        mode_caps.max_count
    };
    if count > max_count {
        anyhow::bail!(
            "{} {} supports at most {} image(s), requested {}.",
            provider_name,
            if is_edit { "edit" } else { "generate" },
            max_count,
            count
        );
    }

    if aspect_ratio.is_some() && !mode_caps.supports_aspect_ratio {
        anyhow::bail!(
            "{} {} does not support aspectRatio.",
            provider_name,
            if is_edit { "edit" } else { "generate" }
        );
    }

    if let Some(ar) = aspect_ratio {
        if let Some(ref geo) = caps.geometry {
            if !geo.aspect_ratios.is_empty() && !geo.aspect_ratios.contains(&ar) {
                anyhow::bail!(
                    "{} aspectRatio must be one of: {}",
                    provider_name,
                    geo.aspect_ratios.join(", ")
                );
            }
        }
    }

    if resolution.is_some() && !mode_caps.supports_resolution {
        anyhow::bail!(
            "{} {} does not support resolution.",
            provider_name,
            if is_edit { "edit" } else { "generate" }
        );
    }

    if let Some(res) = resolution {
        if let Some(ref geo) = caps.geometry {
            if !geo.resolutions.is_empty() && !geo.resolutions.contains(&res) {
                anyhow::bail!(
                    "{} resolution must be one of: {}",
                    provider_name,
                    geo.resolutions.join(", ")
                );
            }
        }
    }

    if size != "1024x1024" && !mode_caps.supports_size {
        // Only validate non-default sizes
        anyhow::bail!(
            "{} {} does not support custom size.",
            provider_name,
            if is_edit { "edit" } else { "generate" }
        );
    }

    if mode_caps.supports_size {
        if let Some(ref geo) = caps.geometry {
            if !geo.sizes.is_empty() && !geo.sizes.contains(&size) {
                anyhow::bail!(
                    "{} size must be one of: {}",
                    provider_name,
                    geo.sizes.join(", ")
                );
            }
        }
    }

    Ok(())
}
