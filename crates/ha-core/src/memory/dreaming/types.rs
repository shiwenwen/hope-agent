//! Shared types for the Dreaming pipeline.

use serde::{Deserialize, Serialize};

use super::triggers::DreamTrigger;

/// Summary of a single promotion decision.
/// Emitted back to the UI / diary; also written into the Dream Diary
/// markdown as a `<!-- oc-dream-promotion: ... -->` comment so the file
/// is both human-readable and machine-indexable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionRecord {
    /// ID of the promoted `MemoryEntry` in the backend.
    pub memory_id: i64,
    /// Score the LLM assigned (0.0–1.0), post-filter.
    pub score: f32,
    /// One-sentence title / headline for this memory (derived by the LLM).
    pub title: String,
    /// Short human-readable rationale for why it was promoted.
    pub rationale: String,
}

/// Terminal outcome of a dreaming cycle. Serialised into the trigger
/// response payload and logged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamReport {
    /// Which trigger fired this cycle.
    pub trigger: DreamTrigger,
    /// Total candidates scanned from the memory backend.
    pub candidates_scanned: usize,
    /// Candidates that the LLM nominated for promotion (pre-filter).
    pub candidates_nominated: usize,
    /// Candidates actually pinned after applying `min_score` and
    /// `max_promote` cutoffs.
    pub promoted: Vec<PromotionRecord>,
    /// Absolute path to the written Dream Diary markdown file (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diary_path: Option<String>,
    /// Total wall-clock duration (ms).
    pub duration_ms: u64,
    /// Human-readable notes (e.g. "no active agent", "backend empty").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
