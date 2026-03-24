use anyhow::Result;
use serde_json::Value;

use super::{expand_tilde, extract_string_param};

// ── Image Detection & Resize ──────────────────────────────────────

/// Known image MIME types detected by magic bytes.
pub(crate) fn detect_image_mime(header: &[u8]) -> Option<&'static str> {
    if header.len() < 4 {
        return None;
    }
    // PNG: 89 50 4E 47
    if header.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some("image/png");
    }
    // JPEG: FF D8 FF
    if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    // GIF: GIF87a or GIF89a
    if header.starts_with(b"GIF8") {
        return Some("image/gif");
    }
    // WebP: RIFF....WEBP
    if header.len() >= 12 && header.starts_with(b"RIFF") && &header[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // BMP: BM
    if header.starts_with(b"BM") {
        return Some("image/bmp");
    }
    // ICO: 00 00 01 00
    if header.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        return Some("image/x-icon");
    }
    // TIFF: II (little-endian) or MM (big-endian)
    if header.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || header.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
    {
        return Some("image/tiff");
    }
    None
}

/// Max dimension (width or height) for images sent to LLM.
const IMAGE_MAX_DIMENSION: u32 = 1200;
/// Max bytes for base64-encoded image payload.
const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Resize an image buffer if it exceeds dimension or byte limits.
/// Returns (base64_data, mime_type).
fn resize_image_if_needed(data: &[u8], original_mime: &str) -> Result<(String, &'static str)> {
    use image::ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("Cannot detect image format: {}", e))?;
    let img = reader
        .decode()
        .map_err(|e| anyhow::anyhow!("Cannot decode image: {}", e))?;

    let (w, h) = (img.width(), img.height());
    let needs_resize =
        w > IMAGE_MAX_DIMENSION || h > IMAGE_MAX_DIMENSION || data.len() > IMAGE_MAX_BYTES;

    if !needs_resize {
        // Return original data as base64
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
        // Keep original mime, but map to static str
        let mime: &'static str = match original_mime {
            "image/png" => "image/png",
            "image/gif" => "image/gif",
            "image/webp" => "image/webp",
            "image/bmp" => "image/bmp",
            "image/tiff" => "image/tiff",
            "image/x-icon" => "image/x-icon",
            _ => "image/jpeg",
        };
        return Ok((b64, mime));
    }

    // Resize to fit within IMAGE_MAX_DIMENSION, preserving aspect ratio
    let resized = img.resize(
        IMAGE_MAX_DIMENSION,
        IMAGE_MAX_DIMENSION,
        image::imageops::FilterType::Lanczos3,
    );

    // Encode as JPEG with quality steps
    for quality in [85u8, 70, 50] {
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        resized
            .write_with_encoder(encoder)
            .map_err(|e| anyhow::anyhow!("Failed to encode resized image: {}", e))?;
        let jpeg_bytes = buf.into_inner();
        if jpeg_bytes.len() <= IMAGE_MAX_BYTES {
            let b64 = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &jpeg_bytes,
            );
            return Ok((b64, "image/jpeg"));
        }
    }

    Err(anyhow::anyhow!(
        "Image too large: could not reduce below {}MB (original {}x{}, {} bytes)",
        IMAGE_MAX_BYTES / 1024 / 1024,
        w,
        h,
        data.len()
    ))
}

// ── Read Constants ────────────────────────────────────────────────

/// Default max bytes for a single read page (50KB).
const DEFAULT_READ_PAGE_MAX_BYTES: usize = 50 * 1024;
/// Max bytes for adaptive read (512KB).
const MAX_ADAPTIVE_READ_MAX_BYTES: usize = 512 * 1024;
/// Share of model context window to use for read output (20%).
const ADAPTIVE_READ_CONTEXT_SHARE: f64 = 0.2;
/// Estimated chars per token.
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
/// Max pages for adaptive paging.
const MAX_ADAPTIVE_READ_PAGES: usize = 8;
/// Default max lines per page when no limit is specified.
const READ_DEFAULT_MAX_LINES: usize = 2000;

/// Compute max bytes for a single adaptive read page based on model context window.
fn compute_adaptive_read_max_bytes(context_window_tokens: Option<u32>) -> usize {
    match context_window_tokens {
        Some(tokens) if tokens > 0 => {
            let from_context = (tokens as usize)
                * CHARS_PER_TOKEN_ESTIMATE
                * (ADAPTIVE_READ_CONTEXT_SHARE * 100.0) as usize
                / 100;
            from_context.clamp(DEFAULT_READ_PAGE_MAX_BYTES, MAX_ADAPTIVE_READ_MAX_BYTES)
        }
        _ => DEFAULT_READ_PAGE_MAX_BYTES,
    }
}

/// Verify base64 image data's actual MIME type by decoding first 192 bytes and re-sniffing magic bytes.
fn verify_base64_mime(b64: &str, declared_mime: &str) -> &'static str {
    // Decode first 256 base64 chars (aligned to 4)
    let take = b64.len().min(256);
    let slice_len = take - (take % 4);
    if slice_len < 8 {
        return match declared_mime {
            "image/png" => "image/png",
            "image/gif" => "image/gif",
            "image/webp" => "image/webp",
            "image/bmp" => "image/bmp",
            "image/tiff" => "image/tiff",
            "image/x-icon" => "image/x-icon",
            _ => "image/jpeg",
        };
    }

    if let Ok(head) = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &b64[..slice_len],
    ) {
        if let Some(sniffed) = detect_image_mime(&head) {
            return sniffed;
        }
    }

    // Fallback to declared
    match declared_mime {
        "image/png" => "image/png",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/bmp" => "image/bmp",
        "image/tiff" => "image/tiff",
        "image/x-icon" => "image/x-icon",
        _ => "image/jpeg",
    }
}

/// Read a single page of a text file. Returns (output_text, lines_read, truncated, total_lines).
fn read_text_page(
    lines: &[&str],
    start_idx: usize,
    max_lines: usize,
) -> (String, usize, bool, usize) {
    let total_lines = lines.len();
    let start = start_idx.min(total_lines);
    let end = (start + max_lines).min(total_lines);
    let selected = &lines[start..end];

    let mut output = String::new();
    for (i, line) in selected.iter().enumerate() {
        let line_num = start + i + 1;
        output.push_str(&format!("{:6}\t{}\n", line_num, line));
    }

    let truncated = end < total_lines;
    (output, selected.len(), truncated, total_lines)
}

pub(crate) async fn tool_read_file(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    // Accept both "path" and "file_path", with structured content support
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let path = expand_tilde(raw_path);

    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v.max(1) as usize)
        .unwrap_or(1); // 1-based

    let explicit_limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    app_info!("tool", "read",
        "Reading file: {} (offset={}, limit={:?})",
        path,
        offset,
        explicit_limit
    );

    // Read raw bytes first to detect file type
    let data = tokio::fs::read(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    // Check if file is an image via magic bytes
    let mime = detect_image_mime(&data);
    if let Some(mime_type) = mime {
        app_info!("tool", "read", "Detected image file: {} ({})", path, mime_type);
        match resize_image_if_needed(&data, mime_type) {
            Ok((b64, declared_mime)) => {
                // Secondary MIME verification: decode base64 header and re-sniff
                let verified_mime = verify_base64_mime(&b64, declared_mime);
                return Ok(format!(
                    "Read image file [{}] ({} bytes, {})\nbase64:{}\n",
                    verified_mime,
                    data.len(),
                    path,
                    b64
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Image file '{}' detected as {} but cannot be processed: {}",
                    path,
                    mime_type,
                    e
                ));
            }
        }
    }

    // Text file — convert to string
    let content = String::from_utf8(data)
        .map_err(|_| anyhow::anyhow!("File '{}' contains invalid UTF-8 (binary file?)", path))?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // If user specified an explicit limit, use single-page mode (no adaptive paging)
    if let Some(limit) = explicit_limit {
        let (output, lines_read, truncated, _) = read_text_page(&lines, offset - 1, limit);
        let mut result = output;
        if truncated {
            result.push_str(&format!(
                "\n[Read {} lines ({}–{} of {}). Use offset={} to continue reading.]\n",
                lines_read,
                offset,
                offset - 1 + lines_read,
                total_lines,
                offset + lines_read
            ));
        }
        return Ok(result);
    }

    // Adaptive paging: auto-aggregate multiple pages up to max_bytes budget
    let max_bytes = compute_adaptive_read_max_bytes(ctx.context_window_tokens);
    let page_max_lines = READ_DEFAULT_MAX_LINES;
    let mut aggregated = String::new();
    let mut aggregated_bytes: usize = 0;
    let mut next_offset = offset - 1; // convert to 0-based
    let mut capped = false;

    for _page in 0..MAX_ADAPTIVE_READ_PAGES {
        if next_offset >= total_lines {
            break;
        }

        let (page_text, lines_read, truncated, _) =
            read_text_page(&lines, next_offset, page_max_lines);

        if lines_read == 0 {
            break;
        }

        let page_bytes = page_text.len();

        // Check if adding this page would exceed budget (skip check for first page)
        if !aggregated.is_empty() && aggregated_bytes + page_bytes > max_bytes {
            capped = true;
            break;
        }

        aggregated.push_str(&page_text);
        aggregated_bytes += page_bytes;
        next_offset += lines_read;

        if !truncated {
            // Reached end of file
            break;
        }
    }

    // Add truncation/continuation notice
    if next_offset < total_lines {
        aggregated.push_str(&format!(
            "\n[Read lines {}–{} of {} ({} bytes). {}Use offset={} to continue reading.]\n",
            offset,
            next_offset,
            total_lines,
            aggregated_bytes,
            if capped {
                format!("Output capped at ~{}KB for this call. ", max_bytes / 1024)
            } else {
                String::new()
            },
            next_offset + 1
        ));
    }

    Ok(aggregated)
}
