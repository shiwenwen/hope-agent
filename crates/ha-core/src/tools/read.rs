use anyhow::Result;
use serde_json::Value;

use super::extract_string_param;

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
/// Conservative upper bound for decoder + resize working buffers. Dimension
/// headers are inspected before decode so a tiny compressed image cannot force
/// an unbounded pixel allocation through `read_context_resource`.
const IMAGE_MAX_DECODE_WORKING_BYTES: usize = 128 * 1024 * 1024;

fn inspect_image_for_bounded_decode(data: &[u8]) -> Result<(u32, u32, usize)> {
    use image::ImageReader;
    use std::io::Cursor;

    let (width, height) = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("Cannot detect image format: {}", e))?
        .into_dimensions()
        .map_err(|e| anyhow::anyhow!("Cannot inspect image dimensions: {}", e))?;
    let working_bytes = usize::try_from(width)
        .ok()
        .and_then(|value| {
            usize::try_from(height)
                .ok()
                .and_then(|height| value.checked_mul(height))
        })
        // DynamicImage variants, decoder scratch, and resize filters can keep
        // several pixel buffers live. Sixteen bytes/pixel is conservative for
        // the formats accepted by image-rs.
        .and_then(|pixels| pixels.checked_mul(16))
        .ok_or_else(|| anyhow::anyhow!("Image dimensions exceed the bounded decode budget"))?;
    if working_bytes > IMAGE_MAX_DECODE_WORKING_BYTES {
        anyhow::bail!("Image dimensions exceed the bounded decode budget");
    }
    Ok((width, height, working_bytes))
}

/// Upper bounds needed before a frozen context image allocates its decoded or
/// Base64 result buffers. The output length is exact for pass-through images
/// and the configured 5 MiB ceiling for a resized JPEG.
pub(crate) fn bounded_image_delivery_projection(data: &[u8]) -> Result<(usize, usize)> {
    let (width, height, working_bytes) = inspect_image_for_bounded_decode(data)?;
    let output_raw = if width > IMAGE_MAX_DIMENSION
        || height > IMAGE_MAX_DIMENSION
        || data.len() > IMAGE_MAX_BYTES
    {
        IMAGE_MAX_BYTES
    } else {
        data.len()
    };
    let encoded_upper = output_raw
        .checked_add(2)
        .and_then(|value| value.checked_div(3))
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("Image Base64 projection overflow"))?;
    let transient_bytes = working_bytes
        .checked_add(output_raw)
        .and_then(|value| value.checked_add(encoded_upper))
        .ok_or_else(|| anyhow::anyhow!("Image delivery projection overflow"))?;
    Ok((encoded_upper, transient_bytes))
}

/// Resize an image buffer if it exceeds dimension or byte limits.
/// Returns (base64_data, mime_type).
pub(crate) fn resize_image_if_needed(
    data: &[u8],
    original_mime: &str,
) -> Result<(String, &'static str)> {
    use image::ImageReader;
    use std::io::Cursor;

    let (w, h, _) = inspect_image_for_bounded_decode(data)?;
    let needs_resize =
        w > IMAGE_MAX_DIMENSION || h > IMAGE_MAX_DIMENSION || data.len() > IMAGE_MAX_BYTES;

    // Decode even pass-through images after the dimension preflight. Returning
    // a truncated/corrupt compressed payload merely postpones the failure to a
    // provider and makes a successful read result misleading.
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("Cannot detect image format: {}", e))?
        .decode()
        .map_err(|e| anyhow::anyhow!("Cannot decode image: {}", e))?;

    if !needs_resize {
        drop(img);
        // Return original data as base64
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
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
            let b64 =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes);
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

/// Page output must fit Tier 1's exact C0, including its continuation cursor.
/// Advancing a cursor over a larger page would make an omitted middle range
/// unreachable while durable ResultStore readback is disabled.
const DEFAULT_READ_PAGE_MAX_BYTES: usize = crate::session::MAX_RESUMABLE_TOOL_PAGE_BYTES;
const MAX_ADAPTIVE_READ_MAX_BYTES: usize = crate::session::MAX_RESUMABLE_TOOL_PAGE_BYTES;
const READ_PAGE_CURSOR_RESERVE_BYTES: usize = 768;
/// Max pages for adaptive paging.
const MAX_ADAPTIVE_READ_PAGES: usize = 8;
/// Default max lines per page when no limit is specified.
const READ_DEFAULT_MAX_LINES: usize = 2000;

/// Compute the safe inline page size.
///
/// The downstream admission ceiling is currently the tighter invariant, so
/// context-window inputs cannot raise this value. Keeping the parameters in
/// the signature preserves the caller contract for the future ResultStore
/// phase where durable readback can safely support larger adaptive pages.
fn compute_adaptive_read_max_bytes(
    _context_window_tokens: Option<u32>,
    _used_tokens: Option<u32>,
) -> usize {
    DEFAULT_READ_PAGE_MAX_BYTES.min(MAX_ADAPTIVE_READ_MAX_BYTES)
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

#[derive(Debug)]
struct TextPage {
    output: String,
    lines_touched: usize,
    complete_lines: usize,
    truncated: bool,
    total_lines: usize,
    next_line_index: usize,
    next_byte_offset: usize,
}

/// Read a text page with both a line ceiling and a hard byte ceiling.
///
/// The old pager bounded only the number of lines, so a minified JSON document
/// or another single-line payload could return tens of megabytes and crowd out
/// the model context. `start_byte_offset` makes that line resumable without
/// cutting UTF-8 code points.
fn read_text_page_bounded(
    lines: &[&str],
    start_idx: usize,
    start_byte_offset: usize,
    max_lines: usize,
    max_bytes: usize,
) -> TextPage {
    let total_lines = lines.len();
    let mut line_index = start_idx.min(total_lines);
    let mut byte_offset = start_byte_offset;
    let mut output = String::new();
    let mut lines_touched = 0usize;
    let mut complete_lines = 0usize;

    while line_index < total_lines && lines_touched < max_lines {
        let line = lines[line_index];
        let mut safe_offset = byte_offset.min(line.len());
        while safe_offset > 0 && !line.is_char_boundary(safe_offset) {
            safe_offset -= 1;
        }

        if safe_offset == line.len() && !line.is_empty() {
            line_index += 1;
            byte_offset = 0;
            continue;
        }

        let prefix = format!("{:6}\t", line_index + 1);
        let required_overhead = prefix.len() + 1; // trailing newline
        if output.len().saturating_add(required_overhead) >= max_bytes {
            break;
        }
        let available = max_bytes - output.len() - required_overhead;
        let remaining = &line[safe_offset..];
        let take = crate::truncate_utf8(remaining, available).len();
        if take == 0 && !remaining.is_empty() {
            break;
        }

        output.push_str(&prefix);
        output.push_str(&remaining[..take]);
        output.push('\n');
        lines_touched += 1;

        if take < remaining.len() {
            byte_offset = safe_offset + take;
            return TextPage {
                output,
                lines_touched,
                complete_lines,
                truncated: true,
                total_lines,
                next_line_index: line_index,
                next_byte_offset: byte_offset,
            };
        }

        complete_lines += 1;
        line_index += 1;
        byte_offset = 0;
    }

    TextPage {
        output,
        lines_touched,
        complete_lines,
        truncated: line_index < total_lines,
        total_lines,
        next_line_index: line_index,
        next_byte_offset: byte_offset,
    }
}

pub(crate) async fn tool_read_file(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    // Accept both "path" and "file_path", with structured content support
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let path = ctx.resolve_path(raw_path);

    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v.max(1) as usize)
        .unwrap_or(1); // 1-based

    let explicit_limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.max(1) as usize);
    let byte_offset = args
        .get("byte_offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);

    app_info!(
        "tool",
        "read",
        "Reading file: {} (offset={}, byte_offset={}, limit={:?})",
        path,
        offset,
        byte_offset,
        explicit_limit
    );

    // Check file size before reading to prevent memory exhaustion
    const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to stat file '{}': {}", path, e))?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(anyhow::anyhow!(
            "File '{}' is too large ({:.1} MB, max {} MB). Use a streaming approach or read specific sections.",
            path,
            metadata.len() as f64 / 1_048_576.0,
            MAX_FILE_SIZE / 1_048_576
        ));
    }

    // Read raw bytes first to detect file type
    let data = tokio::fs::read(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    // Check if file is an image via magic bytes
    let mime = detect_image_mime(&data);
    if let Some(mime_type) = mime {
        app_info!(
            "tool",
            "read",
            "Detected image file: {} ({})",
            path,
            mime_type
        );
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

    let max_bytes = compute_adaptive_read_max_bytes(ctx.context_window_tokens, ctx.used_tokens);

    // If user specified an explicit limit, use a single bounded page. `limit`
    // remains a line ceiling; the byte ceiling is mandatory and supplies a
    // resumable byte cursor for pathological long lines.
    if let Some(limit) = explicit_limit {
        let content_budget = max_bytes
            .saturating_sub(READ_PAGE_CURSOR_RESERVE_BYTES)
            .max(512);
        let mut page =
            read_text_page_bounded(&lines, offset - 1, byte_offset, limit, content_budget);
        let mut result = std::mem::take(&mut page.output);
        append_text_page_cursor(&mut result, &page, max_bytes);
        emit_file_read_metadata(ctx, &path, total_lines).await;
        return Ok(result);
    }

    // Adaptive paging: auto-aggregate multiple pages up to max_bytes budget
    app_debug!(
        "tool",
        "read",
        "Adaptive read budget: {}KB (window={}K, used={}K, remaining={}K)",
        max_bytes / 1024,
        ctx.context_window_tokens.unwrap_or(0) / 1000,
        ctx.used_tokens.unwrap_or(0) / 1000,
        ctx.context_window_tokens
            .unwrap_or(0)
            .saturating_sub(ctx.used_tokens.unwrap_or(0))
            / 1000
    );
    let content_budget = max_bytes
        .saturating_sub(READ_PAGE_CURSOR_RESERVE_BYTES)
        .max(512);
    let mut page = read_text_page_bounded(
        &lines,
        offset - 1,
        byte_offset,
        READ_DEFAULT_MAX_LINES * MAX_ADAPTIVE_READ_PAGES,
        content_budget,
    );
    let mut aggregated = std::mem::take(&mut page.output);
    append_text_page_cursor(&mut aggregated, &page, max_bytes);

    emit_file_read_metadata(ctx, &path, total_lines).await;

    Ok(aggregated)
}

fn append_text_page_cursor(output: &mut String, page: &TextPage, max_bytes: usize) {
    if !page.truncated {
        return;
    }

    if page.next_byte_offset > 0 {
        output.push_str(&format!(
            "\n[Read part of line {} of {} ({} bytes returned). Output capped at ~{}KB. Use offset={}, byte_offset={} to continue.]\n",
            page.next_line_index + 1,
            page.total_lines,
            output.len(),
            max_bytes / 1024,
            page.next_line_index + 1,
            page.next_byte_offset,
        ));
    } else {
        let first_line = page
            .next_line_index
            .saturating_sub(page.complete_lines)
            .saturating_add(1);
        output.push_str(&format!(
            "\n[Read {} lines ({}–{} of {}, {} bytes). Output capped at ~{}KB. Use offset={} to continue.]\n",
            page.lines_touched,
            first_line,
            page.next_line_index,
            page.total_lines,
            output.len(),
            max_bytes / 1024,
            page.next_line_index + 1,
        ));
    }
}

async fn emit_file_read_metadata(ctx: &super::ToolExecContext, path: &str, total_lines: usize) {
    if ctx.metadata_sink.is_none() {
        return;
    }
    ctx.emit_metadata(serde_json::json!({
        "kind": "file_read",
        "path": path,
        "lines": total_lines as u32,
    }))
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_page_returns_cursor_for_one_huge_utf8_line() {
        let line = "中".repeat(10_000);
        let lines = vec![line.as_str()];
        let first = read_text_page_bounded(&lines, 0, 0, 10, 1_024);

        assert!(first.truncated);
        assert_eq!(first.next_line_index, 0);
        assert!(first.next_byte_offset > 0);
        assert!(line.is_char_boundary(first.next_byte_offset));
        assert!(first.output.len() <= 1_024);

        let second = read_text_page_bounded(
            &lines,
            first.next_line_index,
            first.next_byte_offset,
            10,
            1_024,
        );
        assert!(second.next_byte_offset > first.next_byte_offset);
        assert!(line.is_char_boundary(second.next_byte_offset));
        let (_, second_payload) = second
            .output
            .split_once('\t')
            .expect("page output has a line-number prefix");
        let second_payload = second_payload
            .strip_suffix('\n')
            .expect("page output ends with a newline");
        assert_eq!(
            second_payload,
            &line[first.next_byte_offset..second.next_byte_offset]
        );
    }

    #[test]
    fn bounded_text_page_keeps_empty_lines_and_line_cursor() {
        let lines = vec!["a", "", "b", "c"];
        let page = read_text_page_bounded(&lines, 0, 0, 3, 4_096);

        assert!(page.truncated);
        assert_eq!(page.lines_touched, 3);
        assert_eq!(page.complete_lines, 3);
        assert_eq!(page.next_line_index, 3);
        assert_eq!(page.next_byte_offset, 0);
        assert!(page.output.contains("     2\t\n"));
    }

    #[test]
    fn pageable_read_never_exceeds_downstream_inline_projection() {
        let line = "x".repeat(DEFAULT_READ_PAGE_MAX_BYTES * 2);
        let lines = vec![line.as_str()];
        let content_budget =
            DEFAULT_READ_PAGE_MAX_BYTES.saturating_sub(READ_PAGE_CURSOR_RESERVE_BYTES);
        let mut page = read_text_page_bounded(&lines, 0, 0, 10, content_budget);
        let mut output = std::mem::take(&mut page.output);
        append_text_page_cursor(&mut output, &page, DEFAULT_READ_PAGE_MAX_BYTES);

        assert!(page.truncated);
        assert!(output.len() <= crate::session::MAX_RESUMABLE_TOOL_PAGE_BYTES);
        assert!(output.contains("byte_offset="));
    }
}
