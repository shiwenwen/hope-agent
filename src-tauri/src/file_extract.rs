//! File content extraction module.
//!
//! Extracts text and images from various file types (PDF, Word, Excel, PPT, text files)
//! so they can be sent to LLM APIs alongside user messages.

use anyhow::Result;
use base64::Engine;
use std::io::Read;
use std::path::Path;

/// Maximum characters to extract from a single file.
const MAX_TEXT_CHARS: usize = 200_000;

/// Maximum PDF pages to render as images.
const MAX_PDF_PAGES: usize = 8;

/// Maximum width (pixels) for rendered PDF page images.
const PDF_RENDER_WIDTH: u32 = 1200;

/// Maximum file size (bytes) to read as text (10 MB).
const MAX_TEXT_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// An image extracted from a document (PDF page, PPT embedded image, etc.).
#[derive(Debug, Clone)]
pub struct ExtractedImage {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type (e.g., "image/png").
    pub mime_type: String,
    /// Human-readable label (e.g., "Page 1", "Slide 3").
    pub label: String,
}

/// Result of extracting content from a file.
#[derive(Debug, Clone)]
pub struct FileContent {
    /// Extracted text content (if any).
    pub text: Option<String>,
    /// Extracted images (PDF pages, PPT media, etc.).
    pub images: Vec<ExtractedImage>,
    /// Original file path on disk (always present).
    pub file_path: String,
    /// Original file name.
    pub file_name: String,
}

/// Extract content from a file based on its MIME type and extension.
pub fn extract(file_path: &str, file_name: &str, mime_type: &str) -> FileContent {
    let path = Path::new(file_path);
    let lower_name = file_name.to_lowercase();

    let result = if mime_type == "application/pdf" || lower_name.ends_with(".pdf") {
        extract_pdf(path)
    } else if mime_type
        == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || lower_name.ends_with(".docx")
    {
        extract_docx(path)
    } else if mime_type
        == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || mime_type == "application/vnd.ms-excel"
        || lower_name.ends_with(".xlsx")
        || lower_name.ends_with(".xls")
    {
        extract_excel(path)
    } else if mime_type
        == "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        || lower_name.ends_with(".pptx")
    {
        extract_pptx(path)
    } else if is_text_file(mime_type, &lower_name) {
        extract_text_file(path)
    } else {
        // Unknown binary file — no extraction, just provide path
        Ok((None, Vec::new()))
    };

    match result {
        Ok((text, images)) => FileContent {
            text,
            images,
            file_path: file_path.to_string(),
            file_name: file_name.to_string(),
        },
        Err(e) => {
            log::warn!("Failed to extract content from '{}': {}", file_name, e);
            FileContent {
                text: Some(format!("[Error extracting content: {}]", e)),
                images: Vec::new(),
                file_path: file_path.to_string(),
                file_name: file_name.to_string(),
            }
        }
    }
}

/// Truncate text to `MAX_TEXT_CHARS`, appending a notice if truncated.
fn truncate_text(text: String) -> String {
    let char_count = text.chars().count();
    if char_count > MAX_TEXT_CHARS {
        let truncated: String = text.chars().take(MAX_TEXT_CHARS).collect();
        format!(
            "{}...\n[Content truncated at {} characters, total {} characters]",
            truncated, MAX_TEXT_CHARS, char_count
        )
    } else {
        text
    }
}

// ---------------------------------------------------------------------------
// PDF extraction
// ---------------------------------------------------------------------------

fn extract_pdf(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    // 1. Extract text
    let text = match pdf_extract::extract_text(path) {
        Ok(t) => {
            let trimmed = t.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(truncate_text(trimmed))
            }
        }
        Err(e) => {
            log::warn!("PDF text extraction failed for {:?}: {}", path, e);
            None
        }
    };

    // 2. Render pages as images via pdfium
    let images = render_pdf_pages(path).unwrap_or_else(|e| {
        log::warn!("PDF page rendering failed for {:?}: {}", path, e);
        Vec::new()
    });

    Ok((text, images))
}

/// Render PDF pages to PNG images using pdfium-render.
fn render_pdf_pages(path: &Path) -> Result<Vec<ExtractedImage>> {
    use pdfium_render::prelude::*;

    // Try to bind to system pdfium library
    let pdfium = Pdfium::new(
        Pdfium::bind_to_system_library()
            .or_else(|_| {
                // Try common locations
                #[cfg(target_os = "macos")]
                {
                    Pdfium::bind_to_library("/usr/local/lib/libpdfium.dylib")
                        .or_else(|_| Pdfium::bind_to_library("/opt/homebrew/lib/libpdfium.dylib"))
                }
                #[cfg(target_os = "linux")]
                {
                    Pdfium::bind_to_library("/usr/lib/libpdfium.so")
                        .or_else(|_| Pdfium::bind_to_library("/usr/local/lib/libpdfium.so"))
                }
                #[cfg(target_os = "windows")]
                {
                    Pdfium::bind_to_library("pdfium.dll")
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
                {
                    Err(PdfiumError::NoBindingsAvailable)
                }
            })
            .map_err(|e| anyhow::anyhow!("PDFium library not found: {:?}", e))?,
    );

    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    let pages = document.pages();
    let page_count = pages.len().min(MAX_PDF_PAGES as u16);
    let mut images = Vec::new();

    for i in 0..page_count {
        let page = pages.get(i).map_err(|e| {
            anyhow::anyhow!("Failed to get page {}: {:?}", i, e)
        })?;

        // Calculate render size maintaining aspect ratio
        let width = page.width();
        let height = page.height();
        let scale = PDF_RENDER_WIDTH as f32 / width.value;
        let render_height = (height.value * scale) as u32;

        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(PDF_RENDER_WIDTH as i32)
                    .set_target_height(render_height as i32),
            )
            .map_err(|e| anyhow::anyhow!("Failed to render page {}: {:?}", i, e))?;

        let img = bitmap.as_image();
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| anyhow::anyhow!("Failed to encode page {} as PNG: {}", i, e))?;

        let b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
        images.push(ExtractedImage {
            data: b64,
            mime_type: "image/png".to_string(),
            label: format!("Page {}", i + 1),
        });
    }

    Ok(images)
}

// ---------------------------------------------------------------------------
// Word (.docx) extraction
// ---------------------------------------------------------------------------

fn extract_docx(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Read word/document.xml
    let mut xml_content = String::new();
    {
        let mut doc_entry = archive.by_name("word/document.xml")?;
        doc_entry.read_to_string(&mut xml_content)?;
    }

    // Parse XML and extract text from <w:t> elements
    let text = extract_xml_text(&xml_content, b"w:t");
    let text = if text.is_empty() {
        None
    } else {
        Some(truncate_text(text))
    };

    Ok((text, Vec::new()))
}

// ---------------------------------------------------------------------------
// Excel (.xlsx/.xls) extraction
// ---------------------------------------------------------------------------

fn extract_excel(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    use calamine::{open_workbook_auto, Data, Reader};

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| anyhow::anyhow!("Failed to open workbook: {}", e))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
    let mut output = String::new();

    for name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(name) {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(&format!("=== Sheet: {} ===\n", name));

            for row in range.rows() {
                let cells: Vec<String> = row
                    .iter()
                    .map(|cell| match cell {
                        Data::Empty => String::new(),
                        Data::String(s) => s.clone(),
                        Data::Float(f) => {
                            if *f == (*f as i64) as f64 {
                                format!("{}", *f as i64)
                            } else {
                                format!("{}", f)
                            }
                        }
                        Data::Int(i) => format!("{}", i),
                        Data::Bool(b) => format!("{}", b),
                        Data::Error(e) => format!("#ERR:{:?}", e),
                        Data::DateTime(dt) => format!("{}", dt),
                        Data::DateTimeIso(s) => s.clone(),
                        Data::DurationIso(s) => s.clone(),
                    })
                    .collect();
                output.push_str(&cells.join("\t"));
                output.push('\n');
            }
        }
    }

    let text = if output.trim().is_empty() {
        None
    } else {
        Some(truncate_text(output))
    };

    Ok((text, Vec::new()))
}

// ---------------------------------------------------------------------------
// PowerPoint (.pptx) extraction
// ---------------------------------------------------------------------------

fn extract_pptx(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // 1. Extract text from slides
    let mut slide_texts = Vec::new();
    let mut slide_index = 1;
    loop {
        let slide_path = format!("ppt/slides/slide{}.xml", slide_index);
        match archive.by_name(&slide_path) {
            Ok(mut entry) => {
                let mut xml_content = String::new();
                entry.read_to_string(&mut xml_content)?;
                let text = extract_xml_text(&xml_content, b"a:t");
                if !text.trim().is_empty() {
                    slide_texts.push(format!("--- Slide {} ---\n{}", slide_index, text.trim()));
                }
                slide_index += 1;
            }
            Err(_) => break,
        }
    }

    let text = if slide_texts.is_empty() {
        None
    } else {
        Some(truncate_text(slide_texts.join("\n\n")))
    };

    // 2. Extract embedded images from ppt/media/
    let mut images = Vec::new();
    let media_entries: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            archive
                .by_index(i)
                .ok()
                .map(|e| e.name().to_string())
                .filter(|name| name.starts_with("ppt/media/"))
        })
        .collect();

    for entry_name in media_entries {
        let mime = guess_image_mime(&entry_name);
        if let Some(mime_type) = mime {
            if let Ok(mut entry) = archive.by_name(&entry_name) {
                let mut buf = Vec::new();
                if entry.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
                    let label = entry_name
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry_name)
                        .to_string();
                    images.push(ExtractedImage {
                        data: b64,
                        mime_type,
                        label,
                    });
                }
            }
        }
    }

    Ok((text, images))
}

/// Guess MIME type for common image extensions in Office media folders.
fn guess_image_mime(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        Some("image/png".to_string())
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg".to_string())
    } else if lower.ends_with(".gif") {
        Some("image/gif".to_string())
    } else if lower.ends_with(".webp") {
        Some("image/webp".to_string())
    } else if lower.ends_with(".bmp") {
        Some("image/bmp".to_string())
    } else if lower.ends_with(".tiff") || lower.ends_with(".tif") {
        Some("image/tiff".to_string())
    } else if lower.ends_with(".svg") {
        Some("image/svg+xml".to_string())
    } else if lower.ends_with(".emf") || lower.ends_with(".wmf") {
        // Windows metafile — skip, not useful for LLM
        None
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Text file extraction
// ---------------------------------------------------------------------------

fn extract_text_file(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Ok((
            Some(format!(
                "[File is too large ({:.1} MB). Only showing file path for tool-based access.]",
                metadata.len() as f64 / 1024.0 / 1024.0
            )),
            Vec::new(),
        ));
    }

    match std::fs::read_to_string(path) {
        Ok(content) => Ok((Some(truncate_text(content)), Vec::new())),
        Err(_) => {
            // Possibly binary or non-UTF-8 encoding
            Ok((
                Some("[File could not be read as UTF-8 text.]".to_string()),
                Vec::new(),
            ))
        }
    }
}

/// Check whether a file should be treated as a text file based on MIME type or extension.
fn is_text_file(mime_type: &str, lower_name: &str) -> bool {
    if mime_type.starts_with("text/") {
        return true;
    }
    let text_mimes = [
        "application/json",
        "application/xml",
        "application/javascript",
        "application/typescript",
        "application/x-yaml",
        "application/toml",
        "application/x-sh",
        "application/sql",
    ];
    if text_mimes.iter().any(|m| mime_type == *m) {
        return true;
    }
    // Fallback: check file extension
    let text_extensions = [
        "txt", "md", "markdown", "html", "htm", "css", "js", "jsx", "ts", "tsx",
        "json", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf",
        "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd",
        "py", "rb", "rs", "go", "java", "kt", "scala", "c", "cpp", "h", "hpp",
        "cs", "swift", "m", "mm", "r", "lua", "pl", "pm", "php",
        "sql", "graphql", "proto", "cmake",
        "csv", "tsv", "log", "env", "gitignore", "dockerignore",
        "vue", "svelte", "astro",
    ];
    if let Some(ext) = lower_name.rsplit('.').next() {
        if text_extensions.iter().any(|e| ext == *e) {
            return true;
        }
    }
    // Common extensionless files
    let stem_names = [
        "makefile",
        "dockerfile",
        "rakefile",
        "gemfile",
        "procfile",
        "vagrantfile",
    ];
    stem_names.iter().any(|n| lower_name == *n)
}

// ---------------------------------------------------------------------------
// XML text extraction helper (used by docx and pptx)
// ---------------------------------------------------------------------------

/// Extract all text content from elements with the given tag name in an XML string.
/// Uses quick-xml for efficient streaming parse.
fn extract_xml_text(xml: &str, target_tag: &[u8]) -> String {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut output = String::new();
    let mut inside_target = false;
    let mut last_was_paragraph_end = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                if local.as_ref() == target_tag {
                    inside_target = true;
                    last_was_paragraph_end = false;
                }
                // Track paragraph boundaries for docx (<w:p>) and pptx (<a:p>)
                if local.as_ref() == b"w:p" || local.as_ref() == b"a:p" {
                    if !output.is_empty() && !last_was_paragraph_end {
                        output.push('\n');
                    }
                    last_was_paragraph_end = false;
                }
            }
            Ok(Event::Text(ref e)) => {
                if inside_target {
                    if let Ok(text) = e.unescape() {
                        output.push_str(&text);
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                if local.as_ref() == target_tag {
                    inside_target = false;
                }
                if local.as_ref() == b"w:p" || local.as_ref() == b"a:p" {
                    last_was_paragraph_end = true;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                log::warn!("XML parse error: {}", e);
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    output
}
