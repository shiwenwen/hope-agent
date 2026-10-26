use anyhow::{anyhow, Result};
use serde_json::Value;

use super::expand_tilde;
use super::IMAGE_BASE64_PREFIX;

/// Default max characters to return from text extraction.
const DEFAULT_MAX_CHARS: usize = 50_000;
/// Hard cap on max PDFs (user cannot exceed this).
const CAP_MAX_PDFS: usize = 10;
/// Render width for vision mode (pixels).
const VISION_RENDER_WIDTH: u32 = 1200;
/// HTTP timeout for fetching remote PDFs.
const FETCH_TIMEOUT_SECS: u64 = 30;
/// Maximum bytes to download for a remote PDF (20 MB).
const PDF_MAX_FETCH_BYTES: usize = 20 * 1024 * 1024;
/// If text extraction yields fewer chars than this, auto mode switches to vision.
const AUTO_VISION_THRESHOLD: usize = 200;

// ── PDF Tool Config ─────────────────────────────────────────────

// 类型已下沉 ha-config-schema：PdfToolConfig 及其 serde default helper。
pub use ha_config_schema::tools::pdf::PdfToolConfig;

// ── PDF Source Types ────────────────────────────────────────────────

enum PdfSource {
    File { path: String },
    Url { url: String },
}

/// Load PDF tool config from the global config store, clamped to hard caps.
fn load_pdf_config() -> PdfToolConfig {
    let mut cfg = crate::config::load_config()
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
                let src_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("file");
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
                            sources.push(PdfSource::Url { url: u.to_string() });
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
            sources.push(PdfSource::Url { url: u.to_string() });
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
async fn resolve_file(path_raw: &str) -> Result<(Vec<u8>, String)> {
    let path_raw = path_raw.to_string();
    crate::blocking::run_blocking(move || {
        let path = expand_tilde(&path_raw);
        let file_path = std::path::Path::new(&path);
        if !file_path.exists() {
            return Err(anyhow!("File not found: {}", path));
        }
        let data = std::fs::read(file_path)?;
        Ok((data, format!("file: {}", path)))
    })
    .await
}

/// Fetch PDF bytes from a URL.
async fn resolve_url(url: &str) -> Result<(Vec<u8>, String)> {
    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS)),
    )
    .build()?;
    let headers = crate::tools::web_fetch_common::browser_headers();
    let checked = crate::security::http_redirect::checked_get(
        &client,
        url,
        crate::security::ssrf::SsrfPolicy::Default,
        &[],
        5,
        Some(&headers),
    )
    .await?;
    let resp = checked.response;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {} fetching remote PDF", status));
    }
    let capped =
        crate::security::http_stream::read_bytes_capped_with_info(resp, PDF_MAX_FETCH_BYTES)
            .await?;
    if capped.truncated {
        return Err(anyhow!(
            "PDF too large (max {}MB)",
            PDF_MAX_FETCH_BYTES / 1024 / 1024
        ));
    }
    let bytes = capped.bytes;

    // Validate it looks like a PDF
    if bytes.len() < 5 || &bytes[..5] != b"%PDF-" {
        return Err(anyhow!("URL did not return a valid PDF file"));
    }

    Ok((
        bytes,
        format!(
            "url: {}",
            crate::tools::web_fetch::redact_url_for_display(url)
        ),
    ))
}

/// Resolve a PDF source to raw bytes.
async fn resolve_source(source: &PdfSource) -> Result<(Vec<u8>, String)> {
    match source {
        PdfSource::File { path } => resolve_file(path).await,
        PdfSource::Url { url } => resolve_url(url).await,
    }
}

// ── Text Extraction ─────────────────────────────────────────────────

/// Extract text from PDF bytes, returning per-page text.
fn extract_text_from_bytes(data: &[u8]) -> Result<Vec<String>> {
    let mut document = pdf_extract::Document::load_mem(data)
        .map_err(|_| anyhow!("Text extraction failed: PDF cannot be parsed"))?;
    if document.is_encrypted() {
        document
            .decrypt("")
            .map_err(|_| anyhow!("Text extraction failed: PDF cannot be decrypted"))?;
    }
    let page_ids = document.get_pages();
    if page_ids.is_empty() {
        return Err(anyhow!("PDF has no readable pages"));
    }
    let mut pages = Vec::with_capacity(page_ids.len());
    for (page_number, page_id) in page_ids {
        // pdf-extract's convenience iterator treats every page error as EOF.
        // Enumerate the actual page tree and propagate a failure instead.
        // Its content processor unwraps decoding; validate that fallible step
        // before entering it so malformed streams return a controlled error.
        pdf_extract::content::Content::decode_strict(
            &document
                .get_page_content(page_id)
                .map_err(|_| anyhow!("PDF content on page {page_number} cannot be read"))?,
        )
        .map_err(|_| anyhow!("PDF content on page {page_number} is malformed"))?;
        let mut text = String::new();
        {
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc_page(&document, &mut output, page_number)
                .map_err(|_| anyhow!("Text extraction failed on page {page_number}"))?;
        }
        pages.push(text.trim().to_string());
    }
    Ok(pages)
}

/// Byte-level PDF text extraction shared with `web_fetch`. The caller owns
/// network policy, response caps, and blocking-pool isolation.
pub(crate) fn extract_pdf_text_for_web_fetch(data: &[u8]) -> Result<(String, usize)> {
    let pages = extract_text_from_bytes(data)?;
    let mut output = String::new();
    for (index, page) in pages.iter().enumerate() {
        if page.trim().is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!("--- Page {} ---\n", index + 1));
        output.push_str(page.trim());
    }
    let chars = output.chars().count();
    Ok((output, chars))
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

/// Render PDF pages to image transport markers for LLM vision.
fn build_vision_output(
    data: &[u8],
    page_filter: Option<&[usize]>,
    source_label: &str,
    file_size: usize,
    max_vision_pages: usize,
) -> String {
    // Convert 1-indexed page filter to 0-indexed for render_pdf_bytes
    let indices: Option<Vec<usize>> =
        page_filter.map(|filter| filter.iter().map(|&p| p.saturating_sub(1)).collect());

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

    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("auto");

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
            "text" => match extract_text_from_bytes(&data) {
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
            },
            "vision" => build_vision_output(
                &data,
                page_filter.as_deref(),
                &source_label,
                file_size,
                config.max_vision_pages,
            ),
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

#[cfg(test)]
mod parser_tests {
    use super::*;

    fn synthetic_pdf(catalog_extra: &str, texts: &[&str]) -> Vec<u8> {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"),
            format!(
                "<< /Type /Pages /Count {} /Kids [{}] >>",
                texts.len(),
                (0..texts.len())
                    .map(|i| format!("{} 0 R", 4 + i * 2))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        for (index, text) in texts.iter().enumerate() {
            let stream = format!("BT /F1 12 Tf 20 100 Td ({text}) Tj ET");
            objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + index * 2));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ));
        }
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn normal_pdf_text_pages_and_empty_document_remain_readable() {
        let bytes = synthetic_pdf("", &["First synthetic page", "Second synthetic page"]);
        let pages = extract_text_from_bytes(&bytes).unwrap();
        assert!(pages
            .iter()
            .any(|page| page.contains("First synthetic page")));
        assert!(pages
            .iter()
            .any(|page| page.contains("Second synthetic page")));
        let (text, count) = extract_pdf_text_for_web_fetch(&bytes).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(count, text.chars().count());
        assert!(text.contains("--- Page 2 ---"));
        assert!(text.contains("First synthetic page"));
        assert!(text.contains("Second synthetic page"));
        let empty = extract_text_from_bytes(&synthetic_pdf("", &[""])).unwrap();
        assert!(empty.iter().all(|page| page.trim().is_empty()));
    }

    #[test]
    fn later_page_failure_does_not_return_a_short_successful_document() {
        let bytes = synthetic_pdf("", &["Valid first page", ") [ ("]);
        assert!(pdf_extract::Document::load_mem(&bytes).is_ok());
        let error = extract_text_from_bytes(&bytes).unwrap_err();
        assert!(error.to_string().contains("page 2"));
        assert!(extract_pdf_text_for_web_fetch(&bytes).is_err());
    }

    #[test]
    fn deep_pdf_returns_error_without_aborting_process() {
        const CHILD: &str = "HA_TEST_DEEP_PDF_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let extra = format!("/X {}{}", "[".repeat(10_380), "]".repeat(10_380));
            assert!(extract_text_from_bytes(&synthetic_pdf(&extra, &["unused"])).is_err());
            return;
        }
        // A stack overflow abort cannot be caught by catch_unwind. Isolate the
        // synthetic input so a vulnerable dependency fails this test safely.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tools::pdf::parser_tests::deep_pdf_returns_error_without_aborting_process",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "PDF parser subprocess failed: {status}");
                break;
            }
            if start.elapsed() > std::time::Duration::from_secs(30) {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("PDF parser subprocess exceeded its time limit");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
