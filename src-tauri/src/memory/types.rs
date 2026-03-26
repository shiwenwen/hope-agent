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

/// Result of a batch import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub created: usize,
    pub skipped_duplicate: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}
