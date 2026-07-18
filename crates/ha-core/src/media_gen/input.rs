//! Input/reference image loading (local path / URL / data-URL), shared by
//! the `image_generate` tool front-end and the design space's image paths.
//! Migrated from `tools/image_generate/helpers.rs`.

use anyhow::Result;
use base64::Engine;

use super::adapters::InputImage;
use super::types::MAX_INPUT_IMAGES;

/// 10 MB cap — hostile upstreams that ignore Content-Length can't OOM us;
/// over-cap downloads truncate and fail to decode at the provider layer.
const MAX_IMAGE_DOWNLOAD_BYTES: usize = 10_485_760;

/// Load an input image from a local file path or HTTP(S) URL.
pub async fn load_input_image(path_or_url: &str) -> Result<InputImage> {
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

/// Load a batch of reference images (paths / URLs / data URLs) for image-to-image
/// generation. Caps at `MAX_INPUT_IMAGES`; a single bad entry is logged and skipped
/// rather than failing the whole generation — matches the owner-plane single-reference
/// degrade behaviour (a bad reference must never sink an otherwise valid generate).
pub async fn load_input_images(paths: &[String]) -> Result<Vec<InputImage>> {
    let mut out = Vec::new();
    for p in paths {
        if p.trim().is_empty() {
            continue;
        }
        if out.len() >= MAX_INPUT_IMAGES {
            crate::app_warn!(
                "media_gen",
                "load_input_images",
                "more than {} reference images provided; extra ignored",
                MAX_INPUT_IMAGES
            );
            break;
        }
        match load_input_image(p).await {
            Ok(img) => out.push(img),
            Err(e) => crate::app_warn!(
                "media_gen",
                "load_input_images",
                "reference image '{}' failed to load, skipping: {}",
                p,
                e
            ),
        }
    }
    Ok(out)
}

/// Decode a data URL into InputImage.
pub fn decode_data_url(url: &str) -> Result<InputImage> {
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
pub fn infer_resolution(images: &[InputImage]) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_input_images_skips_empty_and_caps() {
        // Valid data URLs load; empty entries are skipped (not fatal).
        let ok = "data:image/png;base64,aGVsbG8=".to_string();
        let out = load_input_images(&[ok.clone(), "".to_string(), "   ".to_string(), ok.clone()])
            .await
            .unwrap();
        assert_eq!(out.len(), 2, "two valid + two empty → two loaded");
        // More than MAX_INPUT_IMAGES is capped, not errored.
        let many: Vec<String> = std::iter::repeat_n(ok, MAX_INPUT_IMAGES + 3).collect();
        let capped = load_input_images(&many).await.unwrap();
        assert_eq!(capped.len(), MAX_INPUT_IMAGES);
    }

    #[tokio::test]
    async fn load_input_images_bad_entry_skipped_not_fatal() {
        // A malformed data URL is skipped; the whole batch still succeeds.
        let out = load_input_images(&[
            "data:image/png;base64,aGVsbG8=".to_string(),
            "data:garbage-no-comma".to_string(),
        ])
        .await
        .unwrap();
        assert_eq!(out.len(), 1, "bad entry skipped, good one kept");
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
    fn infer_resolution_zero_dim_returns_1k() {
        // Empty input list → max dim stays 0 → "1K".
        assert_eq!(infer_resolution(&[]), "1K");
    }
}
