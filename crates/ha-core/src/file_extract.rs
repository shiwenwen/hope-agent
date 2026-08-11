//! File content extraction module.
//!
//! Extracts text and images from various file types (PDF, Word, Excel, PPT, text files)
//! so they can be sent to LLM APIs alongside user messages.

use anyhow::{Context, Result};
use base64::Engine;
use std::collections::HashMap;
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

const MAX_BOUNDED_ZIP_ENTRIES: usize = 4_096;
const MAX_BOUNDED_ZIP_CENTRAL_DIRECTORY_BYTES: usize = 4 * 1024 * 1024;
const MAX_BOUNDED_ZIP_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const MAX_BOUNDED_ZIP_ENTRY_BYTES: usize = 32 * 1024 * 1024;
const MAX_BOUNDED_SHARED_STRINGS: usize = 10_000;
const MAX_BOUNDED_XML_DEPTH: usize = 256;
const MAX_BOUNDED_XML_ATTRIBUTES_PER_ELEMENT: usize = 256;
const MAX_BOUNDED_XML_ATTRIBUTE_BYTES: usize = 64 * 1024;
const MAX_BOUNDED_XML_TEXT_EVENT_BYTES: usize = MAX_TEXT_CHARS * 12;
const MAX_BOUNDED_XML_REFERENCE_BYTES: usize = 64;
const MAX_BOUNDED_XML_ELEMENT_NAME_BYTES: usize = 1_024;
const MAX_BOUNDED_XLSX_CELL_CHARS: usize = 32_767;
const MAX_BOUNDED_XLSX_CELL_BYTES: usize = MAX_BOUNDED_XLSX_CELL_CHARS * 4;
const RESOURCE_CONTEXT_SHARE_PERCENT: usize = 20;
const MIN_RESOURCE_CONTEXT_CHARS: usize = 8_000;
const MAX_RESOURCE_CONTEXT_CHARS: usize = 200_000;

/// Remaining resident-memory allowance for extraction of one frozen typed
/// resource batch. It starts after raw/Base64/provider baseline accounting;
/// retained text/image charges are cumulative while parser/decompression work
/// is checked as a transient peak before any corresponding allocation.
#[derive(Debug, Clone)]
pub(crate) struct FrozenExtractionBudget {
    remaining_bytes: usize,
}

impl FrozenExtractionBudget {
    pub(crate) fn new(remaining_bytes: usize) -> Self {
        Self { remaining_bytes }
    }

    pub(crate) fn ensure_transient(&self, bytes: usize) -> Result<()> {
        if bytes > self.remaining_bytes {
            anyhow::bail!("bounded typed-resource extraction exceeds its remaining memory budget");
        }
        Ok(())
    }

    pub(crate) fn remaining_bytes(&self) -> usize {
        self.remaining_bytes
    }

    pub(crate) fn retain_provider_text(&mut self, materialized_utf8_bytes: usize) -> Result<()> {
        let charge = self.provider_text_charge(materialized_utf8_bytes)?;
        self.retain(charge)
    }

    fn reserve_provider_text(&mut self, materialized_utf8_bytes: usize) -> Result<usize> {
        let charge = self.provider_text_charge(materialized_utf8_bytes)?;
        self.retain(charge)?;
        Ok(charge)
    }

    fn settle_provider_text_reservation(
        &mut self,
        reserved_bytes: usize,
        materialized_utf8_bytes: usize,
    ) -> Result<()> {
        let actual = self.provider_text_charge(materialized_utf8_bytes)?;
        if actual > reserved_bytes {
            anyhow::bail!("bounded typed-resource text exceeded its reserved memory budget");
        }
        self.remaining_bytes = self
            .remaining_bytes
            .checked_add(reserved_bytes - actual)
            .context("bounded typed-resource text reservation accounting overflow")?;
        Ok(())
    }

    fn provider_text_charge(&self, materialized_utf8_bytes: usize) -> Result<usize> {
        crate::attachments::typed_provider_text_resident_bytes(materialized_utf8_bytes)
            .context("bounded typed-resource text accounting overflow")
    }

    pub(crate) fn reserve_extracted_image(
        &mut self,
        encoded_bytes: usize,
        mime_len: usize,
        transient_bytes: usize,
    ) -> Result<()> {
        let payload =
            crate::attachments::typed_provider_image_payload_bytes(encoded_bytes, mime_len)
                .context("bounded typed-resource image accounting overflow")?;
        // ExtractedImage.data is one additional retained source beyond the
        // seven provider-side copies modeled for direct Attachment.data.
        let retained = payload
            .checked_mul(
                crate::attachments::TYPED_RESOURCE_PROVIDER_PAYLOAD_COPIES
                    .checked_add(1)
                    .context("bounded typed-resource image accounting overflow")?,
            )
            .context("bounded typed-resource image accounting overflow")?;
        let peak = retained
            .checked_add(transient_bytes)
            .context("bounded typed-resource image accounting overflow")?;
        self.ensure_transient(peak)?;
        self.retain(retained)
    }

    fn retain(&mut self, bytes: usize) -> Result<()> {
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(bytes)
            .context("bounded typed-resource extraction exceeds its remaining memory budget")?;
        Ok(())
    }
}

/// An image extracted from a document (PDF page, PPT embedded image, etc.).
#[derive(Debug, Clone)]
pub struct ExtractedImage {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type (e.g., "image/png").
    pub mime_type: String,
    /// Human-readable label (e.g., "Page 1", "Slide 3").
    #[allow(dead_code)]
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
    /// The bounded Office parser reached its extraction ceiling. The frozen
    /// bytes remain exact, but text continuation cannot claim document EOF.
    pub extraction_truncated: bool,
}

/// Extract content from a file based on its MIME type and extension.
pub fn extract(file_path: &str, file_name: &str, mime_type: &str) -> FileContent {
    let path = Path::new(file_path);
    let lower_name = file_name.to_lowercase();

    let result = if mime_type == "application/pdf" || lower_name.ends_with(".pdf") {
        extract_pdf(path)
    } else if mime_type == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || lower_name.ends_with(".docx")
    {
        extract_docx(path)
    } else if mime_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
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
    } else if is_text_like(mime_type, &lower_name) {
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
            extraction_truncated: false,
        },
        Err(e) => {
            app_warn!(
                "tool",
                "file_extract",
                "Failed to extract content from '{}': {}",
                file_name,
                e
            );
            FileContent {
                text: Some(format!("[Error extracting content: {}]", e)),
                images: Vec::new(),
                file_path: file_path.to_string(),
                file_name: file_name.to_string(),
                extraction_truncated: false,
            }
        }
    }
}

/// Extract a frozen typed resource under a caller-owned aggregate budget.
/// This path intentionally supports only formats whose decompression and
/// materialization can be bounded before allocation. PDF text/page libraries
/// eagerly inflate internal streams, so typed PDFs remain reference-only until
/// extraction is moved into an OS-memory-limited worker.
pub(crate) fn extract_frozen_bytes_bounded(
    bytes: &[u8],
    file_name: &str,
    mime_type: &str,
    budget: &mut FrozenExtractionBudget,
    include_images: bool,
    retained_text_chars: usize,
) -> Result<FileContent> {
    let lower_name = file_name.to_ascii_lowercase();
    let result = if mime_type
        == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || lower_name.ends_with(".docx")
    {
        extract_docx_bounded(bytes, budget, retained_text_chars)
    } else if mime_type
        == "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        || lower_name.ends_with(".pptx")
    {
        extract_pptx_bounded(bytes, budget, include_images, retained_text_chars)
    } else if mime_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || lower_name.ends_with(".xlsx")
    {
        extract_xlsx_bounded(bytes, budget, retained_text_chars)
    } else if mime_type == "application/pdf" || lower_name.ends_with(".pdf") {
        anyhow::bail!("bounded PDF extraction is unavailable; use the frozen resource Base64 pager")
    } else if mime_type == "application/vnd.ms-excel" || lower_name.ends_with(".xls") {
        anyhow::bail!(
            "bounded legacy XLS extraction is unavailable; use the frozen resource Base64 pager"
        )
    } else {
        anyhow::bail!(
            "bounded extraction is unavailable for this frozen binary resource; use the frozen resource Base64 pager"
        )
    }?;

    Ok(FileContent {
        text: result.0,
        images: result.1,
        file_path: file_name.to_string(),
        file_name: file_name.to_string(),
        extraction_truncated: result.2,
    })
}

#[derive(Debug)]
struct BoundedOfficeEntry {
    name: String,
    compression: u16,
    compressed_start: usize,
    compressed_size: usize,
    uncompressed_size: usize,
    crc32: u32,
}

#[derive(Debug)]
struct BoundedOfficeArchive<'a> {
    bytes: &'a [u8],
    entries: Vec<BoundedOfficeEntry>,
    entry_index: HashMap<String, usize>,
    index_working_set: usize,
}

impl<'a> BoundedOfficeArchive<'a> {
    fn open(bytes: &'a [u8], budget: &FrozenExtractionBudget) -> Result<Self> {
        let (eocd, declared_entries, central_offset, central_size) = zip_end_record(bytes)?;
        // Entry names live in the entry vector and lookup map; format parsers
        // can additionally retain bounded relationship/order lists. Charge
        // four central-directory-sized copies before any of those allocate.
        let index_working_set = central_size
            .checked_mul(4)
            .and_then(|value| value.checked_add(declared_entries.checked_mul(512)?))
            .context("typed Office archive index accounting overflow")?;
        budget.ensure_transient(index_working_set)?;

        let central_end = central_offset
            .checked_add(central_size)
            .context("typed Office central directory accounting overflow")?;
        if central_end != eocd {
            anyhow::bail!("typed Office central directory is not contiguous with its end record");
        }
        let mut entries = Vec::with_capacity(declared_entries);
        let mut entry_index = HashMap::with_capacity(declared_entries);
        let mut cursor = central_offset;
        let mut total_uncompressed = 0usize;
        for _ in 0..declared_entries {
            if cursor.checked_add(46).is_none_or(|end| end > central_end)
                || &bytes[cursor..cursor + 4] != b"PK\x01\x02"
            {
                anyhow::bail!("typed Office central directory is malformed");
            }
            let flags = read_le_u16(bytes, cursor + 8)?;
            let compression = read_le_u16(bytes, cursor + 10)?;
            let crc32 = read_le_u32(bytes, cursor + 16)?;
            let compressed_size = read_le_u32(bytes, cursor + 20)? as usize;
            let uncompressed_size = read_le_u32(bytes, cursor + 24)? as usize;
            let name_len = read_le_u16(bytes, cursor + 28)? as usize;
            let extra_len = read_le_u16(bytes, cursor + 30)? as usize;
            let comment_len = read_le_u16(bytes, cursor + 32)? as usize;
            let disk_start = read_le_u16(bytes, cursor + 34)?;
            let local_offset = read_le_u32(bytes, cursor + 42)? as usize;
            // Deflate may use bits 1-2 as a compression-level hint. Apart
            // from those, data descriptors and UTF-8 names, reject every ZIP
            // feature (notably encryption/patching/masked headers) rather than
            // interpreting a central-directory-only view.
            let allowed_flags = (1 << 3)
                | (1 << 11)
                | if compression == 8 {
                    (1 << 1) | (1 << 2)
                } else {
                    0
                };
            if flags & !allowed_flags != 0
                || disk_start != 0
                || !matches!(compression, 0 | 8)
                || compressed_size == u32::MAX as usize
                || uncompressed_size == u32::MAX as usize
                || local_offset == u32::MAX as usize
                || uncompressed_size > MAX_BOUNDED_ZIP_ENTRY_BYTES
            {
                anyhow::bail!("typed Office entry uses an unsupported ZIP feature");
            }
            let variable_len = name_len
                .checked_add(extra_len)
                .and_then(|value| value.checked_add(comment_len))
                .context("typed Office central entry accounting overflow")?;
            let next = cursor
                .checked_add(46)
                .and_then(|value| value.checked_add(variable_len))
                .context("typed Office central entry accounting overflow")?;
            if next > central_end {
                anyhow::bail!("typed Office central entry exceeds its directory");
            }
            let name_start = cursor + 46;
            let name = std::str::from_utf8(&bytes[name_start..name_start + name_len])
                .context("typed Office entry name is not UTF-8")?
                .to_string();
            if entry_index.contains_key(&name) {
                anyhow::bail!("typed Office archive contains duplicate entry names");
            }

            let local_header_end = local_offset
                .checked_add(30)
                .context("typed Office local header accounting overflow")?;
            if local_header_end > central_offset
                || bytes.get(local_offset..local_offset.saturating_add(4)) != Some(b"PK\x03\x04")
            {
                anyhow::bail!("typed Office local entry header is malformed");
            }
            let local_field = |delta: usize| {
                local_offset
                    .checked_add(delta)
                    .context("typed Office local header field accounting overflow")
            };
            let local_flags = read_le_u16(bytes, local_field(6)?)?;
            if local_flags != flags || read_le_u16(bytes, local_field(8)?)? != compression {
                anyhow::bail!("typed Office local entry header differs from its central entry");
            }
            let local_name_len = read_le_u16(bytes, local_field(26)?)? as usize;
            let local_extra_len = read_le_u16(bytes, local_field(28)?)? as usize;
            let compressed_start = local_header_end
                .checked_add(local_name_len)
                .and_then(|value| value.checked_add(local_extra_len))
                .context("typed Office local entry accounting overflow")?;
            let local_name_start = local_header_end;
            let local_name_end = local_name_start
                .checked_add(local_name_len)
                .context("typed Office local name accounting overflow")?;
            if local_name_end > central_offset
                || local_name_len != name_len
                || bytes.get(local_name_start..local_name_end)
                    != bytes.get(name_start..name_start + name_len)
            {
                anyhow::bail!("typed Office local and central entry names differ");
            }
            if flags & (1 << 3) == 0
                && (read_le_u32(bytes, local_field(14)?)? != crc32
                    || read_le_u32(bytes, local_field(18)?)? as usize != compressed_size
                    || read_le_u32(bytes, local_field(22)?)? as usize != uncompressed_size)
            {
                anyhow::bail!("typed Office local and central entry declarations differ");
            }
            if compressed_start
                .checked_add(compressed_size)
                .is_none_or(|end| end > central_offset)
                || compression == 0 && compressed_size != uncompressed_size
            {
                anyhow::bail!("typed Office compressed entry exceeds its archive");
            }
            total_uncompressed = total_uncompressed
                .checked_add(uncompressed_size)
                .context("typed Office aggregate size accounting overflow")?;
            let index = entries.len();
            entries.push(BoundedOfficeEntry {
                name,
                compression,
                compressed_start,
                compressed_size,
                uncompressed_size,
                crc32,
            });
            entry_index.insert(entries[index].name.clone(), index);
            cursor = next;
        }
        if cursor != central_end || total_uncompressed > MAX_BOUNDED_ZIP_UNCOMPRESSED_BYTES {
            anyhow::bail!("typed Office archive exceeds its bounded aggregate layout");
        }
        Ok(Self {
            bytes,
            entries,
            entry_index,
            index_working_set,
        })
    }

    fn contains(&self, name: &str) -> bool {
        self.entry_index.contains_key(name)
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }

    fn declared_size(&self, name: &str) -> Result<usize> {
        self.entry(name).map(|entry| entry.uncompressed_size)
    }

    fn declared_size_for(&self, predicate: impl Fn(&str) -> bool) -> Result<usize> {
        self.entries
            .iter()
            .filter(|entry| predicate(&entry.name))
            .try_fold(0usize, |total, entry| {
                total
                    .checked_add(entry.uncompressed_size)
                    .context("typed Office relevant-entry accounting overflow")
            })
    }

    fn read_bytes(
        &self,
        name: &str,
        hard_limit: usize,
        budget: &FrozenExtractionBudget,
    ) -> Result<Vec<u8>> {
        let entry = self.entry(name)?;
        let limit = hard_limit
            .min(MAX_BOUNDED_ZIP_ENTRY_BYTES)
            .min(budget.remaining_bytes);
        if entry.uncompressed_size > limit {
            anyhow::bail!("typed Office entry exceeds its bounded read limit");
        }
        let transient = self
            .index_working_set
            .checked_add(entry.uncompressed_size)
            .and_then(|value| value.checked_add(1))
            .and_then(|value| value.checked_add(64 * 1024))
            .context("typed Office decompression accounting overflow")?;
        budget.ensure_transient(transient)?;
        let compressed_end = entry.compressed_start + entry.compressed_size;
        let compressed = &self.bytes[entry.compressed_start..compressed_end];
        let mut output = vec![0; entry.uncompressed_size];
        if entry.compression == 0 {
            if compressed.len() != output.len() {
                anyhow::bail!("typed Office stored entry changed size while being read");
            }
            output.copy_from_slice(compressed);
        } else {
            let mut decoder = flate2::read::DeflateDecoder::new(compressed);
            decoder
                .read_exact(&mut output)
                .context("typed Office entry ended before its declared size")?;
            let mut probe = [0u8; 1];
            loop {
                match decoder.read(&mut probe) {
                    Ok(0) => break,
                    Ok(_) => {
                        anyhow::bail!("typed Office entry exceeded its declared uncompressed size")
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error).context("probe typed Office entry EOF"),
                }
            }
        }
        let mut crc = flate2::Crc::new();
        crc.update(&output);
        if crc.sum() != entry.crc32 {
            anyhow::bail!("typed Office entry failed its CRC check");
        }
        Ok(output)
    }

    fn read_string(
        &self,
        name: &str,
        hard_limit: usize,
        budget: &FrozenExtractionBudget,
    ) -> Result<String> {
        let text = String::from_utf8(self.read_bytes(name, hard_limit, budget)?)
            .context("typed Office XML entry is not UTF-8")?;
        ensure_bounded_xml_encoding(&text)?;
        Ok(text)
    }

    fn entry(&self, name: &str) -> Result<&BoundedOfficeEntry> {
        self.entry_index
            .get(name)
            .and_then(|index| self.entries.get(*index))
            .context("typed Office archive is missing a required entry")
    }
}

fn zip_end_record(bytes: &[u8]) -> Result<(usize, usize, usize, usize)> {
    const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const EOCD_FIXED_BYTES: usize = 22;
    if bytes.len() < EOCD_FIXED_BYTES {
        anyhow::bail!("typed Office archive has no valid end record");
    }
    let search_start = bytes
        .len()
        .saturating_sub(EOCD_FIXED_BYTES + u16::MAX as usize);
    let eocd = (search_start..=bytes.len() - EOCD_FIXED_BYTES)
        .rev()
        .find(|offset| &bytes[*offset..*offset + 4] == EOCD_SIGNATURE)
        .context("typed Office archive has no valid end record")?;
    let read_u16 =
        |offset: usize| u16::from_le_bytes([bytes[eocd + offset], bytes[eocd + offset + 1]]);
    let read_u32 = |offset: usize| {
        u32::from_le_bytes([
            bytes[eocd + offset],
            bytes[eocd + offset + 1],
            bytes[eocd + offset + 2],
            bytes[eocd + offset + 3],
        ])
    };
    let comment_len = read_u16(20) as usize;
    if eocd
        .checked_add(EOCD_FIXED_BYTES)
        .and_then(|value| value.checked_add(comment_len))
        != Some(bytes.len())
        || read_u16(4) != 0
        || read_u16(6) != 0
        || read_u16(8) != read_u16(10)
    {
        anyhow::bail!("typed Office archive uses an unsupported multi-disk/end-record layout");
    }
    let entries = read_u16(10) as usize;
    let central_size = read_u32(12) as usize;
    let central_offset = read_u32(16) as usize;
    if entries == u16::MAX as usize
        || central_size == u32::MAX as usize
        || central_offset == u32::MAX as usize
        || entries > MAX_BOUNDED_ZIP_ENTRIES
        || central_size > MAX_BOUNDED_ZIP_CENTRAL_DIRECTORY_BYTES
        || central_offset
            .checked_add(central_size)
            .is_none_or(|end| end > eocd)
    {
        anyhow::bail!("typed Office archive index exceeds the bounded extraction limits");
    }
    Ok((eocd, entries, central_offset, central_size))
}

fn read_le_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .context("typed Office ZIP field is out of bounds")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .context("typed Office ZIP field is out of bounds")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn ensure_bounded_xml_encoding(text: &str) -> Result<()> {
    let trimmed = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let mut prefix_len = trimmed.len().min(512);
    while !trimmed.is_char_boundary(prefix_len) {
        prefix_len -= 1;
    }
    let prefix = &trimmed[..prefix_len];
    let prefix = prefix.to_ascii_lowercase();
    if !prefix.starts_with("<?xml") {
        return Ok(());
    }
    let declaration_end = prefix
        .find("?>")
        .context("typed Office XML declaration exceeds the bounded prefix or is unclosed")?;
    let declaration = &prefix[..declaration_end];
    let Some(encoding_at) = declaration.find("encoding") else {
        return Ok(());
    };
    let after = declaration[encoding_at + "encoding".len()..].trim_start();
    let after = after
        .strip_prefix('=')
        .context("typed Office XML has a malformed encoding declaration")?
        .trim_start();
    let quote = after
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))
        .context("typed Office XML has a malformed encoding declaration")?;
    let quoted = &after[quote.len_utf8()..];
    let closing = quoted
        .find(quote)
        .context("typed Office XML has an unclosed encoding declaration")?;
    let value = &quoted[..closing];
    if !matches!(value, "utf-8" | "utf8" | "us-ascii" | "ascii") {
        anyhow::bail!("typed Office XML encoding is not bounded UTF-8/ASCII");
    }
    Ok(())
}

fn projected_retained_text_bytes_upper(
    extracted_utf8_upper: usize,
    retained_text_chars: usize,
) -> usize {
    extracted_utf8_upper
        .min(retained_text_chars.saturating_mul(4))
        .saturating_add(512)
}

pub(crate) fn projected_retained_text_bytes(text: &str, retained_text_chars: usize) -> usize {
    let extracted_chars = text.chars().count();
    if extracted_chars <= retained_text_chars {
        text.len()
    } else {
        let head_chars = retained_text_chars.saturating_mul(3) / 4;
        let tail_chars = retained_text_chars.saturating_sub(head_chars);
        let selected_bytes = text
            .chars()
            .take(head_chars)
            .map(char::len_utf8)
            .sum::<usize>()
            .saturating_add(
                text.chars()
                    .rev()
                    .take(tail_chars)
                    .map(char::len_utf8)
                    .sum::<usize>(),
            );
        selected_bytes.saturating_add(512)
    }
}

pub(crate) fn resource_materialization_budget_chars(
    context_window: u32,
    resource_count: usize,
) -> usize {
    let estimated_window_chars =
        (context_window as usize).saturating_mul(crate::context_compact::CHARS_PER_TOKEN);
    estimated_window_chars
        .saturating_mul(RESOURCE_CONTEXT_SHARE_PERCENT)
        .checked_div(100)
        .unwrap_or(MIN_RESOURCE_CONTEXT_CHARS)
        .clamp(MIN_RESOURCE_CONTEXT_CHARS, MAX_RESOURCE_CONTEXT_CHARS)
        .checked_div(resource_count.max(1))
        .unwrap_or(MIN_RESOURCE_CONTEXT_CHARS)
        .max(1)
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

/// Text-only PDF extraction, no pdfium load. Shared by the generic
/// file-attachment extractor (which still separately rasterizes for chat
/// image context via `extract_pdf`) and the knowledge-base import path
/// (which only rasterizes+OCRs when this returns `None` — rasterizing a PDF
/// whose text already extracted cleanly would load pdfium for images
/// nothing downstream reads).
pub(crate) fn extract_pdf_text(path: &Path) -> Option<String> {
    match pdf_extract::extract_text(path) {
        Ok(t) => {
            let trimmed = t.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(truncate_text(trimmed))
            }
        }
        Err(e) => {
            app_warn!(
                "tool",
                "file_extract",
                "PDF text extraction failed for {:?}: {}",
                path,
                e
            );
            None
        }
    }
}

fn extract_pdf(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    let text = extract_pdf_text(path);

    // Render pages as images via pdfium
    let images = render_pdf_pages(path).unwrap_or_else(|e| {
        app_warn!(
            "tool",
            "file_extract",
            "PDF page rendering failed for {:?}: {}",
            path,
            e
        );
        Vec::new()
    });

    Ok((text, images))
}

/// Bind to the system pdfium library.
fn bind_pdfium() -> Result<pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::*;

    let bindings = Pdfium::bind_to_system_library()
        .or_else(|system_err| {
            let mut last_err = system_err;
            for candidate in crate::platform::pdfium_library_candidates() {
                match Pdfium::bind_to_library(candidate) {
                    Ok(bindings) => return Ok(bindings),
                    Err(err) => last_err = err,
                }
            }
            Err(last_err)
        })
        .map_err(|e| anyhow::anyhow!("PDFium library not found: {:?}", e))?;

    Ok(Pdfium::new(bindings))
}

/// Render a single PDF page to a base64 PNG string.
fn render_page_to_b64(page: &pdfium_render::prelude::PdfPage, render_width: u32) -> Result<String> {
    use pdfium_render::prelude::*;

    let width = page.width();
    let height = page.height();
    let scale = render_width as f32 / width.value;
    let render_height = (height.value * scale) as u32;

    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(render_width as i32)
                .set_target_height(render_height as i32),
        )
        .map_err(|e| anyhow::anyhow!("Failed to render page: {:?}", e))?;

    let img = bitmap
        .as_image()
        .map_err(|e| anyhow::anyhow!("Failed to convert bitmap to image: {:?}", e))?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| anyhow::anyhow!("Failed to encode page as PNG: {}", e))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(buf.into_inner()))
}

/// Render PDF pages to PNG images using pdfium-render.
fn render_pdf_pages(path: &Path) -> Result<Vec<ExtractedImage>> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    let pages = document.pages();
    let page_count = pages.len().min(MAX_PDF_PAGES as i32);
    let mut images = Vec::new();

    for i in 0..page_count {
        let page = pages
            .get(i)
            .map_err(|e| anyhow::anyhow!("Failed to get page {}: {:?}", i, e))?;

        let b64 = render_page_to_b64(&page, PDF_RENDER_WIDTH)?;
        images.push(ExtractedImage {
            data: b64,
            mime_type: "image/png".to_string(),
            label: format!("Page {}", i + 1),
        });
    }

    Ok(images)
}

/// Render specific PDF pages from raw bytes to base64 PNG images.
/// `page_indices` is 0-indexed. If `None`, renders all pages up to `max_pages`.
/// Returns `(total_page_count, Vec<(page_number_1indexed, base64_png)>)`.
pub(crate) fn render_pdf_bytes(
    data: &[u8],
    page_indices: Option<&[usize]>,
    max_pages: usize,
    render_width: u32,
) -> Result<(usize, Vec<(usize, String)>)> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(data, None)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    let pages = document.pages();
    let total = pages.len() as usize;
    let mut results = Vec::new();

    let indices_to_render: Vec<usize> = if let Some(indices) = page_indices {
        indices
            .iter()
            .copied()
            .filter(|&i| i < total)
            .take(max_pages)
            .collect()
    } else {
        (0..total.min(max_pages)).collect()
    };

    for i in indices_to_render {
        let page = pages
            .get(i as i32)
            .map_err(|e| anyhow::anyhow!("Failed to get page {}: {:?}", i + 1, e))?;

        let b64 = render_page_to_b64(&page, render_width)?;
        results.push((i + 1, b64)); // 1-indexed page number
    }

    Ok((total, results))
}

/// Outcome of rendering a single PDF page, isolated from its siblings — a
/// corrupt/unrenderable page fails on its own and does not abort the batch
/// (unlike `render_pdf_pages`/`render_pdf_bytes`, where one `?` inside the
/// loop short-circuits the whole call). Used by the knowledge-base scanned-
/// PDF OCR fallback, which needs per-page failure isolation to support
/// retrying just the pages that failed.
pub struct PageRenderResult {
    /// 1-indexed page number.
    pub page_number: usize,
    pub result: std::result::Result<String, String>,
}

/// Render specific PDF pages from raw bytes to base64 PNG images, isolating
/// per-page render failures instead of aborting the whole call. Failure to
/// load the document itself (corrupt file, wrong format) still bails —
/// that's a whole-file problem, not a single-page one.
///
/// `pub`（而非 `pub(crate)`）：唯一消费者 `knowledge::source` 的 PDF OCR 分页
/// 渲染随 ha-knowledge 迁出。pdfium 绑定与抽取本体留 kernel。
pub fn render_pdf_bytes_isolated(
    data: &[u8],
    page_indices: Option<&[usize]>,
    max_pages: usize,
    render_width: u32,
) -> Result<(usize, Vec<PageRenderResult>)> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(data, None)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    let pages = document.pages();
    let total = pages.len() as usize;

    let indices_to_render: Vec<usize> = if let Some(indices) = page_indices {
        indices
            .iter()
            .copied()
            .filter(|&i| i < total)
            .take(max_pages)
            .collect()
    } else {
        (0..total.min(max_pages)).collect()
    };

    let mut results = Vec::with_capacity(indices_to_render.len());
    for i in indices_to_render {
        let outcome = pages
            .get(i as i32)
            .map_err(|e| format!("failed to get page: {:?}", e))
            .and_then(|page| render_page_to_b64(&page, render_width).map_err(|e| e.to_string()));
        results.push(PageRenderResult {
            page_number: i + 1,
            result: outcome,
        });
    }

    Ok((total, results))
}

// ---------------------------------------------------------------------------
// Word (.docx) extraction
// ---------------------------------------------------------------------------

fn extract_docx_bounded(
    bytes: &[u8],
    budget: &mut FrozenExtractionBudget,
    retained_text_chars: usize,
) -> Result<(Option<String>, Vec<ExtractedImage>, bool)> {
    let archive = BoundedOfficeArchive::open(bytes, budget)?;
    let text_upper = archive
        .declared_size("word/document.xml")?
        .min(MAX_TEXT_CHARS.saturating_mul(4))
        .saturating_add(128);
    // Reserve the retained extraction/provider copies before allocating the
    // XML buffer or output String. The archive reader separately preflights
    // its descriptor index, exact entry buffer, and decoder scratch.
    let delivery_upper = projected_retained_text_bytes_upper(text_upper, retained_text_chars);
    let text_reservation = budget.reserve_provider_text(delivery_upper)?;
    budget.ensure_transient(docx_text_transient_peak(&archive, text_upper)?)?;
    let xml = archive.read_string("word/document.xml", MAX_BOUNDED_ZIP_ENTRY_BYTES, budget)?;
    let (mut text, truncated) = extract_xml_text_limited(&xml, b"w:t", MAX_TEXT_CHARS)?;
    drop(xml);
    if truncated {
        text.push_str("\n[Content truncated at the bounded typed-resource text limit]");
    }
    let text = (!text.trim().is_empty()).then_some(text);
    budget.settle_provider_text_reservation(
        text_reservation,
        text.as_deref()
            .map(|text| projected_retained_text_bytes(text, retained_text_chars))
            .unwrap_or_default(),
    )?;
    Ok((text, Vec::new(), truncated))
}

fn docx_text_transient_peak(
    archive: &BoundedOfficeArchive<'_>,
    text_upper: usize,
) -> Result<usize> {
    let document_declared = archive.declared_size("word/document.xml")?;
    archive
        .index_working_set
        .checked_add(document_declared)
        .and_then(|value| value.checked_add(text_upper))
        // quick-xml retains opened tag names in an internal Vec whose capacity
        // may grow to twice the cumulative name bytes. This remains live with
        // the borrowed XML, bounded text output, attribute decode, and text
        // unescape allocation.
        .and_then(|value| {
            xml_reader_opened_buffer_peak(document_declared)
                .and_then(|reader| value.checked_add(reader))
        })
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_ATTRIBUTE_BYTES))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_TEXT_EVENT_BYTES))
        .context("typed DOCX working-set accounting overflow")
}

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
    let text = extract_xml_text(&xml_content, b"w:t")?;
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

fn extract_xlsx_bounded(
    bytes: &[u8],
    budget: &mut FrozenExtractionBudget,
    retained_text_chars: usize,
) -> Result<(Option<String>, Vec<ExtractedImage>, bool)> {
    let archive = BoundedOfficeArchive::open(bytes, budget)?;
    let text_upper = MAX_TEXT_CHARS.saturating_mul(4).saturating_add(128);
    let delivery_upper = projected_retained_text_bytes_upper(text_upper, retained_text_chars);
    let text_reservation = budget.reserve_provider_text(delivery_upper)?;
    let shared_declared = archive
        .contains("xl/sharedStrings.xml")
        .then(|| archive.declared_size("xl/sharedStrings.xml"))
        .transpose()?
        .unwrap_or_default();
    let workbook_declared = archive.declared_size("xl/workbook.xml")?;
    let workbook_rels_declared = archive.declared_size("xl/_rels/workbook.xml.rels")?;
    let max_sheet_declared = archive
        .names()
        .filter(|name| name.starts_with("xl/worksheets/") && name.ends_with(".xml"))
        .map(|name| archive.declared_size(name))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or_default();
    let shared_structures = MAX_BOUNDED_SHARED_STRINGS
        .checked_mul(std::mem::size_of::<String>())
        .context("typed XLSX shared-string structure accounting overflow")?;
    let workbook_twice = workbook_declared
        .checked_mul(2)
        .context("typed XLSX workbook working-set accounting overflow")?;
    let relationship_reader = xml_reader_opened_buffer_peak(workbook_rels_declared)
        .context("typed XLSX relationship reader accounting overflow")?;
    let workbook_reader = xml_reader_opened_buffer_peak(workbook_declared)
        .context("typed XLSX workbook reader accounting overflow")?;
    let shared_reader = xml_reader_opened_buffer_peak(shared_declared)
        .context("typed XLSX shared-string reader accounting overflow")?;
    let sheet_reader = xml_reader_opened_buffer_peak(max_sheet_declared)
        .context("typed XLSX worksheet reader accounting overflow")?;
    let relationship_parse_peak = workbook_rels_declared
        .checked_mul(2)
        .and_then(|value| value.checked_add(relationship_reader))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_ATTRIBUTE_BYTES))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_TEXT_EVENT_BYTES))
        .context("typed XLSX relationship working-set accounting overflow")?;
    let workbook_parse_peak = workbook_rels_declared
        .checked_mul(2)
        .and_then(|value| value.checked_add(workbook_twice))
        .and_then(|value| value.checked_add(workbook_reader))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_ATTRIBUTE_BYTES))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_TEXT_EVENT_BYTES))
        .context("typed XLSX workbook working-set accounting overflow")?;
    let shared_parse_peak = shared_declared
        .checked_mul(2)
        .and_then(|value| value.checked_add(shared_reader))
        .and_then(|value| value.checked_add(workbook_declared))
        .and_then(|value| value.checked_add(workbook_rels_declared))
        .and_then(|value| value.checked_add(shared_structures))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_ATTRIBUTE_BYTES))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_TEXT_EVENT_BYTES))
        .context("typed XLSX shared-string working-set accounting overflow")?;
    let sheet_parse_peak = shared_declared
        .checked_add(max_sheet_declared)
        .and_then(|value| value.checked_add(sheet_reader))
        .and_then(|value| value.checked_add(workbook_declared))
        .and_then(|value| value.checked_add(workbook_rels_declared))
        .and_then(|value| value.checked_add(shared_structures))
        // A bounded cell value and `render_xlsx_cell`'s owned result can
        // coexist until the latter is appended to the final output.
        .and_then(|value| value.checked_add(MAX_BOUNDED_XLSX_CELL_BYTES * 2))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_ATTRIBUTE_BYTES))
        .and_then(|value| value.checked_add(MAX_BOUNDED_XML_TEXT_EVENT_BYTES))
        .and_then(|value| value.checked_add(text_upper))
        .context("typed XLSX sheet working-set accounting overflow")?;
    let parse_peak = relationship_parse_peak
        .max(workbook_parse_peak)
        .max(shared_parse_peak)
        .max(sheet_parse_peak);
    budget.ensure_transient(
        archive
            .index_working_set
            .checked_add(parse_peak)
            .context("typed XLSX working-set accounting overflow")?,
    )?;
    let relationships_xml = archive.read_string(
        "xl/_rels/workbook.xml.rels",
        MAX_BOUNDED_ZIP_ENTRY_BYTES,
        budget,
    )?;
    let relationships = parse_xlsx_relationships(&relationships_xml)?;
    drop(relationships_xml);
    let workbook_xml =
        archive.read_string("xl/workbook.xml", MAX_BOUNDED_ZIP_ENTRY_BYTES, budget)?;
    let sheets = parse_xlsx_sheets(&workbook_xml, &relationships, &archive)?;
    drop(workbook_xml);
    drop(relationships);
    let shared_strings = if archive.contains("xl/sharedStrings.xml") {
        let xml =
            archive.read_string("xl/sharedStrings.xml", MAX_BOUNDED_ZIP_ENTRY_BYTES, budget)?;
        parse_xlsx_shared_strings(&xml)?
    } else {
        Vec::new()
    };

    let mut output = String::new();
    let mut output_chars = 0usize;
    let mut truncated = false;
    for sheet in &sheets {
        for heading_part in ["=== Sheet: ", sheet.display_name.as_str(), " ===\n"] {
            if !push_limited_chars(&mut output, heading_part, &mut output_chars, MAX_TEXT_CHARS) {
                truncated = true;
            }
        }
        let xml = archive.read_string(&sheet.entry_name, MAX_BOUNDED_ZIP_ENTRY_BYTES, budget)?;
        truncated |= !append_xlsx_sheet_limited(
            &xml,
            &shared_strings,
            &mut output,
            &mut output_chars,
            MAX_TEXT_CHARS,
        )?;
        truncated |= !push_limited_chars(&mut output, "\n", &mut output_chars, MAX_TEXT_CHARS);
    }
    if truncated {
        output.push_str("\n[Content truncated at the bounded typed-resource text limit]");
    }
    let text = (!output.trim().is_empty()).then_some(output);
    budget.settle_provider_text_reservation(
        text_reservation,
        text.as_deref()
            .map(|text| projected_retained_text_bytes(text, retained_text_chars))
            .unwrap_or_default(),
    )?;
    Ok((text, Vec::new(), truncated))
}

#[derive(Debug)]
struct XlsxSheet {
    display_name: String,
    entry_name: String,
}

fn parse_xlsx_relationships(xml: &str) -> Result<HashMap<String, String>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut saw_element = false;
    let mut relationships = HashMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                enter_bounded_xml_depth(&mut depth, "typed XLSX relationships")?;
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                if event.local_name().as_ref() == b"Relationship" {
                    parse_xlsx_relationship(&reader, &event, &mut relationships)?;
                }
            }
            Ok(Event::Empty(event)) => {
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                if event.local_name().as_ref() == b"Relationship" {
                    parse_xlsx_relationship(&reader, &event, &mut relationships)?;
                }
            }
            Ok(Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .context("typed XLSX relationships have an unmatched end element")?;
            }
            Ok(Event::Text(event)) => {
                with_bounded_decoded_xml_text(&event, |_| Ok(()))?;
            }
            Ok(Event::CData(event)) => {
                event
                    .decode()
                    .map_err(|_| anyhow::anyhow!("typed Office XML CDATA cannot be decoded"))?;
            }
            Ok(Event::GeneralRef(event)) => {
                decode_xml_reference(&event)?;
            }
            Ok(Event::DocType(_)) => {
                anyhow::bail!("typed Office XML DTDs are unsupported");
            }
            Ok(Event::Eof) => {
                if depth != 0 || !saw_element {
                    anyhow::bail!("typed XLSX relationships ended with an incomplete tree");
                }
                break;
            }
            Err(_) => anyhow::bail!("typed XLSX relationships XML is malformed"),
            _ => {}
        }
    }
    Ok(relationships)
}

fn parse_xlsx_relationship(
    reader: &quick_xml::Reader<&[u8]>,
    event: &quick_xml::events::BytesStart<'_>,
    relationships: &mut HashMap<String, String>,
) -> Result<()> {
    let id = required_xml_attribute(reader, event, b"Id")?;
    let relationship_type = required_xml_attribute(reader, event, b"Type")?;
    let target = required_xml_attribute(reader, event, b"Target")?;
    let target_mode = optional_xml_attribute(reader, event, b"TargetMode")?;
    if !relationship_type.ends_with("/worksheet") {
        return Ok(());
    }
    if target_mode
        .as_deref()
        .is_some_and(|mode| mode != "Internal")
    {
        anyhow::bail!("typed XLSX worksheet relationship is external");
    }
    let entry_name = normalize_xlsx_relationship_target(&target)?;
    if relationships.insert(id, entry_name).is_some() {
        anyhow::bail!("typed XLSX contains duplicate worksheet relationship ids");
    }
    if relationships.len() > MAX_BOUNDED_ZIP_ENTRIES {
        anyhow::bail!("typed XLSX has too many worksheet relationships");
    }
    Ok(())
}

fn normalize_xlsx_relationship_target(target: &str) -> Result<String> {
    if target.contains('\\') {
        anyhow::bail!("typed XLSX worksheet relationship uses a non-canonical separator");
    }
    let candidate = if let Some(value) = target.strip_prefix('/') {
        value.to_string()
    } else if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{target}")
    };
    if candidate
        .split('/')
        .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || !candidate.starts_with("xl/worksheets/")
        || !candidate.ends_with(".xml")
    {
        anyhow::bail!("typed XLSX worksheet relationship target is invalid");
    }
    Ok(candidate)
}

fn parse_xlsx_sheets(
    xml: &str,
    relationships: &HashMap<String, String>,
    archive: &BoundedOfficeArchive<'_>,
) -> Result<Vec<XlsxSheet>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut saw_element = false;
    let mut sheets = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                enter_bounded_xml_depth(&mut depth, "typed XLSX workbook")?;
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                if event.local_name().as_ref() == b"sheet" {
                    push_xlsx_sheet(&reader, &event, relationships, archive, &mut sheets)?;
                }
            }
            Ok(Event::Empty(event)) => {
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                if event.local_name().as_ref() == b"sheet" {
                    push_xlsx_sheet(&reader, &event, relationships, archive, &mut sheets)?;
                }
            }
            Ok(Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .context("typed XLSX workbook has an unmatched end element")?;
            }
            Ok(Event::Text(event)) => {
                with_bounded_decoded_xml_text(&event, |_| Ok(()))?;
            }
            Ok(Event::CData(event)) => {
                event
                    .decode()
                    .map_err(|_| anyhow::anyhow!("typed Office XML CDATA cannot be decoded"))?;
            }
            Ok(Event::GeneralRef(event)) => {
                decode_xml_reference(&event)?;
            }
            Ok(Event::DocType(_)) => {
                anyhow::bail!("typed Office XML DTDs are unsupported");
            }
            Ok(Event::Eof) => {
                if depth != 0 || !saw_element {
                    anyhow::bail!("typed XLSX workbook ended with an incomplete tree");
                }
                break;
            }
            Err(_) => anyhow::bail!("typed XLSX workbook XML is malformed"),
            _ => {}
        }
    }
    if sheets.is_empty() {
        anyhow::bail!("typed XLSX workbook has no worksheets");
    }
    Ok(sheets)
}

fn push_xlsx_sheet(
    reader: &quick_xml::Reader<&[u8]>,
    event: &quick_xml::events::BytesStart<'_>,
    relationships: &HashMap<String, String>,
    archive: &BoundedOfficeArchive<'_>,
    sheets: &mut Vec<XlsxSheet>,
) -> Result<()> {
    if sheets.len() >= MAX_BOUNDED_ZIP_ENTRIES {
        anyhow::bail!("typed XLSX has too many worksheets");
    }
    let display_name = required_xml_attribute(reader, event, b"name")?;
    let relationship_id = required_xml_attribute(reader, event, b"id")?;
    let entry_name = relationships
        .get(&relationship_id)
        .context("typed XLSX sheet has an unknown relationship")?
        .clone();
    if !archive.contains(&entry_name) {
        anyhow::bail!("typed XLSX worksheet relationship target is missing");
    }
    if sheets
        .iter()
        .any(|sheet| sheet.display_name == display_name || sheet.entry_name == entry_name)
    {
        anyhow::bail!("typed XLSX contains duplicate worksheet bindings");
    }
    sheets.push(XlsxSheet {
        display_name,
        entry_name,
    });
    Ok(())
}

fn required_xml_attribute(
    reader: &quick_xml::Reader<&[u8]>,
    event: &quick_xml::events::BytesStart<'_>,
    name: &[u8],
) -> Result<String> {
    optional_xml_attribute(reader, event, name)?.with_context(|| {
        format!(
            "typed Office XML is missing attribute {}",
            String::from_utf8_lossy(name)
        )
    })
}

fn optional_xml_attribute(
    reader: &quick_xml::Reader<&[u8]>,
    event: &quick_xml::events::BytesStart<'_>,
    name: &[u8],
) -> Result<Option<String>> {
    let mut found = None;
    for (index, attribute) in event.attributes().enumerate() {
        let attribute =
            attribute.map_err(|_| anyhow::anyhow!("typed Office XML has a malformed attribute"))?;
        ensure_bounded_xml_attribute(&attribute, index)?;
        if attribute.key.local_name().as_ref() != name {
            continue;
        }
        if found.is_some() {
            anyhow::bail!("typed Office XML contains a duplicate attribute");
        }
        found = Some(
            attribute
                .decode_and_unescape_value(reader.decoder())
                .map_err(|_| anyhow::anyhow!("typed Office XML attribute cannot be decoded"))?
                .into_owned(),
        );
    }
    Ok(found)
}

fn validate_xml_attributes(
    reader: &quick_xml::Reader<&[u8]>,
    event: &quick_xml::events::BytesStart<'_>,
) -> Result<()> {
    if event.name().as_ref().len() > MAX_BOUNDED_XML_ELEMENT_NAME_BYTES {
        anyhow::bail!("typed Office XML element name exceeds its bounded limit");
    }
    for (index, attribute) in event.attributes().enumerate() {
        let attribute =
            attribute.map_err(|_| anyhow::anyhow!("typed Office XML has a malformed attribute"))?;
        ensure_bounded_xml_attribute(&attribute, index)?;
        attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|_| anyhow::anyhow!("typed Office XML attribute cannot be decoded"))?;
    }
    Ok(())
}

fn ensure_bounded_xml_attribute(
    attribute: &quick_xml::events::attributes::Attribute<'_>,
    zero_based_index: usize,
) -> Result<()> {
    if zero_based_index >= MAX_BOUNDED_XML_ATTRIBUTES_PER_ELEMENT {
        anyhow::bail!("typed Office XML element has too many attributes");
    }
    if attribute.value.len() > MAX_BOUNDED_XML_ATTRIBUTE_BYTES {
        anyhow::bail!("typed Office XML attribute exceeds its bounded value limit");
    }
    Ok(())
}

fn with_bounded_decoded_xml_text<T>(
    event: &quick_xml::events::BytesText<'_>,
    consume: impl FnOnce(&str) -> Result<T>,
) -> Result<T> {
    if event.len() > MAX_BOUNDED_XML_TEXT_EVENT_BYTES {
        anyhow::bail!("typed Office XML text event exceeds its bounded value limit");
    }
    let decoded = event
        .decode()
        .map_err(|_| anyhow::anyhow!("typed Office XML text cannot be decoded"))?;
    let unescaped = quick_xml::escape::unescape(&decoded)
        .map_err(|_| anyhow::anyhow!("typed Office XML text cannot be unescaped"))?;
    consume(&unescaped)
}

fn xml_reader_opened_buffer_peak(xml_bytes: usize) -> Option<usize> {
    // ReaderState::emit_start copies every opened name into a Vec. Its length
    // is bounded by the XML entry and geometric growth can retain <2x length.
    xml_bytes.checked_mul(2)
}

fn parse_xlsx_shared_strings(xml: &str) -> Result<Vec<String>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut inside_item = false;
    let mut inside_text = false;
    let mut total_chars = 0usize;
    let mut depth = 0usize;
    let mut saw_element = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                enter_bounded_xml_depth(&mut depth, "typed XLSX shared strings")?;
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                match event.local_name().as_ref() {
                    b"si" => {
                        if inside_item {
                            anyhow::bail!("typed XLSX contains nested shared-string items");
                        }
                        if strings.len() >= MAX_BOUNDED_SHARED_STRINGS {
                            anyhow::bail!("typed XLSX has too many shared strings");
                        }
                        current.clear();
                        inside_item = true;
                    }
                    b"t" if inside_item => {
                        if inside_text {
                            anyhow::bail!("typed XLSX contains nested shared-string text");
                        }
                        inside_text = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(event)) => {
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                if event.local_name().as_ref() == b"si" {
                    if strings.len() >= MAX_BOUNDED_SHARED_STRINGS {
                        anyhow::bail!("typed XLSX has too many shared strings");
                    }
                    strings.push(String::new());
                }
            }
            Ok(Event::Text(event)) => {
                with_bounded_decoded_xml_text(&event, |unescaped| {
                    if inside_text
                        && !push_limited_chars(
                            &mut current,
                            unescaped,
                            &mut total_chars,
                            MAX_TEXT_CHARS,
                        )
                    {
                        anyhow::bail!("typed XLSX shared strings exceed the bounded text limit");
                    }
                    Ok(())
                })?;
            }
            Ok(Event::CData(event)) => {
                let decoded = event
                    .decode()
                    .map_err(|_| anyhow::anyhow!("typed Office XML CDATA cannot be decoded"))?;
                if inside_text
                    && !push_limited_chars(&mut current, &decoded, &mut total_chars, MAX_TEXT_CHARS)
                {
                    anyhow::bail!("typed XLSX shared strings exceed the bounded text limit");
                }
            }
            Ok(Event::GeneralRef(event)) => {
                let decoded = decode_xml_reference(&event)?;
                if inside_text
                    && !push_limited_chars(&mut current, &decoded, &mut total_chars, MAX_TEXT_CHARS)
                {
                    anyhow::bail!("typed XLSX shared strings exceed the bounded text limit");
                }
            }
            Ok(Event::DocType(_)) => {
                anyhow::bail!("typed Office XML DTDs are unsupported");
            }
            Ok(Event::End(event)) => {
                match event.local_name().as_ref() {
                    b"t" => {
                        if !inside_text {
                            anyhow::bail!("typed XLSX shared-string text state is malformed");
                        }
                        inside_text = false;
                    }
                    b"si" => {
                        if !inside_item || inside_text {
                            anyhow::bail!("typed XLSX shared-string item state is malformed");
                        }
                        strings.push(std::mem::take(&mut current));
                        inside_item = false;
                    }
                    _ => {}
                }
                depth = depth
                    .checked_sub(1)
                    .context("typed XLSX shared strings have an unmatched end element")?;
            }
            Ok(Event::Eof) => {
                if depth != 0 || inside_item || inside_text || !saw_element {
                    anyhow::bail!("typed XLSX shared strings ended with an incomplete tree");
                }
                break;
            }
            Err(_) => anyhow::bail!("typed XLSX shared-strings XML is malformed"),
            _ => {}
        }
    }
    Ok(strings)
}

fn append_xlsx_sheet_limited(
    xml: &str,
    shared_strings: &[String],
    output: &mut String,
    output_chars: &mut usize,
    max_chars: usize,
) -> Result<bool> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut cell_type = None::<String>;
    let mut cell_value = String::new();
    let mut cell_value_chars = 0usize;
    let mut cell_value_truncated = false;
    let mut inside_cell = false;
    let mut inside_value = false;
    let mut cell_count = 0usize;
    let mut depth = 0usize;
    let mut saw_element = false;
    let mut complete = true;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                enter_bounded_xml_depth(&mut depth, "typed XLSX worksheet")?;
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                match event.local_name().as_ref() {
                    b"c" => {
                        if inside_cell {
                            anyhow::bail!("typed XLSX contains nested cells");
                        }
                        cell_count = cell_count
                            .checked_add(1)
                            .context("typed XLSX cell count overflow")?;
                        if cell_count > 50_000 {
                            anyhow::bail!("typed XLSX has too many cells for bounded extraction");
                        }
                        cell_type = optional_xml_attribute(&reader, &event, b"t")?;
                        cell_value.clear();
                        cell_value_chars = 0;
                        cell_value_truncated = false;
                        inside_cell = true;
                    }
                    b"v" | b"t" if inside_cell => {
                        if inside_value {
                            anyhow::bail!("typed XLSX contains nested cell values");
                        }
                        inside_value = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(event)) => {
                saw_element = true;
                validate_xml_attributes(&reader, &event)?;
                if event.local_name().as_ref() == b"c" {
                    if inside_cell {
                        anyhow::bail!("typed XLSX contains nested cells");
                    }
                    cell_count = cell_count
                        .checked_add(1)
                        .context("typed XLSX cell count overflow")?;
                    if cell_count > 50_000 {
                        anyhow::bail!("typed XLSX has too many cells for bounded extraction");
                    }
                    // Parse every attribute even though an empty cell has no
                    // rendered value; malformed attributes must not be lost.
                    let _ = optional_xml_attribute(&reader, &event, b"t")?;
                    complete &= push_limited_chars(output, "\t", output_chars, max_chars);
                } else if event.local_name().as_ref() == b"row" {
                    complete &= push_limited_chars(output, "\n", output_chars, max_chars);
                }
            }
            Ok(Event::Text(event)) => {
                with_bounded_decoded_xml_text(&event, |unescaped| {
                    if inside_value
                        && !push_limited_chars(
                            &mut cell_value,
                            unescaped,
                            &mut cell_value_chars,
                            MAX_BOUNDED_XLSX_CELL_CHARS,
                        )
                    {
                        cell_value_truncated = true;
                    }
                    Ok(())
                })?;
            }
            Ok(Event::CData(event)) => {
                let decoded = event
                    .decode()
                    .map_err(|_| anyhow::anyhow!("typed Office XML CDATA cannot be decoded"))?;
                if inside_value
                    && !push_limited_chars(
                        &mut cell_value,
                        &decoded,
                        &mut cell_value_chars,
                        MAX_BOUNDED_XLSX_CELL_CHARS,
                    )
                {
                    cell_value_truncated = true;
                }
            }
            Ok(Event::GeneralRef(event)) => {
                let decoded = decode_xml_reference(&event)?;
                if inside_value
                    && !push_limited_chars(
                        &mut cell_value,
                        &decoded,
                        &mut cell_value_chars,
                        MAX_BOUNDED_XLSX_CELL_CHARS,
                    )
                {
                    cell_value_truncated = true;
                }
            }
            Ok(Event::DocType(_)) => {
                anyhow::bail!("typed Office XML DTDs are unsupported");
            }
            Ok(Event::End(event)) => {
                match event.local_name().as_ref() {
                    b"v" | b"t" if inside_cell => {
                        if !inside_value {
                            anyhow::bail!("typed XLSX cell value state is malformed");
                        }
                        inside_value = false;
                    }
                    b"c" => {
                        if !inside_cell || inside_value {
                            anyhow::bail!("typed XLSX cell state is malformed");
                        }
                        complete &= !cell_value_truncated;
                        let rendered =
                            render_xlsx_cell(cell_type.as_deref(), &cell_value, shared_strings)?;
                        complete &= push_limited_chars(output, &rendered, output_chars, max_chars);
                        complete &= push_limited_chars(output, "\t", output_chars, max_chars);
                        inside_cell = false;
                        cell_type = None;
                    }
                    b"row" => {
                        complete &= push_limited_chars(output, "\n", output_chars, max_chars);
                    }
                    _ => {}
                }
                depth = depth
                    .checked_sub(1)
                    .context("typed XLSX worksheet has an unmatched end element")?;
            }
            Ok(Event::Eof) => {
                if depth != 0 || inside_cell || inside_value || !saw_element {
                    anyhow::bail!("typed XLSX worksheet ended with an incomplete tree");
                }
                break;
            }
            Err(_) => anyhow::bail!("typed XLSX worksheet XML is malformed"),
            _ => {}
        }
    }
    Ok(complete)
}

fn render_xlsx_cell(
    cell_type: Option<&str>,
    value: &str,
    shared_strings: &[String],
) -> Result<String> {
    match cell_type {
        Some("s") => {
            let index = value
                .parse::<usize>()
                .context("typed XLSX shared-string reference is not an index")?;
            shared_strings
                .get(index)
                .cloned()
                .context("typed XLSX shared-string reference is out of bounds")
        }
        Some("b") => match value {
            "0" => Ok("false".to_string()),
            "1" => Ok("true".to_string()),
            _ => anyhow::bail!("typed XLSX boolean cell has an invalid value"),
        },
        Some("e") => {
            let label = match value {
                "#NULL!" => "Null",
                "#DIV/0!" => "Div0",
                "#VALUE!" => "Value",
                "#REF!" => "Ref",
                "#NAME?" => "Name",
                "#NUM!" => "Num",
                "#N/A" => "NA",
                "#GETTING_DATA" => "GettingData",
                _ => anyhow::bail!("typed XLSX error cell has an invalid value"),
            };
            Ok(format!("#ERR:{label}"))
        }
        Some("inlineStr" | "str" | "d") => Ok(value.to_string()),
        None | Some("n") if value.is_empty() => Ok(String::new()),
        None | Some("n") => {
            let number = value
                .parse::<f64>()
                .context("typed XLSX numeric cell has an invalid value")?;
            if !number.is_finite() {
                anyhow::bail!("typed XLSX numeric cell is not finite");
            }
            if number == (number as i64) as f64 {
                Ok((number as i64).to_string())
            } else {
                Ok(number.to_string())
            }
        }
        Some(_) => anyhow::bail!("typed XLSX cell uses an unsupported type"),
    }
}

fn extract_excel(path: &Path) -> Result<(Option<String>, Vec<ExtractedImage>)> {
    use calamine::{open_workbook_auto, Data, Reader};

    let mut workbook =
        open_workbook_auto(path).map_err(|e| anyhow::anyhow!("Failed to open workbook: {}", e))?;

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

fn extract_pptx_bounded(
    bytes: &[u8],
    budget: &mut FrozenExtractionBudget,
    include_images: bool,
    retained_text_chars: usize,
) -> Result<(Option<String>, Vec<ExtractedImage>, bool)> {
    let archive = BoundedOfficeArchive::open(bytes, budget)?;
    let mut slide_names = archive
        .names()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut media_names = archive
        .names()
        .filter(|name| name.starts_with("ppt/media/") && guess_image_mime(name).is_some())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    slide_names.sort_by_key(|name| office_numbered_entry_index(name, "slide"));
    media_names.sort();

    let slide_xml_bytes = archive
        .declared_size_for(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))?;
    let text_upper = slide_xml_bytes
        .min(MAX_TEXT_CHARS.saturating_mul(4))
        .saturating_add(slide_names.len().saturating_mul(32))
        .saturating_add(128);
    let delivery_upper = projected_retained_text_bytes_upper(text_upper, retained_text_chars);
    let text_reservation = budget.reserve_provider_text(delivery_upper)?;
    let max_slide_declared = slide_names
        .iter()
        .map(|name| archive.declared_size(name))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or_default();
    let max_slide_text = max_slide_declared.min(MAX_TEXT_CHARS.saturating_mul(4));
    budget.ensure_transient(
        archive
            .index_working_set
            .checked_add(max_slide_declared)
            .and_then(|value| {
                xml_reader_opened_buffer_peak(max_slide_declared)
                    .and_then(|reader| value.checked_add(reader))
            })
            .and_then(|value| value.checked_add(text_upper))
            .and_then(|value| value.checked_add(max_slide_text))
            .and_then(|value| value.checked_add(MAX_BOUNDED_XML_ATTRIBUTE_BYTES))
            .and_then(|value| value.checked_add(MAX_BOUNDED_XML_TEXT_EVENT_BYTES))
            .context("typed PPTX text working-set accounting overflow")?,
    )?;

    let mut text = String::new();
    let mut text_chars = 0usize;
    let mut text_truncated = false;
    for (ordinal, slide_name) in slide_names.iter().enumerate() {
        if text_chars >= MAX_TEXT_CHARS {
            text_truncated = true;
            break;
        }
        let heading = format!("--- Slide {} ---\n", ordinal + 1);
        if !push_limited_chars(&mut text, &heading, &mut text_chars, MAX_TEXT_CHARS) {
            text_truncated = true;
            break;
        }
        let xml = archive.read_string(slide_name, MAX_BOUNDED_ZIP_ENTRY_BYTES, budget)?;
        let (slide_text, truncated) =
            extract_xml_text_limited(&xml, b"a:t", MAX_TEXT_CHARS.saturating_sub(text_chars))?;
        if !push_limited_chars(
            &mut text,
            slide_text.trim(),
            &mut text_chars,
            MAX_TEXT_CHARS,
        ) || !push_limited_chars(&mut text, "\n\n", &mut text_chars, MAX_TEXT_CHARS)
            || truncated
        {
            text_truncated = true;
            break;
        }
    }
    if text_truncated {
        text.push_str("\n[Content truncated at the bounded typed-resource text limit]");
    }
    let text = (!text.trim().is_empty()).then_some(text);
    budget.settle_provider_text_reservation(
        text_reservation,
        text.as_deref()
            .map(|text| projected_retained_text_bytes(text, retained_text_chars))
            .unwrap_or_default(),
    )?;
    let mut images = Vec::new();
    if !include_images {
        return Ok((text, images, text_truncated));
    }
    for (media_index, media_name) in media_names.into_iter().enumerate() {
        let mime_type = guess_image_mime(&media_name).expect("filtered image extension");
        let declared = archive.declared_size(&media_name)?;
        let encoded_len = declared
            .checked_add(2)
            .and_then(|value| value.checked_div(3))
            .and_then(|value| value.checked_mul(4))
            .context("typed PPTX media Base64 accounting overflow")?;
        let transient = declared
            .checked_add(encoded_len)
            .and_then(|value| value.checked_add(64 * 1024))
            .and_then(|value| value.checked_add(archive.index_working_set))
            .context("typed PPTX media accounting overflow")?;
        let mut image_budget = budget.clone();
        image_budget.reserve_extracted_image(encoded_len, mime_type.len(), transient)?;
        let media = archive.read_bytes(&media_name, MAX_BOUNDED_ZIP_ENTRY_BYTES, budget)?;
        let data = base64::engine::general_purpose::STANDARD.encode(media);
        if data.len() != encoded_len {
            anyhow::bail!("typed PPTX media changed size during Base64 encoding");
        }
        // ZIP entry names are user-controlled and can be tens of KiB. The
        // provider does not consume this label, so keep retained metadata
        // fixed-size instead of cloning the archive name across resources.
        let label = format!("Embedded image {}", media_index + 1);
        *budget = image_budget;
        images.push(ExtractedImage {
            data,
            mime_type,
            label,
        });
    }
    Ok((text, images, text_truncated))
}

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
                let text = extract_xml_text(&xml_content, b"a:t")?;
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

fn office_numbered_entry_index(name: &str, prefix: &str) -> usize {
    name.rsplit('/')
        .next()
        .and_then(|value| value.strip_suffix(".xml"))
        .and_then(|value| value.strip_prefix(prefix))
        .and_then(|value| value.parse().ok())
        .unwrap_or(usize::MAX)
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
pub(crate) fn is_text_like(mime_type: &str, file_name: &str) -> bool {
    if mime_type.starts_with("text/") {
        return true;
    }
    let text_mimes = [
        "application/json",
        "application/xml",
        "application/javascript",
        "application/x-javascript",
        "application/typescript",
        "application/yaml",
        "application/x-yaml",
        "application/toml",
        "application/x-sh",
        "application/sql",
    ];
    if text_mimes.contains(&mime_type) {
        return true;
    }
    // Fallback: check file extension
    let text_extensions = [
        "txt",
        "md",
        "markdown",
        "html",
        "htm",
        "css",
        "scss",
        "js",
        "jsx",
        "ts",
        "tsx",
        "json",
        "jsonl",
        "xml",
        "yaml",
        "yml",
        "toml",
        "ini",
        "cfg",
        "conf",
        "sh",
        "bash",
        "zsh",
        "fish",
        "ps1",
        "bat",
        "cmd",
        "py",
        "rb",
        "rs",
        "go",
        "java",
        "kt",
        "scala",
        "c",
        "cpp",
        "h",
        "hpp",
        "cs",
        "swift",
        "m",
        "mm",
        "r",
        "lua",
        "pl",
        "pm",
        "php",
        "sql",
        "graphql",
        "proto",
        "cmake",
        "csv",
        "tsv",
        "log",
        "env",
        "gitignore",
        "dockerignore",
        "vue",
        "svelte",
        "astro",
    ];
    let lower_name = file_name.to_ascii_lowercase();
    if let Some(ext) = lower_name.rsplit('.').next() {
        if text_extensions.contains(&ext) {
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
    let basename = Path::new(&lower_name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&lower_name);
    stem_names.contains(&basename)
}

// ---------------------------------------------------------------------------
// XML text extraction helper (used by docx and pptx)
// ---------------------------------------------------------------------------

/// Extract all text content from elements with the given tag name in an XML string.
/// Uses quick-xml for efficient streaming parse.
fn extract_xml_text(xml: &str, target_tag: &[u8]) -> Result<String> {
    extract_xml_text_limited(xml, target_tag, usize::MAX).map(|(text, _)| text)
}

fn push_limited_chars(
    output: &mut String,
    value: &str,
    output_chars: &mut usize,
    max_chars: usize,
) -> bool {
    let remaining = max_chars.saturating_sub(*output_chars);
    let mut appended = 0usize;
    for character in value.chars().take(remaining) {
        output.push(character);
        appended += 1;
    }
    *output_chars = output_chars.saturating_add(appended);
    appended == value.chars().count()
}

fn enter_bounded_xml_depth(depth: &mut usize, label: &str) -> Result<()> {
    *depth = depth
        .checked_add(1)
        .with_context(|| format!("{label} nesting depth overflow"))?;
    if *depth > MAX_BOUNDED_XML_DEPTH {
        anyhow::bail!("{label} exceeds the bounded XML nesting depth");
    }
    Ok(())
}

/// XML text extraction with an output cap applied while events are consumed;
/// callers never build an unbounded String and truncate it afterwards.
fn extract_xml_text_limited(
    xml: &str,
    target_tag: &[u8],
    max_chars: usize,
) -> Result<(String, bool)> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut output = String::new();
    let mut target_depth = None::<usize>;
    let mut last_was_paragraph_end = false;
    let mut output_chars = 0usize;
    let mut truncated = false;
    let mut depth = 0usize;
    let mut saw_element = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                enter_bounded_xml_depth(&mut depth, "typed Office XML")?;
                saw_element = true;
                validate_xml_attributes(&reader, e)?;
                let name = e.name();
                if name.as_ref() == target_tag {
                    if target_depth.is_some() {
                        anyhow::bail!("typed Office XML contains nested text elements");
                    }
                    target_depth = Some(depth);
                    last_was_paragraph_end = false;
                }
                // Track paragraph boundaries for docx (<w:p>) and pptx (<a:p>)
                if name.as_ref() == b"w:p" || name.as_ref() == b"a:p" {
                    if !output.is_empty() && last_was_paragraph_end {
                        truncated |=
                            !push_limited_chars(&mut output, "\n", &mut output_chars, max_chars);
                    }
                    last_was_paragraph_end = false;
                }
            }
            Ok(Event::Empty(ref e)) => {
                saw_element = true;
                validate_xml_attributes(&reader, e)?;
                let name = e.name();
                // An empty target element has no body and must not leak the
                // parser into subsequent unrelated text.
                if name.as_ref() == target_tag {
                    last_was_paragraph_end = false;
                }
                if name.as_ref() == b"w:p" || name.as_ref() == b"a:p" {
                    last_was_paragraph_end = true;
                }
            }
            Ok(Event::Text(ref e)) => {
                // quick-xml 0.39 拆分了 BytesText::unescape：decode() 处理字符编码，
                // escape::unescape() 处理 XML 实体，二者组合等价于旧的 unescape()。
                with_bounded_decoded_xml_text(e, |text| {
                    if target_depth.is_some()
                        && !push_limited_chars(&mut output, text, &mut output_chars, max_chars)
                    {
                        truncated = true;
                    }
                    Ok(())
                })?;
            }
            Ok(Event::CData(ref e)) => {
                let decoded = e
                    .decode()
                    .map_err(|_| anyhow::anyhow!("typed Office XML CDATA cannot be decoded"))?;
                if target_depth.is_some()
                    && !push_limited_chars(&mut output, &decoded, &mut output_chars, max_chars)
                {
                    truncated = true;
                }
            }
            Ok(Event::GeneralRef(ref e)) => {
                let text = decode_xml_reference(e)?;
                if target_depth.is_some()
                    && !push_limited_chars(&mut output, &text, &mut output_chars, max_chars)
                {
                    truncated = true;
                }
            }
            Ok(Event::DocType(_)) => {
                anyhow::bail!("typed Office XML DTDs are unsupported");
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                if name.as_ref() == target_tag {
                    if target_depth != Some(depth) {
                        anyhow::bail!("typed Office XML text element state is malformed");
                    }
                    target_depth = None;
                }
                if name.as_ref() == b"w:p" || name.as_ref() == b"a:p" {
                    last_was_paragraph_end = true;
                }
                depth = depth
                    .checked_sub(1)
                    .context("typed Office XML has an unmatched closing element")?;
            }
            Ok(Event::Eof) => {
                if depth != 0 || target_depth.is_some() || !saw_element {
                    anyhow::bail!("typed Office XML ended with an incomplete element tree");
                }
                break;
            }
            Err(_) => anyhow::bail!("typed Office XML is malformed"),
            _ => {}
        }
    }

    Ok((output, truncated))
}

fn decode_xml_reference(reference: &quick_xml::events::BytesRef<'_>) -> Result<String> {
    if reference.len() > MAX_BOUNDED_XML_REFERENCE_BYTES {
        anyhow::bail!("typed Office XML reference exceeds its bounded name limit");
    }
    if let Some(character) = reference
        .resolve_char_ref()
        .map_err(|_| anyhow::anyhow!("typed Office XML reference is malformed"))?
    {
        return Ok(character.to_string());
    }
    let name = reference
        .decode()
        .map_err(|_| anyhow::anyhow!("typed Office XML reference cannot be decoded"))?;
    quick_xml::escape::resolve_predefined_entity(&name)
        .map(ToOwned::to_owned)
        .context("typed Office XML uses an unsupported entity")
}

#[cfg(test)]
mod bounded_tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;

    use super::*;

    fn office_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            writer
                .start_file(*name, options)
                .expect("start Office entry");
            writer
                .write_all(content.as_bytes())
                .expect("write Office entry");
        }
        writer.finish().expect("finish Office ZIP").into_inner()
    }

    fn docx(text: &str) -> Vec<u8> {
        office_zip(&[(
            "word/document.xml",
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p><w:p><w:r><w:t>Tail &amp; end</w:t></w:r></w:p></w:body></w:document>"
            ),
        )])
    }

    fn central_records(bytes: &[u8]) -> Vec<(usize, usize)> {
        let (_, entries, central_offset, central_size) = zip_end_record(bytes).expect("EOCD");
        let central_end = central_offset + central_size;
        let mut cursor = central_offset;
        let mut records = Vec::new();
        for _ in 0..entries {
            let local = read_le_u32(bytes, cursor + 42).expect("local offset") as usize;
            records.push((cursor, local));
            let variable = read_le_u16(bytes, cursor + 28).unwrap() as usize
                + read_le_u16(bytes, cursor + 30).unwrap() as usize
                + read_le_u16(bytes, cursor + 32).unwrap() as usize;
            cursor += 46 + variable;
        }
        assert_eq!(cursor, central_end);
        records
    }

    fn central_record_named(bytes: &[u8], expected: &str) -> (usize, usize) {
        central_records(bytes)
            .into_iter()
            .find(|(central, _)| {
                let name_len = read_le_u16(bytes, *central + 28).unwrap() as usize;
                bytes.get(*central + 46..*central + 46 + name_len) == Some(expected.as_bytes())
            })
            .expect("named central record")
    }

    fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn extract_office(bytes: &[u8], name: &str, mime: &str) -> Result<FileContent> {
        extract_frozen_bytes_bounded(
            bytes,
            name,
            mime,
            &mut FrozenExtractionBudget::new(128 * 1024 * 1024),
            false,
            8_000,
        )
    }

    #[test]
    fn bounded_docx_preserves_paragraphs_entities_and_empty_target_state() {
        let bytes = docx("Hello<w:t/> world");
        let content = extract_office(
            &bytes,
            "demo.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )
        .expect("bounded DOCX");
        let text = content.text.expect("DOCX text");
        assert!(text.contains("Hello world"));
        assert!(text.contains("\nTail & end"));
        assert!(!content.extraction_truncated);
    }

    #[test]
    fn bounded_docx_marks_extraction_ceiling_explicitly() {
        let content = extract_office(
            &docx(&"x".repeat(MAX_TEXT_CHARS + 1)),
            "large.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )
        .expect("bounded DOCX");
        assert!(content.extraction_truncated);
        assert!(content
            .text
            .as_deref()
            .is_some_and(|text| text.contains("Content truncated")));
    }

    #[test]
    fn bounded_docx_reservation_covers_one_char_over_preview_threshold() {
        let content = extract_office(
            &docx(&"x".repeat(8_001)),
            "threshold.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )
        .expect("near-threshold DOCX");
        assert!(content.text.is_some());
    }

    #[test]
    fn bounded_docx_rejects_one_byte_below_declared_transient_peak() {
        let bytes = docx("critical peak");
        let probe_budget = FrozenExtractionBudget::new(usize::MAX / 4);
        let archive = BoundedOfficeArchive::open(&bytes, &probe_budget).expect("Office index");
        let text_upper = archive
            .declared_size("word/document.xml")
            .unwrap()
            .min(MAX_TEXT_CHARS.saturating_mul(4))
            .saturating_add(128);
        let delivery_upper = projected_retained_text_bytes_upper(text_upper, 8_000);
        let retained = probe_budget
            .provider_text_charge(delivery_upper)
            .expect("provider charge");
        let transient = docx_text_transient_peak(&archive, text_upper).expect("transient peak");
        drop(archive);

        let mut budget = FrozenExtractionBudget::new(
            retained
                .checked_add(transient)
                .and_then(|value| value.checked_sub(1))
                .expect("critical budget"),
        );
        let error = extract_docx_bounded(&bytes, &mut budget, 8_000)
            .expect_err("peak must be rejected before XML allocation/decompression");
        assert!(error.to_string().contains("memory budget"));
    }

    #[test]
    fn bounded_pptx_and_xlsx_keep_common_document_semantics() {
        let pptx = office_zip(&[
            (
                "ppt/slides/slide2.xml",
                "<p:sld><a:p><a:r><a:t>Second</a:t></a:r></a:p></p:sld>",
            ),
            (
                "ppt/slides/slide1.xml",
                "<p:sld><a:p><a:r><a:t>First &amp; one</a:t></a:r></a:p></p:sld>",
            ),
        ]);
        let ppt = extract_office(
            &pptx,
            "deck.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        )
        .expect("bounded PPTX")
        .text
        .expect("PPTX text");
        assert!(ppt.find("Slide 1").unwrap() < ppt.find("Slide 2").unwrap());
        assert!(ppt.contains("First & one"));

        let xlsx = office_zip(&[
            (
                "xl/_rels/workbook.xml.rels",
                "<Relationships><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
            ),
            (
                "xl/workbook.xml",
                "<workbook xmlns:r=\"r\"><sheets><sheet name=\"Summary &amp; Plan\" r:id=\"rId2\"/><sheet name=\"Raw\" r:id=\"rId1\"/></sheets></workbook>",
            ),
            (
                "xl/sharedStrings.xml",
                "<sst><si><t>Shared &amp; safe</t></si></sst>",
            ),
            (
                "xl/worksheets/sheet2.xml",
                "<worksheet><sheetData><row><c t=\"s\"><v>0</v></c><c t=\"b\"><v>1</v></c><c t=\"inlineStr\"><is><t>Inline</t></is></c><c t=\"e\"><v>#DIV/0!</v></c><c><v>42.0</v></c></row></sheetData></worksheet>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                "<worksheet><sheetData><row><c><v>42</v></c></row></sheetData></worksheet>",
            ),
        ]);
        let xls = extract_office(
            &xlsx,
            "book.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )
        .expect("bounded XLSX")
        .text
        .expect("XLSX text");
        assert!(xls.find("Summary & Plan").unwrap() < xls.find("Raw").unwrap());
        assert!(xls.contains("Shared & safe\ttrue\tInline\t#ERR:Div0\t42"));
    }

    #[test]
    fn bounded_pptx_media_label_does_not_retain_a_user_controlled_zip_name() {
        let sentinel = "private-media-name-".repeat(1_000);
        let media_name = format!("ppt/media/{sentinel}.png");
        let bytes = office_zip(&[
            (
                "ppt/slides/slide1.xml",
                "<p:sld><a:p><a:r><a:t>safe</a:t></a:r></a:p></p:sld>",
            ),
            (media_name.as_str(), "x"),
        ]);
        let content = extract_frozen_bytes_bounded(
            &bytes,
            "deck.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            &mut FrozenExtractionBudget::new(128 * 1024 * 1024),
            true,
            8_000,
        )
        .expect("bounded PPTX");
        assert_eq!(content.images.len(), 1);
        assert_eq!(content.images[0].label, "Embedded image 1");
        assert!(!content.images[0].label.contains("private-media-name"));
    }

    #[test]
    fn bounded_zip_rejects_crc_declared_size_and_aggregate_bombs() {
        let original = docx("safe");
        let (central, local) = central_records(&original)[0];

        let mut bad_crc = original.clone();
        let wrong_crc = read_le_u32(&bad_crc, central + 16).unwrap() ^ 0x00ff_00ff;
        write_u32(&mut bad_crc, central + 16, wrong_crc);
        if read_le_u16(&bad_crc, central + 8).unwrap() & (1 << 3) == 0 {
            write_u32(&mut bad_crc, local + 14, wrong_crc);
        }
        let error = extract_office(&bad_crc, "bad.docx", "application/octet-stream")
            .expect_err("CRC mismatch");
        assert!(error.to_string().contains("CRC"));

        let mut bad_size = original.clone();
        let declared = read_le_u32(&bad_size, central + 24).unwrap() + 1;
        write_u32(&mut bad_size, central + 24, declared);
        if read_le_u16(&bad_size, central + 8).unwrap() & (1 << 3) == 0 {
            write_u32(&mut bad_size, local + 22, declared);
        }
        assert!(extract_office(&bad_size, "bad.docx", "application/octet-stream").is_err());

        let mut aggregate = office_zip(&[
            ("word/document.xml", "<w:document/>"),
            ("word/a.xml", "<x/>"),
            ("word/b.xml", "<x/>"),
        ]);
        for (central, local) in central_records(&aggregate) {
            write_u32(&mut aggregate, central + 24, 25 * 1024 * 1024);
            if read_le_u16(&aggregate, central + 8).unwrap() & (1 << 3) == 0 {
                write_u32(&mut aggregate, local + 22, 25 * 1024 * 1024);
            }
        }
        let error = extract_office(&aggregate, "bomb.docx", "application/octet-stream")
            .expect_err("aggregate bomb");
        assert!(error.to_string().contains("aggregate"));
    }

    #[test]
    fn bounded_zip_accepts_legal_deflate_hint_flags() {
        let mut bytes = docx("hinted");
        for (central, local) in central_records(&bytes) {
            let flags = read_le_u16(&bytes, central + 8).unwrap() | (1 << 1);
            write_u16(&mut bytes, central + 8, flags);
            write_u16(&mut bytes, local + 6, flags);
        }
        assert!(extract_office(&bytes, "hint.docx", "application/octet-stream").is_ok());
    }

    #[test]
    fn pptx_media_budget_is_rejected_before_entry_read_or_base64_allocation() {
        let media = "z".repeat(1024 * 1024);
        let mut bytes = office_zip(&[
            (
                "ppt/slides/slide1.xml",
                "<p:sld><a:p><a:r><a:t>text</a:t></a:r></a:p></p:sld>",
            ),
            ("ppt/media/image1.png", media.as_str()),
        ]);
        let (central, local) = central_record_named(&bytes, "ppt/media/image1.png");
        let wrong_crc = read_le_u32(&bytes, central + 16).unwrap() ^ 0x00ff_00ff;
        write_u32(&mut bytes, central + 16, wrong_crc);
        if read_le_u16(&bytes, central + 8).unwrap() & (1 << 3) == 0 {
            write_u32(&mut bytes, local + 14, wrong_crc);
        }
        let error = extract_frozen_bytes_bounded(
            &bytes,
            "deck.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            &mut FrozenExtractionBudget::new(4 * 1024 * 1024),
            true,
            100,
        )
        .expect_err("image budget must reject before reading corrupt media");
        assert!(error.to_string().contains("memory budget"));
        assert!(!error.to_string().contains("CRC"));
    }

    #[test]
    fn bounded_xml_rejects_unclosed_declarations_trees_attributes_and_depth() {
        assert!(ensure_bounded_xml_encoding("<?xml version=\"1.0\" encoding=\"utf-8").is_err());
        assert!(extract_xml_text_limited("<w:p><w:t>open", b"w:t", 100).is_err());
        assert!(extract_xml_text_limited(
            "<w:p>&unknown;<w:t>otherwise safe</w:t></w:p>",
            b"w:t",
            100,
        )
        .is_err());
        assert!(extract_xml_text_limited(
            "<!DOCTYPE w:p [<!ENTITY x \"expanded\">]><w:p><w:t>&x;</w:t></w:p>",
            b"w:t",
            100,
        )
        .is_err());
        let oversized_text = "&amp;".repeat(MAX_BOUNDED_XML_TEXT_EVENT_BYTES / 5 + 1);
        let oversized_event = quick_xml::events::BytesText::from_escaped(oversized_text);
        let error = with_bounded_decoded_xml_text(&oversized_event, |_| Ok(()))
            .expect_err("raw text must be rejected before unescape allocation");
        assert!(error.to_string().contains("text event exceeds"));
        let oversized_reference = format!(
            "<w:p>&{};</w:p>",
            "a".repeat(MAX_BOUNDED_XML_REFERENCE_BYTES + 1)
        );
        let error = extract_xml_text_limited(&oversized_reference, b"w:t", 100)
            .expect_err("reference name must be bounded before error formatting");
        assert!(error.to_string().contains("reference exceeds"));
        let oversized_name = "n".repeat(MAX_BOUNDED_XML_ELEMENT_NAME_BYTES + 1);
        let error = extract_xml_text_limited(
            &format!("<{oversized_name}></{oversized_name}>"),
            b"w:t",
            100,
        )
        .expect_err("element name must be bounded after Reader allocation");
        assert!(error.to_string().contains("element name exceeds"));
        let secret_end_name = "PRIVATE-END-NAME-SENTINEL".repeat(100);
        let error = extract_xml_text_limited(&format!("<root></{secret_end_name}>"), b"w:t", 100)
            .expect_err("mismatched end name must fail with a fixed error");
        let error = error.to_string();
        assert!(!error.contains("PRIVATE-END-NAME-SENTINEL"));
        assert!(error.len() < 128);
        assert!(parse_xlsx_relationships(
            "<Relationships><Relationship Id=\"broken Type=\"x\"/></Relationships>"
        )
        .is_err());
        let mut deep = "<root>".to_string();
        deep.push_str(&"<x>".repeat(MAX_BOUNDED_XML_DEPTH));
        deep.push_str(&"</x>".repeat(MAX_BOUNDED_XML_DEPTH));
        deep.push_str("</root>");
        assert!(extract_xml_text_limited(&deep, b"w:t", 100).is_err());

        let huge_attribute = "&amp;".repeat(MAX_BOUNDED_XML_ATTRIBUTE_BYTES / 5 + 1);
        let bytes = office_zip(&[(
            "word/document.xml",
            &format!(
                "<w:document oversized=\"{huge_attribute}\"><w:p><w:t>safe</w:t></w:p></w:document>"
            ),
        )]);
        let error = extract_office(&bytes, "attribute.docx", "application/octet-stream")
            .expect_err("oversized attribute must fail before unescape allocation");
        assert!(error.to_string().contains("attribute exceeds"));
    }

    #[test]
    fn shared_text_like_catalog_covers_code_data_and_extensionless_files() {
        for name in [
            "query.sql",
            "types.d.ts",
            "settings.toml",
            "script.zsh",
            "style.scss",
            "Makefile",
            "Dockerfile",
        ] {
            assert!(is_text_like("application/octet-stream", name), "{name}");
        }
        assert!(!is_text_like("application/octet-stream", "archive.bin"));
    }
}
