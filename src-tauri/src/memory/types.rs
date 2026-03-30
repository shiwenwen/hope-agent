use serde::{Deserialize, Serialize};

// ── Data Structures ─────────────────────────────────────────────

/// Memory entry types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Information about the end user
    User,
    /// User feedback and preferences about agent behavior
    Feedback,
    /// Project-specific context and knowledge
    Project,
    /// Reference materials and external resource pointers
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "feedback" => MemoryType::Feedback,
            "project" => MemoryType::Project,
            "reference" => MemoryType::Reference,
            _ => MemoryType::User,
        }
    }

    /// Display heading for system prompt summary.
    pub(crate) fn heading(&self) -> &str {
        match self {
            MemoryType::User => "About the User",
            MemoryType::Feedback => "Preferences & Feedback",
            MemoryType::Project => "Project Context",
            MemoryType::Reference => "References",
        }
    }
}

/// Memory scope: global (shared across agents) or per-agent (private).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MemoryScope {
    Global,
    Agent { id: String },
}

/// A stored memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: i64,
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content: String,
    pub tags: Vec<String>,
    /// Source: "user" (manual), "auto" (agent-extracted), "import"
    pub source: String,
    pub source_session_id: Option<String>,
    /// Whether this memory is pinned (always prioritized in system prompt injection)
    #[serde(default)]
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Populated during search, not stored in DB
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f32>,
    /// Absolute path to attached file (image/audio), stored in ~/.opencomputer/memory_attachments/
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_path: Option<String>,
    /// MIME type of the attachment (e.g. "image/jpeg", "audio/mpeg")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_mime: Option<String>,
}

/// Input for creating a new memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub source_session_id: Option<String>,
    /// Whether this memory should be pinned (prioritized in system prompt)
    #[serde(default)]
    pub pinned: bool,
    /// Absolute path to an image/audio file attachment
    #[serde(default)]
    pub attachment_path: Option<String>,
    /// MIME type of the attachment
    #[serde(default)]
    pub attachment_mime: Option<String>,
}

fn default_source() -> String {
    "user".to_string()
}

/// Search query parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchQuery {
    /// Natural language query text
    pub query: String,
    /// Filter by type(s)
    #[serde(default)]
    pub types: Option<Vec<MemoryType>>,
    /// Filter by scope
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    /// Shorthand: load global + this agent's memories
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Max results (default 20)
    #[serde(default)]
    pub limit: Option<usize>,
}

// ── Statistics ──────────────────────────────────────────────────

/// Memory statistics for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total: usize,
    pub by_type: std::collections::HashMap<String, usize>,
    pub with_embedding: usize,
    pub oldest: Option<String>,
    pub newest: Option<String>,
}

// ── Global Memory Extract Config ────────────────────────────────

/// Global auto-extract configuration, stored in config.json `memoryExtract` field.
/// Per-agent MemoryConfig can override these with Some(...) values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractConfig {
    #[serde(default)]
    pub auto_extract: bool,
    #[serde(default = "default_extract_min_turns")]
    pub extract_min_turns: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_model_id: Option<String>,
    /// Auto-extract memories before context compaction (Tier 3 summarization)
    #[serde(default)]
    pub flush_before_compact: bool,
}

fn default_extract_min_turns() -> usize { 3 }

impl Default for MemoryExtractConfig {
    fn default() -> Self {
        Self {
            auto_extract: false,
            extract_min_turns: 3,
            extract_provider_id: None,
            extract_model_id: None,
            flush_before_compact: false,
        }
    }
}

// ── Deduplication ───────────────────────────────────────────────

/// Default dedup thresholds (RRF scores)
pub const DEDUP_THRESHOLD_HIGH: f32 = 0.02;   // Above this → duplicate, skip
pub const DEDUP_THRESHOLD_MERGE: f32 = 0.012;  // Between merge..high → update existing

/// Configurable dedup thresholds, stored in config.json `dedup` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupConfig {
    #[serde(default = "default_dedup_high")]
    pub threshold_high: f32,
    #[serde(default = "default_dedup_merge")]
    pub threshold_merge: f32,
}

fn default_dedup_high() -> f32 { DEDUP_THRESHOLD_HIGH }
fn default_dedup_merge() -> f32 { DEDUP_THRESHOLD_MERGE }

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            threshold_high: DEDUP_THRESHOLD_HIGH,
            threshold_merge: DEDUP_THRESHOLD_MERGE,
        }
    }
}

/// Result of adding a memory with deduplication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AddResult {
    /// New memory created
    Created { id: i64 },
    /// Skipped — too similar to existing entry
    Duplicate { existing_id: i64, score: f32 },
    /// Updated existing entry with new content
    Updated { id: i64 },
}

// ── Hybrid Search Config ───────────────────────────────────────

/// Configurable hybrid search weights, stored in config.json `hybridSearch` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchConfig {
    /// Weight for vector similarity results (0.0-1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    /// Weight for FTS keyword results (0.0-1.0)
    #[serde(default = "default_text_weight")]
    pub text_weight: f32,
    /// RRF constant k (higher = more equal weighting across ranks)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,
}

fn default_vector_weight() -> f32 { 0.6 }
fn default_text_weight() -> f32 { 0.4 }
fn default_rrf_k() -> f64 { 60.0 }

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.6,
            text_weight: 0.4,
            rrf_k: 60.0,
        }
    }
}

// ── Temporal Decay Config ──────────────────────────────────────

/// Temporal decay configuration for memory search scoring.
/// Recent memories rank higher; pinned memories are exempt (evergreen).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalDecayConfig {
    /// Enable temporal decay (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Half-life in days: after this many days, score is halved (default: 30)
    #[serde(default = "default_half_life_days")]
    pub half_life_days: f64,
}

fn default_half_life_days() -> f64 { 30.0 }

impl Default for TemporalDecayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            half_life_days: 30.0,
        }
    }
}

// ── MMR Config ─────────────────────────────────────────────────

/// MMR (Maximal Marginal Relevance) reranking config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MmrConfig {
    /// Enable MMR reranking (default: true)
    #[serde(default = "default_true_val")]
    pub enabled: bool,
    /// Lambda: 0 = max diversity, 1 = max relevance (default: 0.7)
    #[serde(default = "default_mmr_lambda")]
    pub lambda: f32,
}

fn default_true_val() -> bool { true }
fn default_mmr_lambda() -> f32 { 0.7 }

impl Default for MmrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lambda: 0.7,
        }
    }
}

// ── Embedding Cache Config ─────────────────────────────────────

/// Configuration for caching computed embeddings to reduce API calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingCacheConfig {
    /// Enable embedding cache (default: true)
    #[serde(default = "default_true_val")]
    pub enabled: bool,
    /// Maximum number of cached entries (default: 10000)
    #[serde(default = "default_max_cache_entries")]
    pub max_entries: usize,
}

fn default_max_cache_entries() -> usize { 10000 }

impl Default for EmbeddingCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 10000,
        }
    }
}

// ── Multimodal Config ──────────────────────────────────────────

/// Supported modalities for multimodal embedding.
pub const MULTIMODAL_IMAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"),
    ("png", "image/png"), ("webp", "image/webp"),
    ("gif", "image/gif"), ("heic", "image/heic"), ("heif", "image/heif"),
];

pub const MULTIMODAL_AUDIO_EXTENSIONS: &[(&str, &str)] = &[
    ("mp3", "audio/mpeg"), ("wav", "audio/wav"),
    ("ogg", "audio/ogg"), ("opus", "audio/opus"),
    ("m4a", "audio/mp4"), ("aac", "audio/aac"), ("flac", "audio/flac"),
];

/// Detect MIME type from file extension.
pub fn mime_from_extension(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next()?;
    for (e, mime) in MULTIMODAL_IMAGE_EXTENSIONS.iter().chain(MULTIMODAL_AUDIO_EXTENSIONS.iter()) {
        if ext == *e {
            return Some(mime.to_string());
        }
    }
    None
}

/// Get modality label ("image" or "audio") from MIME type.
pub fn modality_label(mime: &str) -> &'static str {
    if mime.starts_with("image/") { "image" }
    else if mime.starts_with("audio/") { "audio" }
    else { "file" }
}

/// Multimodal embedding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultimodalConfig {
    /// Enable multimodal embedding (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Supported modalities: "image", "audio"
    #[serde(default = "default_modalities")]
    pub modalities: Vec<String>,
    /// Max file size in bytes (default: 10MB)
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
}

fn default_modalities() -> Vec<String> { vec!["image".to_string(), "audio".to_string()] }
fn default_max_file_bytes() -> u64 { 10 * 1024 * 1024 } // 10MB

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            modalities: default_modalities(),
            max_file_bytes: default_max_file_bytes(),
        }
    }
}

/// Result of a batch import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub created: usize,
    pub skipped_duplicate: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}
