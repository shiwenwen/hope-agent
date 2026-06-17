//! Chunking: split a note body into retrieval units (design D12).
//!
//! Phase 1 strategy is deliberately simple: split at heading boundaries, then
//! cap oversized sections by character budget. Each chunk carries D14
//! coordinates relative to the original full file and a BLAKE3 content hash that
//! drives per-chunk incremental re-embedding.

use serde::{Deserialize, Serialize};

use super::parser::{heading_path_at, ParsedDoc, PosMap};

/// Default target chunk size in code points before a section is sub-split.
pub const DEFAULT_MAX_CHUNK_CHARS: usize = 1500;
/// Default code points of tail context prepended to a sub-chunk's *search body*.
pub const DEFAULT_OVERLAP_CHARS: usize = 80;
/// Hard bounds so a bad value can't wreck retrieval (chunk too small = no
/// context, too large = poor recall / model-window overflow).
const MIN_CHUNK_CHARS: usize = 200;
const MAX_CHUNK_CHARS_LIMIT: usize = 8000;

fn default_max_chars() -> usize {
    DEFAULT_MAX_CHUNK_CHARS
}
fn default_overlap_chars() -> usize {
    DEFAULT_OVERLAP_CHARS
}

/// User-tunable chunking parameters (advanced; D12). Persisted in
/// `AppConfig.knowledge_chunk`. Changing them requires a full reindex (re-chunk
/// + re-embed) of every KB — driven by `service::set_chunk_config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkConfig {
    /// Target chunk size in code points before a section is sub-split.
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    /// Code points of tail context prepended to a sub-chunk's search body to
    /// improve recall across a split boundary.
    #[serde(default = "default_overlap_chars")]
    pub overlap_chars: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHUNK_CHARS,
            overlap_chars: DEFAULT_OVERLAP_CHARS,
        }
    }
}

impl ChunkConfig {
    /// Clamp to sane bounds: `max_chars` in `[200, 8000]`; `overlap_chars` in
    /// `[0, max_chars / 2]` (overlap larger than half the chunk is nonsensical).
    pub fn clamped(&self) -> ChunkConfig {
        let max_chars = self.max_chars.clamp(MIN_CHUNK_CHARS, MAX_CHUNK_CHARS_LIMIT);
        let overlap_chars = self.overlap_chars.min(max_chars / 2);
        ChunkConfig {
            max_chars,
            overlap_chars,
        }
    }
}

/// A produced chunk, ready to persist as a `note_chunk` row.
#[derive(Debug, Clone)]
pub struct ParsedChunk {
    pub chunk_index: i64,
    pub heading_path: Option<String>,
    /// Search text (trimmed; may carry a small overlap prefix). Decoupled from
    /// coordinates per D14.
    pub body: String,
    pub start_offset: u32,
    pub end_offset: u32,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub content_hash: String,
}

/// Chunk a note's full raw content using the parsed structure. `cfg` is assumed
/// already clamped (callers pass `ChunkConfig::clamped()`).
pub fn chunk(full: &str, parsed: &ParsedDoc, cfg: &ChunkConfig) -> Vec<ParsedChunk> {
    let posmap = PosMap::new(full);

    // Section boundaries: body start + each heading start, ascending.
    let mut bounds: Vec<usize> = vec![parsed.body_start_byte];
    for h in &parsed.headings {
        if h.byte_start >= parsed.body_start_byte {
            bounds.push(h.byte_start);
        }
    }
    bounds.sort_unstable();
    bounds.dedup();
    bounds.push(full.len());

    let mut chunks: Vec<ParsedChunk> = Vec::new();
    let mut prev_tail: String = String::new();
    let mut chunk_index: i64 = 0;

    for w in bounds.windows(2) {
        let (sec_start, sec_end) = (w[0], w[1]);
        if sec_start >= sec_end {
            continue;
        }
        let section = &full[sec_start..sec_end];
        if section.trim().is_empty() {
            continue;
        }
        let heading_path = heading_path_at(sec_start, &parsed.headings);

        for (cs, ce) in split_section(full, sec_start, sec_end, cfg.max_chars) {
            let raw = &full[cs..ce];
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Body search text with a small overlap prefix from the previous
            // chunk tail (coordinates remain the exact [cs, ce) span).
            let body = if prev_tail.is_empty() {
                trimmed.to_string()
            } else {
                format!("{}\n{}", prev_tail, trimmed)
            };
            let start = posmap.pos(cs);
            let end = posmap.pos(ce);
            chunks.push(ParsedChunk {
                chunk_index,
                heading_path: heading_path.clone(),
                content_hash: super::blake3_hex(body.as_bytes()),
                body,
                start_offset: start.offset,
                end_offset: end.offset,
                start_line: start.line,
                start_col: start.col,
                end_line: end.line,
                end_col: end.col,
            });
            chunk_index += 1;
            prev_tail = tail_chars(trimmed, cfg.overlap_chars);
        }
    }

    chunks
}

/// Split a section byte range into chunk byte ranges, capping by code-point
/// count. Splits on line boundaries so a chunk never cuts mid-line.
fn split_section(full: &str, start: usize, end: usize, max_chars: usize) -> Vec<(usize, usize)> {
    let section = &full[start..end];
    if section.chars().count() <= max_chars {
        return vec![(start, end)];
    }
    let mut out = Vec::new();
    let mut chunk_start = start;
    let mut cur_chars = 0usize;
    // Iterate line-inclusive so byte offsets stay exact.
    let mut pos = start;
    for line in section.split_inclusive('\n') {
        let line_chars = line.chars().count();
        if cur_chars > 0 && cur_chars + line_chars > max_chars {
            out.push((chunk_start, pos));
            chunk_start = pos;
            cur_chars = 0;
        }
        cur_chars += line_chars;
        pos += line.len();
    }
    if chunk_start < end {
        out.push((chunk_start, end));
    }
    out
}

/// Last `n` code points of a string.
fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    s.chars().skip(count - n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::parser::parse_document;

    #[test]
    fn one_chunk_per_short_section() {
        let full = "# A\n\nalpha body\n\n# B\n\nbeta body\n";
        let parsed = parse_document(full);
        let chunks = chunk(full, &parsed, &ChunkConfig::default());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading_path.as_deref(), Some("A"));
        assert_eq!(chunks[1].heading_path.as_deref(), Some("B"));
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[1].chunk_index, 1);
    }

    #[test]
    fn nested_heading_path() {
        let full = "# Parent\n\nintro\n\n## Child\n\ndetail\n";
        let parsed = parse_document(full);
        let chunks = chunk(full, &parsed, &ChunkConfig::default());
        let child = chunks.iter().find(|c| c.body.contains("detail")).unwrap();
        assert_eq!(child.heading_path.as_deref(), Some("Parent > Child"));
    }

    #[test]
    fn large_section_is_split() {
        let mut body = String::from("# Big\n\n");
        for i in 0..400 {
            body.push_str(&format!("line {i} with some words here\n"));
        }
        let parsed = parse_document(&body);
        let chunks = chunk(&body, &parsed, &ChunkConfig::default());
        assert!(chunks.len() > 1, "expected a large section to split");
        // chunk_index is monotonic.
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunk_index, i as i64);
        }
    }

    #[test]
    fn coords_are_full_file_relative() {
        let full = "---\ntitle: T\n---\n\n# H\n\nbody\n";
        let parsed = parse_document(full);
        let chunks = chunk(full, &parsed, &ChunkConfig::default());
        // First chunk starts at the heading line (line 5 of the full file).
        assert!(chunks[0].start_line >= 5);
    }
}
