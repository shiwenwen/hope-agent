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

/// Stable location of one provider-level tool result inside a message.
///
/// Anthropic can put several `tool_result` blocks in the same user message, so
/// a message index alone is not enough to identify the result that a compaction
/// policy inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum ToolResultLocator {
    OpenAiChatContent,
    OpenAiResponsesOutput,
    AnthropicBlock(usize),
}

/// Read-only snapshot of one provider-level tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct ToolResultUnit {
    pub locator: ToolResultLocator,
    pub call_id: Option<String>,
    pub direct_tool_name: Option<String>,
    pub text: Option<String>,
}

/// Information about a tool result unit found in a message.
pub(super) struct ToolResultInfo {
    /// Index in the messages array
    pub(super) msg_index: usize,
    /// Provider-specific result location within the message.
    pub(super) locator: ToolResultLocator,
    /// Tool name (if extractable)
    #[allow(dead_code)]
    pub(super) tool_name: Option<String>,
    /// Content text length
    pub(super) content_chars: usize,
}
