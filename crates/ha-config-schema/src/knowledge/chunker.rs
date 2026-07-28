//! Chunking configuration (design D12) — the tunable half of
//! `knowledge::chunker`; the splitting algorithm itself stays in ha-core.

use serde::{Deserialize, Serialize};

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
