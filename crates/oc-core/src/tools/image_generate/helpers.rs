use anyhow::Result;
use base64::Engine;

use super::types::*;

/// 10 MB cap — hostile upstreams that ignore Content-Length can't OOM us;
/// over-cap downloads truncate and fail to decode at the provider layer.
const MAX_IMAGE_DOWNLOAD_BYTES: usize = 10_485_760;

// ── Public Helpers ──────────────────────────────────────────────

/// Check if at least one provider is enabled with an API key.
#[allow(dead_code)]
pub fn has_configured_provider() -> bool {
    has_configured_provider_from_config(&crate::config::cached_config().image_generate)
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
        let parsed_url = {
            let ssrf_cfg = &crate::config::cached_config().ssrf;
            crate::security::ssrf::check_url(
                trimmed,
                ssrf_cfg.image_generate(),
                &ssrf_cfg.trusted_hosts,
            )
            .await?
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let resp = client.get(parsed_url).send().await?;
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
        let data = crate::security::http_stream::read_bytes_capped(resp, MAX_IMAGE_DOWNLOAD_BYTES)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read image from {}: {}", trimmed, e))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        id: &str,
        enabled: bool,
        key: Option<&str>,
        model: Option<&str>,
    ) -> ImageGenProviderEntry {
        ImageGenProviderEntry {
            id: id.to_string(),
            enabled,
            api_key: key.map(|k| k.to_string()),
            base_url: None,
            model: model.map(|m| m.to_string()),
            thinking_level: None,
        }
    }

    fn cfg(entries: Vec<ImageGenProviderEntry>) -> ImageGenConfig {
        ImageGenConfig {
            providers: entries,
            timeout_seconds: 60,
            default_size: "1024x1024".to_string(),
        }
    }

    #[test]
    fn has_configured_provider_rejects_disabled_or_empty_key() {
        assert!(!has_configured_provider_from_config(&cfg(vec![])));
        assert!(!has_configured_provider_from_config(&cfg(vec![entry(
            "openai",
            false,
            Some("sk-x"),
            None
        )])));
        assert!(!has_configured_provider_from_config(&cfg(vec![entry(
            "openai",
            true,
            Some(""),
            None
        )])));
        assert!(!has_configured_provider_from_config(&cfg(vec![entry(
            "openai", true, None, None
        )])));
    }

    #[test]
    fn has_configured_provider_accepts_one_enabled_with_key() {
        assert!(has_configured_provider_from_config(&cfg(vec![
            entry("openai", false, Some("sk-x"), None),
            entry("google", true, Some("real-key"), None),
        ])));
    }

    #[test]
    fn effective_model_prefers_user_override() {
        let e = entry("openai", true, Some("sk"), Some("dall-e-custom"));
        assert_eq!(effective_model(&e), "dall-e-custom");
    }

    #[test]
    fn effective_model_falls_back_to_provider_default() {
        let e = entry("openai", true, Some("sk"), None);
        // Default model for openai provider is non-empty (per ImageGenProviderImpl).
        let got = effective_model(&e);
        assert!(!got.is_empty());
        assert_ne!(got, "unknown");
    }

    #[test]
    fn effective_model_unknown_provider_returns_unknown() {
        let e = entry("nonesuch", true, Some("sk"), None);
        assert_eq!(effective_model(&e), "unknown");
    }

    #[test]
    fn effective_model_empty_string_treated_as_none() {
        let e = entry("openai", true, Some("sk"), Some(""));
        // Empty override must not win; falls back to provider default.
        let got = effective_model(&e);
        assert!(!got.is_empty());
        assert_ne!(got, "");
    }

    #[test]
    fn find_provider_by_model_matches_user_override_exactly() {
        let config = cfg(vec![
            entry("openai", true, Some("sk"), Some("custom-model")),
            entry("google", true, Some("sk2"), None),
        ]);
        let found = find_provider_by_model("custom-model", &config).unwrap();
        assert_eq!(found.id, "openai");
    }

    #[test]
    fn find_provider_by_model_misses_when_no_configured_provider() {
        let config = cfg(vec![entry("openai", false, Some("sk"), Some("x"))]);
        assert!(find_provider_by_model("x", &config).is_none());
    }

    #[test]
    fn find_provider_by_model_missing_returns_none() {
        let config = cfg(vec![entry("openai", true, Some("sk"), Some("foo"))]);
        assert!(find_provider_by_model("not-a-model", &config).is_none());
    }

    #[test]
    fn decode_data_url_base64_png() {
        // Base64 for the ASCII string "hello" == "aGVsbG8=".
        let url = "data:image/png;base64,aGVsbG8=";
        let img = decode_data_url(url).unwrap();
        assert_eq!(img.mime, "image/png");
        assert_eq!(img.data, b"hello");
    }

    #[test]
    fn decode_data_url_missing_comma_is_error() {
        let err = decode_data_url("data:image/png;base64aGVsbG8=");
        assert!(err.is_err(), "expected comma-less data URL to fail");
    }

    #[test]
    fn decode_data_url_empty_header_leaves_mime_empty() {
        // `data:,xxx` (no mime / no parameters) splits to an empty header.
        // `split(';').next()` returns `Some("")`, so the `unwrap_or` fallback
        // doesn't trigger and mime stays "". Documenting current behavior so
        // downstream consumers can decide whether to tolerate it.
        let img = decode_data_url("data:,aGVsbG8=").unwrap();
        assert_eq!(img.mime, "");
        assert_eq!(img.data, b"hello");
    }

    #[test]
    fn infer_resolution_zero_dim_returns_1k() {
        // Empty input list → max dim stays 0 → "1K".
        assert_eq!(infer_resolution(&[]), "1K");
    }

    #[test]
    fn infer_resolution_invalid_bytes_returns_1k() {
        // Garbage bytes that can't be decoded → dims unknown → "1K".
        let bogus = InputImage {
            data: vec![0u8; 32],
            mime: "image/png".to_string(),
        };
        assert_eq!(infer_resolution(&[bogus]), "1K");
    }
}
