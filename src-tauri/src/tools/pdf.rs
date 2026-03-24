use anyhow::Result;
use serde_json::Value;

use super::expand_tilde;

/// Default max characters to return from PDF extraction.
const DEFAULT_MAX_CHARS: usize = 50_000;

/// Tool: pdf — extract text content from PDF documents.
pub(crate) async fn tool_pdf(args: &Value) -> Result<String> {
    let path_raw = args.get("path")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    let pages_spec = args.get("pages")
        .and_then(|v| v.as_str());

    let max_chars = args.get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_CHARS as u64) as usize;

    let path = expand_tilde(path_raw);
    let file_path = std::path::Path::new(&path);

    if !file_path.exists() {
        return Ok(format!("Error: File not found: {}", path));
    }

    let file_size = std::fs::metadata(file_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Extract text
    let full_text = match pdf_extract::extract_text(file_path) {
        Ok(text) => text,
        Err(e) => {
            return Ok(format!(
                "PDF: {} ({} bytes)\n\nError: Failed to extract text: {}",
                path, file_size, e
            ));
        }
    };

    if full_text.trim().is_empty() {
        return Ok(format!(
            "PDF: {} ({} bytes)\n\nNo extractable text found. This PDF may contain only images or scanned content.",
            path, file_size
        ));
    }

    // Split text into pages using form-feed characters (\x0C) which pdf-extract inserts
    let raw_pages: Vec<&str> = full_text.split('\x0C').collect();
    let total_pages = raw_pages.len();

    // Parse page range filter
    let page_filter = if let Some(spec) = pages_spec {
        Some(parse_page_range(spec, total_pages)?)
    } else {
        None
    };

    // Build output
    let mut output = format!(
        "PDF: {} ({} pages, {} bytes)\n",
        path, total_pages, file_size,
    );

    let mut chars_written = output.len();
    let mut last_included_page = 0;
    let mut truncated = false;

    for (idx, page_text) in raw_pages.iter().enumerate() {
        let page_num = idx + 1; // 1-indexed

        // Apply page filter
        if let Some(ref filter) = page_filter {
            if !filter.contains(&page_num) {
                continue;
            }
        }

        let trimmed = page_text.trim();
        if trimmed.is_empty() {
            continue;
        }

        let header = format!("\n--- Page {} ---\n", page_num);
        let entry_len = header.len() + trimmed.len() + 1;

        if chars_written + entry_len > max_chars {
            truncated = true;
            // Write as much of this page as fits
            let remaining = max_chars.saturating_sub(chars_written + header.len() + 50);
            if remaining > 100 {
                output.push_str(&header);
                let partial: String = trimmed.chars().take(remaining).collect();
                output.push_str(&partial);
                output.push_str("...");
            }
            break;
        }

        output.push_str(&header);
        output.push_str(trimmed);
        output.push('\n');
        chars_written += entry_len;
        last_included_page = page_num;
    }

    if truncated {
        output.push_str(&format!(
            "\n\n[Output truncated at {} chars. Use pages=\"{}-{}\" to read remaining pages.]",
            max_chars,
            last_included_page + 1,
            total_pages,
        ));
    }

    Ok(output)
}

/// Parse a page range specification like "1-5", "3", "1-3,5,7-10".
/// Returns a set of 1-indexed page numbers.
fn parse_page_range(spec: &str, total_pages: usize) -> Result<Vec<usize>> {
    let mut pages = Vec::new();

    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((start_str, end_str)) = part.split_once('-') {
            let start: usize = start_str.trim().parse()
                .map_err(|_| anyhow::anyhow!("Invalid page range: '{}'", part))?;
            let end: usize = end_str.trim().parse()
                .map_err(|_| anyhow::anyhow!("Invalid page range: '{}'", part))?;

            if start == 0 || end == 0 || start > end {
                return Err(anyhow::anyhow!("Invalid page range: '{}' (pages are 1-indexed)", part));
            }

            for p in start..=end.min(total_pages) {
                if !pages.contains(&p) {
                    pages.push(p);
                }
            }
        } else {
            let p: usize = part.parse()
                .map_err(|_| anyhow::anyhow!("Invalid page number: '{}'", part))?;
            if p == 0 {
                return Err(anyhow::anyhow!("Page numbers are 1-indexed, got 0"));
            }
            if p <= total_pages && !pages.contains(&p) {
                pages.push(p);
            }
        }
    }

    if pages.is_empty() {
        return Err(anyhow::anyhow!("No valid pages in range '{}' (document has {} pages)", spec, total_pages));
    }

    pages.sort();
    Ok(pages)
}
