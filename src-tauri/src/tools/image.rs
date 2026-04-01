use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::browser::IMAGE_BASE64_PREFIX;
use super::expand_tilde;
use super::read::{detect_image_mime, resize_image_if_needed};

/// Default maximum number of images per single tool call.
const DEFAULT_MAX_IMAGES: usize = 10;
/// Hard cap on max images (user cannot exceed this).
const CAP_MAX_IMAGES: usize = 20;
/// Maximum bytes to download for a remote image (10 MB).
const IMAGE_MAX_FETCH_BYTES: usize = 10 * 1024 * 1024;
/// HTTP timeout for fetching remote images.
const FETCH_TIMEOUT_SECS: u64 = 30;

// ── Image Tool Config ───────────────────────────────────────────

/// Persistent image tool configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageToolConfig {
    /// Maximum number of images per single tool call
    #[serde(default = "default_max_images")]
    pub max_images: usize,
}

fn default_max_images() -> usize {
    DEFAULT_MAX_IMAGES
}

impl Default for ImageToolConfig {
    fn default() -> Self {
        Self {
            max_images: DEFAULT_MAX_IMAGES,
        }
    }
}

/// Load image tool config from the global config store, clamped to hard caps.
fn load_image_config() -> ImageToolConfig {
    let mut cfg = crate::provider::load_store()
        .map(|s| s.image)
        .unwrap_or_default();
    cfg.max_images = cfg.max_images.min(CAP_MAX_IMAGES);
    cfg
}

// ── Image Source Types ───────────────────────────────────────────────

/// Normalized image source parsed from tool arguments.
enum ImageSource {
    File { path: String },
    Url { url: String },
    Clipboard,
    Screenshot { monitor: Option<usize> },
}

/// Parse tool arguments into a list of image sources.
/// Supports: `images` array, `path` shorthand, `url` shorthand.
fn normalize_sources(args: &Value, max_images: usize) -> Result<Vec<ImageSource>> {
    let mut sources = Vec::new();

    // 1. Check `images` array parameter
    if let Some(arr) = args.get("images").and_then(|v| v.as_array()) {
        for item in arr {
            let src_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            match src_type {
                "file" => {
                    let path = item
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("images[].type='file' requires 'path'"))?;
                    sources.push(ImageSource::File {
                        path: path.to_string(),
                    });
                }
                "url" => {
                    let url = item
                        .get("url")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("images[].type='url' requires 'url'"))?;
                    sources.push(ImageSource::Url {
                        url: url.to_string(),
                    });
                }
                "clipboard" => {
                    sources.push(ImageSource::Clipboard);
                }
                "screenshot" => {
                    let monitor = item.get("monitor").and_then(|v| v.as_u64()).map(|n| n as usize);
                    sources.push(ImageSource::Screenshot { monitor });
                }
                other => {
                    return Err(anyhow!("Unknown image source type: '{}'", other));
                }
            }
        }
    }

    // 2. `path` shorthand (backward compatible)
    if sources.is_empty() {
        if let Some(path) = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
        {
            sources.push(ImageSource::File {
                path: path.to_string(),
            });
        }
    }

    // 3. `url` shorthand
    if sources.is_empty() {
        if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
            sources.push(ImageSource::Url {
                url: url.to_string(),
            });
        }
    }

    if sources.is_empty() {
        return Err(anyhow!(
            "At least one image source is required (use 'path', 'url', or 'images' parameter)"
        ));
    }
    if sources.len() > max_images {
        return Err(anyhow!(
            "Too many images: {} provided, maximum is {}",
            sources.len(),
            max_images
        ));
    }

    Ok(sources)
}

// ── Image Resolution ─────────────────────────────────────────────────

/// Resolve a file path to image bytes.
fn resolve_file(path_raw: &str) -> Result<(Vec<u8>, String)> {
    let path = expand_tilde(path_raw);
    let file_path = std::path::Path::new(&path);
    if !file_path.exists() {
        return Err(anyhow!("File not found: {}", path));
    }
    let data = std::fs::read(file_path)?;
    Ok((data, format!("file: {}", path)))
}

/// Fetch an image from a URL (HTTP/HTTPS) or decode a data URI.
async fn resolve_url(url: &str) -> Result<(Vec<u8>, String)> {
    // Handle data: URIs
    if url.starts_with("data:") {
        return decode_data_uri(url);
    }

    // SSRF protection (reuse existing check)
    crate::tools::web_fetch::check_ssrf_safe(url).await?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()?;

    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {} fetching {}", status, url));
    }

    // Validate content type if present
    if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
        if let Ok(ct_str) = ct.to_str() {
            if !ct_str.starts_with("image/") && !ct_str.starts_with("application/octet-stream") {
                return Err(anyhow!(
                    "URL returned non-image content type: {}",
                    ct_str
                ));
            }
        }
    }

    let bytes = resp.bytes().await?;
    if bytes.len() > IMAGE_MAX_FETCH_BYTES {
        return Err(anyhow!(
            "Image too large: {} bytes (max {}MB)",
            bytes.len(),
            IMAGE_MAX_FETCH_BYTES / 1024 / 1024
        ));
    }

    // Truncate URL for label
    let label = if url.len() > 80 {
        format!("url: {}...", &url[..77])
    } else {
        format!("url: {}", url)
    };
    Ok((bytes.to_vec(), label))
}

/// Decode a `data:image/...;base64,...` URI.
fn decode_data_uri(uri: &str) -> Result<(Vec<u8>, String)> {
    let rest = uri
        .strip_prefix("data:")
        .ok_or_else(|| anyhow!("Invalid data URI"))?;
    let (meta, b64_data) = rest
        .split_once(",")
        .ok_or_else(|| anyhow!("Invalid data URI: missing comma"))?;

    if !meta.contains("base64") {
        return Err(anyhow!("Only base64-encoded data URIs are supported"));
    }

    let mime = meta.split(';').next().unwrap_or("image/png");
    if !mime.starts_with("image/") {
        return Err(anyhow!("Data URI is not an image: {}", mime));
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_data)
        .map_err(|e| anyhow!("Failed to decode base64 data URI: {}", e))?;

    Ok((bytes, format!("data URI ({})", mime)))
}

/// Read image from system clipboard.
fn resolve_clipboard() -> Result<(Vec<u8>, String)> {
    use arboard::Clipboard;
    use image::RgbaImage;
    use std::io::Cursor;

    let mut clipboard =
        Clipboard::new().map_err(|e| anyhow!("Failed to access clipboard: {}", e))?;

    let img_data = clipboard
        .get_image()
        .map_err(|_| anyhow!("Clipboard does not contain an image"))?;

    let rgba = RgbaImage::from_raw(img_data.width as u32, img_data.height as u32, img_data.bytes.into_owned())
        .ok_or_else(|| anyhow!("Failed to create image from clipboard data"))?;

    let dyn_img = image::DynamicImage::ImageRgba8(rgba);

    let mut buf = Cursor::new(Vec::new());
    dyn_img
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| anyhow!("Failed to encode clipboard image as PNG: {}", e))?;

    let label = format!(
        "clipboard ({}x{})",
        img_data.width, img_data.height
    );
    Ok((buf.into_inner(), label))
}

/// Capture a screenshot of the desktop.
fn resolve_screenshot(monitor_index: Option<usize>) -> Result<(Vec<u8>, String)> {
    use std::io::Cursor;
    use xcap::Monitor;

    let monitors = Monitor::all().map_err(|e| anyhow!("Failed to list monitors: {}", e))?;
    if monitors.is_empty() {
        return Err(anyhow!("No monitors detected"));
    }

    let idx = monitor_index.unwrap_or(0);
    let monitor = monitors
        .get(idx)
        .ok_or_else(|| anyhow!("Monitor index {} out of range (available: {})", idx, monitors.len()))?;

    let rgba_image = monitor
        .capture_image()
        .map_err(|e| anyhow!("Screenshot capture failed (may need Screen Recording permission): {}", e))?;

    let (w, h) = (rgba_image.width(), rgba_image.height());
    let dyn_img = image::DynamicImage::ImageRgba8(rgba_image);

    let mut buf = Cursor::new(Vec::new());
    dyn_img
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| anyhow!("Failed to encode screenshot as PNG: {}", e))?;

    let label = format!("screenshot ({}x{}, monitor {})", w, h, idx);
    Ok((buf.into_inner(), label))
}

// ── Main Tool Entry ──────────────────────────────────────────────────

/// Tool: image — analyze one or more images from files, URLs, clipboard, or screenshot.
/// Returns base64-encoded image markers for LLM vision.
pub(crate) async fn tool_image(args: &Value) -> Result<String> {
    let config = load_image_config();
    let sources = normalize_sources(args, config.max_images)?;
    let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let total = sources.len();
    let mut result_parts: Vec<String> = Vec::new();
    let mut success_count = 0usize;

    if !prompt.is_empty() {
        result_parts.push(format!("Image analysis prompt: {}\n", prompt));
    }

    for (i, source) in sources.iter().enumerate() {
        let idx = i + 1;
        let label_prefix = if total > 1 {
            format!("[Image {}/{}] ", idx, total)
        } else {
            String::new()
        };

        // Resolve image bytes
        let resolve_result = match source {
            ImageSource::File { path } => resolve_file(path),
            ImageSource::Url { url } => resolve_url(url).await,
            ImageSource::Clipboard => resolve_clipboard(),
            ImageSource::Screenshot { monitor } => resolve_screenshot(*monitor),
        };

        match resolve_result {
            Ok((data, source_label)) => {
                // Validate image format
                let mime = match detect_image_mime(&data) {
                    Some(m) => m,
                    None => {
                        result_parts.push(format!(
                            "{}ERROR: Not a recognized image format ({})\n",
                            label_prefix, source_label
                        ));
                        continue;
                    }
                };

                // Resize if needed
                match resize_image_if_needed(&data, mime) {
                    Ok((b64, final_mime)) => {
                        result_parts.push(format!(
                            "{}{}__{}__\n{}{} ({} bytes, {})\n",
                            IMAGE_BASE64_PREFIX,
                            final_mime,
                            b64,
                            label_prefix,
                            source_label,
                            data.len(),
                            final_mime,
                        ));
                        success_count += 1;
                    }
                    Err(e) => {
                        result_parts.push(format!(
                            "{}ERROR: Failed to process image ({}): {}\n",
                            label_prefix, source_label, e
                        ));
                    }
                }
            }
            Err(e) => {
                result_parts.push(format!("{}ERROR: {}\n", label_prefix, e));
            }
        }
    }

    if success_count == 0 {
        return Ok(format!(
            "Error: All {} image(s) failed to load.\n\n{}",
            total,
            result_parts.join("\n")
        ));
    }

    Ok(result_parts.join("\n"))
}
