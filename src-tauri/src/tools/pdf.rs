use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::browser::IMAGE_BASE64_PREFIX;
use super::expand_tilde;

/// Default max characters to return from text extraction.
const DEFAULT_MAX_CHARS: usize = 50_000;
/// Default maximum number of PDFs per single tool call.
const DEFAULT_MAX_PDFS: usize = 5;
/// Hard cap on max PDFs (user cannot exceed this).
const CAP_MAX_PDFS: usize = 10;
/// Default maximum pages to render in vision mode.
const DEFAULT_MAX_VISION_PAGES: usize = 10;
/// Render width for vision mode (pixels).
const VISION_RENDER_WIDTH: u32 = 1200;
/// HTTP timeout for fetching remote PDFs.
const FETCH_TIMEOUT_SECS: u64 = 30;
/// Maximum bytes to download for a remote PDF (20 MB).
const PDF_MAX_FETCH_BYTES: usize = 20 * 1024 * 1024;
/// If text extraction yields fewer chars than this, auto mode switches to vision.
const AUTO_VISION_THRESHOLD: usize = 200;

// ── PDF Tool Config ─────────────────────────────────────────────

/// Persistent PDF tool configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToolConfig {
    /// Maximum number of PDFs per single tool call
    #[serde(default = "default_max_pdfs")]
    pub max_pdfs: usize,
    /// Maximum pages to render in vision mode
    #[serde(default = "default_max_vision_pages")]
    pub max_vision_pages: usize,
}

fn default_max_pdfs() -> usize {
    DEFAULT_MAX_PDFS
}
fn default_max_vision_pages() -> usize {
    DEFAULT_MAX_VISION_PAGES
}

impl Default for PdfToolConfig {
    fn default() -> Self {
        Self {
            max_pdfs: DEFAULT_MAX_PDFS,
            max_vision_pages: DEFAULT_MAX_VISION_PAGES,
        }
    }
}

// ── PDF Source Types ────────────────────────────────────────────────

enum PdfSource {
    File { path: String },
    Url { url: String },
}

/// Load PDF tool config from the global config store, clamped to hard caps.
fn load_pdf_config() -> PdfToolConfig {
    let mut cfg = crate::provider::load_store()
        .map(|s| s.pdf)
        .unwrap_or_default();
    cfg.max_pdfs = cfg.max_pdfs.min(CAP_MAX_PDFS);
    cfg
}

/// Parse tool arguments into a list of PDF sources.
fn normalize_pdf_sources(args: &Value, max_pdfs: usize) -> Result<Vec<PdfSource>> {
    let mut sources = Vec::new();

    // 1. Check `pdfs` array
    if let Some(arr) = args.get("pdfs").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(obj) = item.as_object() {
                let src_type = obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file");
                match src_type {
                    "file" => {
                        if let Some(p) = obj.get("path").and_then(|v| v.as_str()) {
                            sources.push(PdfSource::File {
                                path: p.to_string(),
                            });
                        }
                    }
                    "url" => {
                        if let Some(u) = obj.get("url").and_then(|v| v.as_str()) {
                            sources.push(PdfSource::Url {
                                url: u.to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            } else if let Some(s) = item.as_str() {
                // Bare string: auto-detect file vs url
                if s.starts_with("http://") || s.starts_with("https://") {
                    sources.push(PdfSource::Url { url: s.to_string() });
                } else {
                    sources.push(PdfSource::File {
                        path: s.to_string(),
                    });
                }
            }
        }
    }

    // 2. Shorthand: `path`
    if sources.is_empty() {
        if let Some(p) = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
        {
            sources.push(PdfSource::File {
                path: p.to_string(),
            });
        }
    }

    // 3. Shorthand: `url`
    if sources.is_empty() {
        if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
            sources.push(PdfSource::Url {
                url: u.to_string(),
            });
        }
    }

    if sources.is_empty() {
        return Err(anyhow!(
            "Missing PDF source. Provide 'path', 'url', or 'pdfs' array."
        ));
    }

    if sources.len() > max_pdfs {
        return Err(anyhow!(
            "Too many PDFs: {} provided, maximum is {}",
            sources.len(),
            max_pdfs
        ));
    }

    Ok(sources)
}

// ── PDF Resolution ──────────────────────────────────────────────────

/// Load PDF bytes from a local file.
fn resolve_file(path_raw: &str) -> Result<(Vec<u8>, String)> {
    let path = expand_tilde(path_raw);
    let file_path = std::path::Path::new(&path);
    if !file_path.exists() {
        return Err(anyhow!("File not found: {}", path));
    }
    let data = std::fs::read(file_path)?;
    Ok((data, format!("file: {}", path)))
}

/// Fetch PDF bytes from a URL.
async fn resolve_url(url: &str) -> Result<(Vec<u8>, String)> {
    crate::tools::web_fetch::check_ssrf_safe(url).await?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()?;

    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {} fetching {}", status, url));
    }

    let bytes = resp.bytes().await?;
    if bytes.len() > PDF_MAX_FETCH_BYTES {
        return Err(anyhow!(
            "PDF too large: {} bytes (max {}MB)",
            bytes.len(),
            PDF_MAX_FETCH_BYTES / 1024 / 1024
        ));
    }

    // Validate it looks like a PDF
    if bytes.len() < 5 || &bytes[..5] != b"%PDF-" {
        return Err(anyhow!("URL did not return a valid PDF file"));
    }

    Ok((bytes.to_vec(), format!("url: {}", url)))
}

/// Resolve a PDF source to raw bytes.
async fn resolve_source(source: &PdfSource) -> Result<(Vec<u8>, String)> {
    match source {
        PdfSource::File { path } => resolve_file(path),
        PdfSource::Url { url } => resolve_url(url).await,
    }
}

// ── Text Extraction ─────────────────────────────────────────────────

/// Extract text from PDF bytes, returning per-page text.
fn extract_text_from_bytes(data: &[u8]) -> Result<Vec<String>> {
    // pdf_extract works on files, write to a temp file
    let tmp = std::env::temp_dir().join(format!("oc_pdf_{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, data)?;
    let result = pdf_extract::extract_text(&tmp);
    let _ = std::fs::remove_file(&tmp);

    let full_text = result.map_err(|e| anyhow!("Text extraction failed: {}", e))?;
    let pages: Vec<String> = full_text
        .split('\x0C')
        .map(|p| p.trim().to_string())
        .collect();
    Ok(pages)
}

/// Build text-mode output from page texts.
fn build_text_output(
    pages: &[String],
    page_filter: Option<&[usize]>,
    max_chars: usize,
    source_label: &str,
    file_size: usize,
) -> String {
    let total_pages = pages.len();
    let mut output = format!(
        "PDF: {} ({} pages, {} bytes)\n",
        source_label, total_pages, file_size,
    );

    let mut chars_written = output.len();
    let mut last_included_page = 0;
    let mut truncated = false;

    for (idx, page_text) in pages.iter().enumerate() {
        let page_num = idx + 1;

        if let Some(filter) = page_filter {
            if !filter.contains(&page_num) {
                continue;
            }
        }

        if page_text.is_empty() {
            continue;
        }

        let header = format!("\n--- Page {} ---\n", page_num);
        let entry_len = header.len() + page_text.len() + 1;

        if chars_written + entry_len > max_chars {
            truncated = true;
            let remaining = max_chars.saturating_sub(chars_written + header.len() + 50);
            if remaining > 100 {
                output.push_str(&header);
                let partial: String = page_text.chars().take(remaining).collect();
                output.push_str(&partial);
                output.push_str("...");
            }
            break;
        }

        output.push_str(&header);
        output.push_str(page_text);
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

    output
}

// ── Vision Rendering ────────────────────────────────────────────────

/// Render PDF pages to `__IMAGE_BASE64__` markers for LLM vision.
fn build_vision_output(
    data: &[u8],
    page_filter: Option<&[usize]>,
    source_label: &str,
    file_size: usize,
    max_vision_pages: usize,
) -> String {
    // Convert 1-indexed page filter to 0-indexed for render_pdf_bytes
    let indices: Option<Vec<usize>> = page_filter.map(|filter| {
        filter.iter().map(|&p| p.saturating_sub(1)).collect()
    });

    match crate::file_extract::render_pdf_bytes(
        data,
        indices.as_deref(),
        max_vision_pages,
        VISION_RENDER_WIDTH,
    ) {
        Ok((total_pages, rendered)) => {
            if rendered.is_empty() {
                return format!(
                    "PDF: {} ({} pages, {} bytes)\n\nFailed to render any pages as images.",
                    source_label, total_pages, file_size
                );
            }

            let mut parts = Vec::new();
            parts.push(format!(
                "PDF: {} ({} pages, {} bytes) — {} page(s) rendered as images",
                source_label,
                total_pages,
                file_size,
                rendered.len()
            ));

            for (page_num, b64) in &rendered {
                parts.push(format!(
                    "{}image/png__{}__\n[Page {}/{}] {}",
                    IMAGE_BASE64_PREFIX, b64, page_num, total_pages, source_label,
                ));
            }

            parts.join("\n\n")
        }
        Err(e) => {
            format!(
                "PDF: {} ({} bytes)\n\nVision rendering failed: {}. Try mode=\"text\" for text extraction.",
                source_label, file_size, e
            )
        }
    }
}

// ── Main Entry Point ────────────────────────────────────────────────

/// Tool: pdf — extract text or render pages as images from PDF documents.
pub(crate) async fn tool_pdf(args: &Value) -> Result<String> {
    let config = load_pdf_config();

    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");

    let pages_spec = args.get("pages").and_then(|v| v.as_str());
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_CHARS as u64) as usize;

    let sources = normalize_pdf_sources(args, config.max_pdfs)?;

    let mut result_parts = Vec::new();

    for (idx, source) in sources.iter().enumerate() {
        let label_prefix = if sources.len() > 1 {
            format!("[PDF {}/{}] ", idx + 1, sources.len())
        } else {
            String::new()
        };

        let (data, source_label) = match resolve_source(source).await {
            Ok(v) => v,
            Err(e) => {
                result_parts.push(format!("{}Error: {}", label_prefix, e));
                continue;
            }
        };

        let file_size = data.len();

        // Parse page range
        let page_filter = if let Some(spec) = pages_spec {
            // Need total pages to validate range — quick count via text extraction page count
            // or pdfium. Use a reasonable upper bound.
            Some(parse_page_range(spec, 9999)?)
        } else {
            None
        };

        let part = match mode {
            "text" => {
                match extract_text_from_bytes(&data) {
                    Ok(pages) => build_text_output(
                        &pages,
                        page_filter.as_deref(),
                        max_chars,
                        &source_label,
                        file_size,
                    ),
                    Err(e) => format!(
                        "{}PDF: {} ({} bytes)\n\nError: {}",
                        label_prefix, source_label, file_size, e
                    ),
                }
            }
            "vision" => {
                build_vision_output(&data, page_filter.as_deref(), &source_label, file_size, config.max_vision_pages)
            }
            _ => {
                // auto: try text first, fall back to vision if sparse
                match extract_text_from_bytes(&data) {
                    Ok(pages) => {
                        let total_text: usize = pages.iter().map(|p| p.len()).sum();
                        if total_text < AUTO_VISION_THRESHOLD {
                            // Sparse text — likely scanned/image PDF, use vision
                            build_vision_output(
                                &data,
                                page_filter.as_deref(),
                                &source_label,
                                file_size,
                                config.max_vision_pages,
                            )
                        } else {
                            build_text_output(
                                &pages,
                                page_filter.as_deref(),
                                max_chars,
                                &source_label,
                                file_size,
                            )
                        }
                    }
                    Err(_) => {
                        // Text extraction failed, try vision
                        build_vision_output(
                            &data,
                            page_filter.as_deref(),
                            &source_label,
                            file_size,
                            config.max_vision_pages,
                        )
                    }
                }
            }
        };

        if label_prefix.is_empty() {
            result_parts.push(part);
        } else {
            result_parts.push(format!("{}{}", label_prefix, part));
        }
    }

    Ok(result_parts.join("\n\n"))
}

/// Parse a page range specification like "1-5", "3", "1-3,5,7-10".
/// Returns a vec of 1-indexed page numbers.
fn parse_page_range(spec: &str, total_pages: usize) -> Result<Vec<usize>> {
    let mut pages = Vec::new();

    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((start_str, end_str)) = part.split_once('-') {
            let start: usize = start_str
                .trim()
                .parse()
                .map_err(|_| anyhow!("Invalid page range: '{}'", part))?;
            let end: usize = end_str
                .trim()
                .parse()
                .map_err(|_| anyhow!("Invalid page range: '{}'", part))?;

            if start == 0 || end == 0 || start > end {
                return Err(anyhow!(
                    "Invalid page range: '{}' (pages are 1-indexed)",
                    part
                ));
            }

            for p in start..=end.min(total_pages) {
                if !pages.contains(&p) {
                    pages.push(p);
                }
            }
        } else {
            let p: usize = part
                .parse()
                .map_err(|_| anyhow!("Invalid page number: '{}'", part))?;
            if p == 0 {
                return Err(anyhow!("Page numbers are 1-indexed, got 0"));
            }
            if p <= total_pages && !pages.contains(&p) {
                pages.push(p);
            }
        }
    }

    if pages.is_empty() {
        return Err(anyhow!(
            "No valid pages in range '{}' (document has {} pages)",
            spec,
            total_pages
        ));
    }

    pages.sort();
    Ok(pages)
}
