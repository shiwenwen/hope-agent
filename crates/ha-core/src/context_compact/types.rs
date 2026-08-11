// ── Types ──

use serde::Serialize;

use super::manifest::CompactionManifest;

// ── Compact Result ──

/// Result of a compaction operation, emitted as frontend event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactResult {
    /// Which tier was applied (0=no-op, 1/2/3/4)
    pub tier_applied: u8,
    /// Estimated tokens before compaction
    pub tokens_before: u32,
    /// Estimated tokens after compaction
    pub tokens_after: u32,
    /// Number of messages affected
    pub messages_affected: usize,
    /// Human-readable description
    pub description: String,
    /// Detailed breakdown
    pub details: Option<CompactDetails>,
    /// Structured observability payload for logs/events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<CompactionManifest>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactDetails {
    pub tool_results_truncated: usize,
    pub tool_results_soft_trimmed: usize,
    pub tool_results_hard_cleared: usize,
    pub messages_summarized: usize,
    pub summary_tokens: Option<u32>,
}

/// Result of a prune operation.
pub struct PruneResult {
    pub soft_trimmed: usize,
    pub hard_cleared: usize,
    pub chars_freed: usize,
}

/// Result of splitting messages for summarization.
pub struct SummarizationSplit {
    pub summarizable: Vec<serde_json::Value>,
    pub preserved: Vec<serde_json::Value>,
    pub preserved_start_index: usize,
    pub boundary_warnings: Vec<String>,
}

/// Information about a tool result found in a message.
pub(super) struct ToolResultInfo {
    /// Index in the messages array
    pub(super) msg_index: usize,
    /// Tool name (if extractable)
    #[allow(dead_code)]
    pub(super) tool_name: Option<String>,
    /// Content text length
    pub(super) content_chars: usize,
}
