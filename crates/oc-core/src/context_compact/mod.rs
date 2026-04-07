// ── Context Compression & Trimming System ──────────────────────────
//
//  5-tier progressive context compression to prevent context overflow:
//   Tier 0: Microcompaction (zero-cost clearing of ephemeral tool results: ls/grep/find)
//   Tier 1: Tool result truncation (head+tail for oversized individual results)
//   Tier 2: Context pruning (soft-trim old tool results -> hard-clear with placeholder)
//   Tier 3: LLM summarization (call model to summarize old messages)
//   Tier 4: Emergency compaction (aggressive truncation on ContextOverflow)
//
//  Reference: openclaw context-pruning + compaction systems + claude-code microcompact.

mod compact;
mod config;
mod estimation;
mod pruning;
pub(crate) mod recovery;
pub(crate) mod round_grouping;
mod summarization;
mod truncation;
mod types;

// ── Hardcoded Constants (safety baselines, not user-configurable) ──

/// General text chars-per-token estimate
const CHARS_PER_TOKEN: usize = 4;
/// Tool results are more compact (openclaw: TOOL_RESULT_CHARS_PER_TOKEN_ESTIMATE = 2)
#[allow(dead_code)]
const TOOL_RESULT_CHARS_PER_TOKEN: usize = 2;
/// Image content char estimate (openclaw: IMAGE_CHAR_ESTIMATE = 8_000)
const IMAGE_CHAR_ESTIMATE: usize = 8_000;

/// Single tool result max share of context window (openclaw: MAX_TOOL_RESULT_CONTEXT_SHARE = 0.3)
const MAX_TOOL_RESULT_CONTEXT_SHARE: f64 = 0.3;
/// Hard char limit per tool result (openclaw: HARD_MAX_TOOL_RESULT_CHARS = 400_000)
const HARD_MAX_TOOL_RESULT_CHARS: usize = 400_000;
/// Minimum chars to keep when truncating (openclaw: MIN_KEEP_CHARS = 2_000)
const MIN_KEEP_CHARS: usize = 2_000;

/// Token estimate safety buffer (openclaw: SAFETY_MARGIN = 1.2)
#[allow(dead_code)]
const SAFETY_MARGIN: f64 = 1.2;
/// Reserved tokens for summarization prompt overhead
#[allow(dead_code)]
const SUMMARIZATION_OVERHEAD_TOKENS: u32 = 4096;
/// Default chunk ratio for splitting messages (openclaw: BASE_CHUNK_RATIO = 0.4)
#[allow(dead_code)]
const BASE_CHUNK_RATIO: f64 = 0.4;
/// Minimum chunk ratio for very large messages
#[allow(dead_code)]
const MIN_CHUNK_RATIO: f64 = 0.15;
/// Max chars for compaction summary (openclaw: MAX_COMPACTION_SUMMARY_CHARS = 16_000)
#[allow(dead_code)]
const MAX_COMPACTION_SUMMARY_CHARS: usize = 16_000;

/// Truncation suffix appended to truncated content
const TRUNCATION_SUFFIX: &str =
    "\n\n\u{26a0}\u{fe0f} [Content truncated \u{2014} original was too large for context window. \
     Use offset/limit to read smaller chunks.]";
/// Marker inserted between head and tail in head+tail truncation
const MIDDLE_OMISSION_MARKER: &str =
    "\n\n\u{26a0}\u{fe0f} [... middle content omitted \u{2014} showing head and tail ...]\n\n";
/// Placeholder for removed images during pruning
#[allow(dead_code)]
const PRUNED_IMAGE_MARKER: &str = "[image removed during context pruning]";
/// Marker appended when summary is too long
#[allow(dead_code)]
const SUMMARY_TRUNCATED_MARKER: &str = "\n\n[Compaction summary truncated to fit budget]";

// ── Summarization prompts ──

/// Identifier preservation instructions (strict policy)
#[allow(dead_code)]
pub(crate) const IDENTIFIER_PRESERVATION_INSTRUCTIONS: &str =
    "Preserve all opaque identifiers exactly as written (no shortening or reconstruction), \
     including UUIDs, hashes, IDs, tokens, hostnames, IPs, ports, URLs, and file names.";

/// Merge instructions for multi-part summaries
#[allow(dead_code)]
pub(crate) const MERGE_SUMMARIES_PROMPT: &str = r#"Merge these partial summaries into a single cohesive summary.

MUST PRESERVE:
- Active tasks and their current status (in-progress, blocked, pending)
- Batch operation progress (e.g., '5/17 items completed')
- The last thing the user requested and what was being done about it
- Decisions made and their rationale
- TODOs, open questions, and constraints
- Any commitments or follow-ups promised

PRIORITIZE recent context over older history."#;

// ── Re-exports ──

pub use compact::{compact_if_needed, emergency_compact, microcompact};
pub use config::CompactConfig;
pub use estimation::estimate_request_tokens;
pub use recovery::build_recovery_message;
pub use round_grouping::{prepare_messages_for_api, push_and_stamp};
pub(crate) use summarization::SUMMARIZATION_SYSTEM_PROMPT;
pub use summarization::{apply_summary, build_summarization_prompt, split_for_summarization};
pub use truncation::truncate_tool_results;
pub use types::{CompactResult, TokenEstimateCalibrator};
