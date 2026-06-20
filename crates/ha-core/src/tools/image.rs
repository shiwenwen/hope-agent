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
pub(crate) const CAP_MAX_IMAGES: usize = 20;
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

/// Effective per-call image cap: the configured value clamped to the hard cap.
///
/// Single source of truth shared by the tool schema (advertised `maxItems` /
/// "max N" text) and runtime enforcement (`normalize_sources`). Routing both
/// through one function prevents the schema from advertising a cap the tool
/// won't accept — which would make the model send more images than allowed and
/// hit a hard "Too many images" rejection.
pub(crate) fn effective_max_images() -> usize {
    crate::config::cached_config()
        .image
        .max_images
        .min(CAP_MAX_IMAGES)
}

// ── Image Source Types ───────────────────────────────────────────────

/// Normalized image source parsed from tool arguments.
///
/// `Clipboard` and `Screenshot` only exist when the `desktop-tools` feature
/// is on — the headless ha-server build (Docker image) drops `xcap` and
/// `arboard` and surfaces a clear error if the user tries to use those
/// sources.
enum ImageSource {
    File {
        path: String,
        label: Option<String>,
    },
    Url {
        url: String,
        label: Option<String>,
    },
    #[cfg(feature = "desktop-tools")]
    Clipboard {
        label: Option<String>,
    },
    #[cfg(feature = "desktop-tools")]
    Screenshot {
        monitor: Option<usize>,
        label: Option<String>,
    },
}

impl ImageSource {
    /// The optional caller-supplied label, independent of the source variant.
    fn label(&self) -> Option<&str> {
        match self {
            ImageSource::File { label, .. } | ImageSource::Url { label, .. } => label.as_deref(),
            #[cfg(feature = "desktop-tools")]
            ImageSource::Clipboard { label } => label.as_deref(),
            #[cfg(feature = "desktop-tools")]
            ImageSource::Screenshot { label, .. } => label.as_deref(),
        }
    }
}

/// Parse tool arguments into a list of image sources.
/// Supports: `images` array, `path` shorthand, `url` shorthand.
fn normalize_sources(args: &Value, max_images: usize) -> Result<Vec<ImageSource>> {
    let mut sources = Vec::new();

    // 1. Check `images` array parameter
    if let Some(arr) = args.get("images").and_then(|v| v.as_array()) {
        for item in arr {
            let src_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("file");
            let label = item
                .get("label")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            match src_type {
                "file" => {
                    let path = item
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("images[].type='file' requires 'path'"))?;
                    sources.push(ImageSource::File {
                        path: path.to_string(),
                        label,
                    });
                }
                "url" => {
                    let url = item
                        .get("url")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("images[].type='url' requires 'url'"))?;
                    sources.push(ImageSource::Url {
                        url: url.to_string(),
                        label,
                    });
                }
                "clipboard" => {
                    #[cfg(feature = "desktop-tools")]
                    sources.push(ImageSource::Clipboard { label });
                    #[cfg(not(feature = "desktop-tools"))]
                    return Err(anyhow!(
                        "image source 'clipboard' is not available in this build (desktop-tools feature disabled — likely a headless / container deployment)"
                    ));
                }
                "screenshot" => {
                    #[cfg(feature = "desktop-tools")]
                    {
                        let monitor = item
                            .get("monitor")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize);
                        sources.push(ImageSource::Screenshot { monitor, label });
                    }
                    #[cfg(not(feature = "desktop-tools"))]
                    return Err(anyhow!(
                        "image source 'screenshot' is not available in this build (desktop-tools feature disabled — likely a headless / container deployment)"
                    ));
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
                label: None,
            });
        }
    }

    // 3. `url` shorthand
    if sources.is_empty() {
        if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
            sources.push(ImageSource::Url {
                url: url.to_string(),
                label: None,
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
                return Err(anyhow!("URL returned non-image content type: {}", ct_str));
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

    Ok((bytes.to_vec(), url_label(url)))
}

fn url_label(url: &str) -> String {
    if url.len() > 80 {
        format!("url: {}...", crate::truncate_utf8(url, 77))
    } else {
        format!("url: {}", url)
    }
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
#[cfg(feature = "desktop-tools")]
fn resolve_clipboard() -> Result<(Vec<u8>, String)> {
    use arboard::Clipboard;
    use image::RgbaImage;
    use std::io::Cursor;

    let mut clipboard =
        Clipboard::new().map_err(|e| anyhow!("Failed to access clipboard: {}", e))?;

    let img_data = clipboard
        .get_image()
        .map_err(|_| anyhow!("Clipboard does not contain an image"))?;

    let rgba = RgbaImage::from_raw(
        img_data.width as u32,
        img_data.height as u32,
        img_data.bytes.into_owned(),
    )
    .ok_or_else(|| anyhow!("Failed to create image from clipboard data"))?;

    let dyn_img = image::DynamicImage::ImageRgba8(rgba);

    let mut buf = Cursor::new(Vec::new());
    dyn_img
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| anyhow!("Failed to encode clipboard image as PNG: {}", e))?;

    let label = format!("clipboard ({}x{})", img_data.width, img_data.height);
    Ok((buf.into_inner(), label))
}

/// Capture a screenshot of the desktop.
#[cfg(feature = "desktop-tools")]
fn resolve_screenshot(monitor_index: Option<usize>) -> Result<(Vec<u8>, String)> {
    use std::io::Cursor;
    use xcap::Monitor;

    let monitors = Monitor::all().map_err(|e| anyhow!("Failed to list monitors: {}", e))?;
    if monitors.is_empty() {
        return Err(anyhow!("No monitors detected"));
    }

    let idx = monitor_index.unwrap_or(0);
    let monitor = monitors.get(idx).ok_or_else(|| {
        anyhow!(
            "Monitor index {} out of range (available: {})",
            idx,
            monitors.len()
        )
    })?;

    let rgba_image = monitor.capture_image().map_err(|e| {
        anyhow!(
            "Screenshot capture failed (may need Screen Recording permission): {}",
            e
        )
    })?;

    let (w, h) = (rgba_image.width(), rgba_image.height());
    let dyn_img = image::DynamicImage::ImageRgba8(rgba_image);

    let mut buf = Cursor::new(Vec::new());
    dyn_img
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| anyhow!("Failed to encode screenshot as PNG: {}", e))?;

    let label = format!("screenshot ({}x{}, monitor {})", w, h, idx);
    Ok((buf.into_inner(), label))
}

fn non_empty_string_field<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn vision_task(args: &Value) -> Option<&str> {
    non_empty_string_field(args, "task")
        .or_else(|| non_empty_string_field(args, "question"))
        // Backward-compatible alias. The tool no longer treats this as a
        // separate analyzer prompt; it is the task text attached to the vision
        // input for the next model round.
        .or_else(|| non_empty_string_field(args, "prompt"))
}

fn with_label(source_label: String, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("{label} - {source_label}"),
        None => source_label,
    }
}

// ── Main Tool Entry ──────────────────────────────────────────────────

/// Tool: image / vision input: attach one or more images from files, URLs,
/// clipboard, or screenshot to the next model round as visual input.
pub(crate) async fn tool_image(args: &Value) -> Result<String> {
    let sources = normalize_sources(args, effective_max_images())?;
    let task = vision_task(args);
    let total = sources.len();
    let mut result_parts: Vec<String> = Vec::new();
    let mut success_count = 0usize;

    if let Some(task) = task {
        result_parts.push(format!("Vision task: {}\n", task));
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
            ImageSource::File { path, .. } => resolve_file(path),
            ImageSource::Url { url, .. } => resolve_url(url).await,
            #[cfg(feature = "desktop-tools")]
            ImageSource::Clipboard { .. } => resolve_clipboard(),
            #[cfg(feature = "desktop-tools")]
            ImageSource::Screenshot { monitor, .. } => resolve_screenshot(*monitor),
        };

        match resolve_result {
            Ok((data, raw_source_label)) => {
                let source_label = with_label(raw_source_label, source.label());
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

#[cfg(test)]
mod tests {
    use super::{normalize_sources, url_label, vision_task, ImageSource};
    use serde_json::json;

    #[test]
    fn url_label_truncates_on_utf8_boundary() {
        let url = format!("https://example.com/{}", "图".repeat(40));
        let label = url_label(&url);
        assert!(std::str::from_utf8(label.as_bytes()).is_ok());
        assert!(label.ends_with("..."));
    }

    #[test]
    fn vision_task_prefers_task_then_question_then_legacy_prompt() {
        assert_eq!(
            vision_task(&json!({"task": "inspect layout"})),
            Some("inspect layout")
        );
        assert_eq!(
            vision_task(&json!({"question": "what is shown?"})),
            Some("what is shown?")
        );
        assert_eq!(
            vision_task(&json!({"prompt": "legacy prompt"})),
            Some("legacy prompt")
        );
        assert_eq!(
            vision_task(&json!({"task": "  ", "question": "fallback"})),
            Some("fallback")
        );
        assert_eq!(vision_task(&json!({"task": "  "})), None);
    }

    #[test]
    fn normalize_sources_preserves_optional_labels() {
        let sources = normalize_sources(
            &json!({
                "images": [
                    {"type": "file", "path": "/tmp/a.png", "label": "report page 1"},
                    {"type": "url", "url": "https://example.com/b.png", "label": "reference"}
                ]
            }),
            10,
        )
        .expect("sources");

        match &sources[0] {
            ImageSource::File { path, label } => {
                assert_eq!(path, "/tmp/a.png");
                assert_eq!(label.as_deref(), Some("report page 1"));
            }
            _ => panic!("expected file source"),
        }
        match &sources[1] {
            ImageSource::Url { url, label } => {
                assert_eq!(url, "https://example.com/b.png");
                assert_eq!(label.as_deref(), Some("reference"));
            }
            _ => panic!("expected url source"),
        }
    }
}
