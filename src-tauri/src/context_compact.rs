// ── Context Compression & Trimming System ──────────────────────────
//
//  4-tier progressive context compression to prevent context overflow:
//   Tier 1: Tool result truncation (head+tail for oversized individual results)
//   Tier 2: Context pruning (soft-trim old tool results → hard-clear with placeholder)
//   Tier 3: LLM summarization (call model to summarize old messages)
//   Tier 4: Emergency compaction (aggressive truncation on ContextOverflow)
//
//  Reference: openclaw context-pruning + compaction systems.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    "\n\n⚠️ [Content truncated — original was too large for context window. \
     Use offset/limit to read smaller chunks.]";
/// Marker inserted between head and tail in head+tail truncation
const MIDDLE_OMISSION_MARKER: &str =
    "\n\n⚠️ [... middle content omitted — showing head and tail ...]\n\n";
/// Placeholder for removed images during pruning
#[allow(dead_code)]
const PRUNED_IMAGE_MARKER: &str = "[image removed during context pruning]";
/// Marker appended when summary is too long
#[allow(dead_code)]
const SUMMARY_TRUNCATED_MARKER: &str = "\n\n[Compaction summary truncated to fit budget]";

// ── Summarization prompts ──

/// System prompt for context summarization (Tier 3)
#[allow(dead_code)]
pub(crate) const SUMMARIZATION_SYSTEM_PROMPT: &str = r#"You are a context compaction assistant.
Summarize the conversation history into a concise summary that preserves all critical context.

MUST PRESERVE:
- Active tasks and their current status (in-progress, blocked, pending)
- Batch operation progress (e.g., "5/17 items completed")
- The last thing the user requested and what was being done about it
- Decisions made and their rationale
- TODOs, open questions, and constraints
- Any commitments or follow-ups promised
- All file paths, function names, and code references mentioned

PRIORITIZE recent context over older history. The agent needs to know what it was doing, not just what was discussed.

Output format:
## Decisions
## Open TODOs
## Constraints/Rules
## Pending user asks
## Exact identifiers
## Conversation summary
"#;

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

// ── Configuration (user-configurable, stored in config.json) ──

fn default_true() -> bool {
    true
}
fn default_soft_trim_ratio() -> f64 {
    0.50
}
fn default_hard_clear_ratio() -> f64 {
    0.70
}
fn default_keep_last_assistants() -> usize {
    4
}
fn default_min_prunable_tool_chars() -> usize {
    20_000
}
fn default_soft_trim_max_chars() -> usize {
    6_000
}
fn default_soft_trim_head_chars() -> usize {
    2_000
}
fn default_soft_trim_tail_chars() -> usize {
    2_000
}
fn default_hard_clear_placeholder() -> String {
    "[Old tool result content cleared]".into()
}
fn default_summarization_threshold() -> f64 {
    0.85
}
fn default_preserve_recent_turns() -> usize {
    4
}
fn default_identifier_policy() -> String {
    "strict".into()
}
fn default_summarization_timeout() -> u64 {
    60
}
fn default_summary_max_tokens() -> u32 {
    4096
}
fn default_max_history_share() -> f64 {
    0.5
}

/// Context compaction configuration, stored in config.json `compact` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactConfig {
    // ── Global ──
    /// Enable context compaction (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    // ── Tier 2: Context Pruning ──
    /// Soft trim trigger ratio (default: 0.50)
    #[serde(default = "default_soft_trim_ratio")]
    pub soft_trim_ratio: f64,
    /// Hard clear trigger ratio (default: 0.70)
    #[serde(default = "default_hard_clear_ratio")]
    pub hard_clear_ratio: f64,
    /// Protect last N assistant messages from pruning (default: 4)
    #[serde(default = "default_keep_last_assistants")]
    pub keep_last_assistants: usize,
    /// Skip hard clear if total prunable chars below this (default: 20_000)
    #[serde(default = "default_min_prunable_tool_chars")]
    pub min_prunable_tool_chars: usize,
    /// Only soft-trim tool results larger than this (default: 6_000)
    #[serde(default = "default_soft_trim_max_chars")]
    pub soft_trim_max_chars: usize,
    /// Head chars to keep during soft trim (default: 2_000)
    #[serde(default = "default_soft_trim_head_chars")]
    pub soft_trim_head_chars: usize,
    /// Tail chars to keep during soft trim (default: 2_000)
    #[serde(default = "default_soft_trim_tail_chars")]
    pub soft_trim_tail_chars: usize,
    /// Enable hard clear phase (default: true)
    #[serde(default = "default_true")]
    pub hard_clear_enabled: bool,
    /// Placeholder text for hard-cleared tool results
    #[serde(default = "default_hard_clear_placeholder")]
    pub hard_clear_placeholder: String,
    /// Tool names exempt from pruning
    #[serde(default = "default_tools_deny_prune")]
    pub tools_deny_prune: Vec<String>,

    // ── Tier 3: LLM Summarization ──
    /// Summarization trigger ratio (default: 0.85)
    #[serde(default = "default_summarization_threshold")]
    pub summarization_threshold: f64,
    /// Preserve last N user turns during summarization (default: 4, max: 12)
    #[serde(default = "default_preserve_recent_turns")]
    pub preserve_recent_turns: usize,
    /// Identifier preservation policy: "strict" | "off" | "custom" (default: "strict")
    #[serde(default = "default_identifier_policy")]
    pub identifier_policy: String,
    /// Custom identifier instructions (when policy is "custom")
    #[serde(default)]
    pub identifier_instructions: Option<String>,
    /// Custom summarization instructions (appended to default prompt)
    #[serde(default)]
    pub custom_instructions: Option<String>,
    /// Summarization timeout in seconds (default: 60)
    #[serde(default = "default_summarization_timeout")]
    pub summarization_timeout_secs: u64,
    /// Max output tokens for summarization call (default: 4096)
    #[serde(default = "default_summary_max_tokens")]
    pub summary_max_tokens: u32,
    /// Max share of context window for history during pruning (default: 0.5)
    #[serde(default = "default_max_history_share")]
    pub max_history_share: f64,
}

fn default_tools_deny_prune() -> Vec<String> {
    vec![
        "web_search".into(),
        "web_fetch".into(),
        "save_memory".into(),
        "recall_memory".into(),
    ]
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            soft_trim_ratio: default_soft_trim_ratio(),
            hard_clear_ratio: default_hard_clear_ratio(),
            keep_last_assistants: default_keep_last_assistants(),
            min_prunable_tool_chars: default_min_prunable_tool_chars(),
            soft_trim_max_chars: default_soft_trim_max_chars(),
            soft_trim_head_chars: default_soft_trim_head_chars(),
            soft_trim_tail_chars: default_soft_trim_tail_chars(),
            hard_clear_enabled: default_true(),
            hard_clear_placeholder: default_hard_clear_placeholder(),
            tools_deny_prune: default_tools_deny_prune(),
            summarization_threshold: default_summarization_threshold(),
            preserve_recent_turns: default_preserve_recent_turns(),
            identifier_policy: default_identifier_policy(),
            identifier_instructions: None,
            custom_instructions: None,
            summarization_timeout_secs: default_summarization_timeout(),
            summary_max_tokens: default_summary_max_tokens(),
            max_history_share: default_max_history_share(),
        }
    }
}

// ── Token Estimate Calibrator ──

/// Calibrates token estimates using actual API usage feedback.
/// Uses exponential moving average (EMA) for smooth adaptation.
#[derive(Debug, Clone)]
pub struct TokenEstimateCalibrator {
    calibration_factor: f64,
    sample_count: u32,
}

impl TokenEstimateCalibrator {
    pub fn new() -> Self {
        Self {
            calibration_factor: 1.0,
            sample_count: 0,
        }
    }

    /// Update calibration factor with actual token count from API response.
    pub fn update(&mut self, estimated: u32, actual: u32) {
        if estimated == 0 || actual == 0 {
            return;
        }
        let ratio = actual as f64 / estimated as f64;
        // EMA with α=0.3 (recent values weighted more)
        self.calibration_factor = self.calibration_factor * 0.7 + ratio * 0.3;
        self.sample_count += 1;
    }

    /// Apply calibration to a raw estimate.
    pub fn calibrated_estimate(&self, raw_estimate: u32) -> u32 {
        (raw_estimate as f64 * self.calibration_factor) as u32
    }
}

impl Default for TokenEstimateCalibrator {
    fn default() -> Self {
        Self::new()
    }
}

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

// ── Token Estimation ──

/// Estimate token count for a JSON value using char/4 heuristic.
pub fn estimate_tokens(value: &Value) -> u32 {
    match value {
        Value::String(s) => (s.len() / CHARS_PER_TOKEN) as u32,
        Value::Array(arr) => arr.iter().map(estimate_tokens).sum(),
        Value::Object(obj) => {
            obj.values().map(estimate_tokens).sum::<u32>()
                + obj
                    .keys()
                    .map(|k| (k.len() / CHARS_PER_TOKEN) as u32)
                    .sum::<u32>()
        }
        Value::Number(_) => 1,
        Value::Bool(_) => 1,
        Value::Null => 1,
    }
}

/// Estimate char count for a message, using IMAGE_CHAR_ESTIMATE for images.
pub fn estimate_message_chars(msg: &Value) -> usize {
    if let Some(content) = msg.get("content") {
        match content {
            Value::String(s) => s.len(),
            Value::Array(arr) => arr
                .iter()
                .map(|block| {
                    if let Some(t) = block.get("type").and_then(|t| t.as_str()) {
                        match t {
                            "text" | "tool_result" => block
                                .get("text")
                                .or_else(|| block.get("content"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.len())
                                .unwrap_or(128),
                            "image" | "image_url" => IMAGE_CHAR_ESTIMATE,
                            _ => 128,
                        }
                    } else {
                        128
                    }
                })
                .sum(),
            _ => 128,
        }
    } else if let Some(output) = msg.get("output") {
        // OpenAI Responses format
        output.as_str().map(|s| s.len()).unwrap_or(128)
    } else {
        128
    }
}

/// Estimate total request tokens: system_prompt + messages + max_output.
pub fn estimate_request_tokens(
    system_prompt: &str,
    messages: &[Value],
    max_output_tokens: u32,
) -> u32 {
    let system_tokens = (system_prompt.len() / CHARS_PER_TOKEN) as u32;
    let message_tokens: u32 = messages.iter().map(|m| estimate_tokens(m)).sum();
    system_tokens + message_tokens + max_output_tokens
}

// ── Tool Result Detection (format-agnostic) ──

/// Information about a tool result found in a message.
struct ToolResultInfo {
    /// Index in the messages array
    msg_index: usize,
    /// Tool name (if extractable)
    tool_name: Option<String>,
    /// Content text length
    content_chars: usize,
}

/// Extract tool name from a message, format-agnostic.
fn extract_tool_name(msg: &Value) -> Option<String> {
    // Anthropic: look in preceding assistant message's tool_use blocks
    // For now, extract from the tool_result's own fields if available
    msg.get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

/// Get the text content of a tool result message, format-agnostic.
fn get_tool_result_text(msg: &Value) -> Option<String> {
    let role = msg.get("role").and_then(|r| r.as_str());
    let msg_type = msg.get("type").and_then(|t| t.as_str());

    // OpenAI Chat: role=tool, content is string
    if role == Some("tool") {
        return msg.get("content").and_then(|c| c.as_str()).map(|s| s.to_string());
    }

    // OpenAI Responses: type=function_call_output, output is string
    if msg_type == Some("function_call_output") {
        return msg.get("output").and_then(|o| o.as_str()).map(|s| s.to_string());
    }

    // Anthropic: role=user with content array containing tool_result blocks
    if role == Some("user") {
        if let Some(Value::Array(blocks)) = msg.get("content") {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    if let Some(content) = block.get("content") {
                        match content {
                            Value::String(s) => return Some(s.clone()),
                            Value::Array(inner) => {
                                // Array of content blocks — collect text
                                let text: String = inner
                                    .iter()
                                    .filter_map(|b| {
                                        if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                            b.get("text").and_then(|t| t.as_str())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                if !text.is_empty() {
                                    return Some(text);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    None
}

/// Set the text content of a tool result message, format-agnostic.
fn set_tool_result_text(msg: &mut Value, new_text: &str) {
    let role = msg.get("role").and_then(|r| r.as_str()).map(|s| s.to_string());
    let msg_type = msg
        .get("type")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());

    // OpenAI Chat: role=tool
    if role.as_deref() == Some("tool") {
        msg["content"] = Value::String(new_text.to_string());
        return;
    }

    // OpenAI Responses: type=function_call_output
    if msg_type.as_deref() == Some("function_call_output") {
        msg["output"] = Value::String(new_text.to_string());
        return;
    }

    // Anthropic: role=user with tool_result blocks
    if role.as_deref() == Some("user") {
        if let Some(Value::Array(blocks)) = msg.get_mut("content") {
            for block in blocks.iter_mut() {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    block["content"] = Value::String(new_text.to_string());
                    return;
                }
            }
        }
    }
}

/// Check if a message is a tool result (any format).
fn is_tool_result(msg: &Value) -> bool {
    let role = msg.get("role").and_then(|r| r.as_str());
    let msg_type = msg.get("type").and_then(|t| t.as_str());

    // OpenAI Chat
    if role == Some("tool") {
        return true;
    }
    // OpenAI Responses
    if msg_type == Some("function_call_output") {
        return true;
    }
    // Anthropic: user message containing tool_result blocks
    if role == Some("user") {
        if let Some(Value::Array(blocks)) = msg.get("content") {
            return blocks
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
        }
    }
    false
}

/// Check if a message has role=assistant.
fn is_assistant_message(msg: &Value) -> bool {
    msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
}

/// Check if a message has role=user (and is NOT a tool_result container).
fn is_user_message(msg: &Value) -> bool {
    let role = msg.get("role").and_then(|r| r.as_str());
    if role != Some("user") {
        return false;
    }
    // Exclude Anthropic tool_result containers
    !is_tool_result(msg)
}

/// Check if a tool name matches any pattern in the deny list.
fn is_tool_denied(tool_name: &str, deny_list: &[String]) -> bool {
    let lower = tool_name.to_lowercase();
    deny_list.iter().any(|pattern| {
        let p = pattern.to_lowercase();
        if p.contains('*') {
            // Simple glob: "memory_*" matches "memory_search"
            let parts: Vec<&str> = p.split('*').collect();
            if parts.len() == 2 {
                lower.starts_with(parts[0]) && lower.ends_with(parts[1])
            } else {
                lower == p
            }
        } else {
            lower == p
        }
    })
}

// ── Truncation Helpers ──

/// Detect if text tail contains important content (errors, JSON closing, results).
/// Reference: openclaw hasImportantTail()
fn has_important_tail(text: &str) -> bool {
    let tail_start = text.len().saturating_sub(2000);
    let tail = &text[tail_start..];
    let lower = tail.to_lowercase();

    // Error patterns
    let error_patterns = [
        "error", "exception", "failed", "fatal", "traceback", "panic",
        "stack trace", "errno", "exit code",
    ];
    if error_patterns.iter().any(|p| lower.contains(p)) {
        return true;
    }

    // JSON closing structure
    if tail.trim_end().ends_with('}') || tail.trim_end().ends_with(']') {
        return true;
    }

    // Result/summary patterns
    let result_patterns = ["total", "summary", "result", "complete", "finished", "done"];
    result_patterns.iter().any(|p| lower.contains(p))
}

/// Find a clean cut point near target_pos, preferring structure boundaries.
/// Improvement over openclaw: recognizes JSON, code blocks, and paragraph boundaries.
fn find_structure_boundary(text: &str, target_pos: usize, search_range: f64) -> usize {
    let search_start = (target_pos as f64 * (1.0 - search_range)) as usize;
    let search_end = target_pos.min(text.len());
    if search_start >= search_end {
        return target_pos.min(text.len());
    }
    let search_slice = &text[search_start..search_end];

    // Priority 1: Empty line (paragraph/block boundary)
    if let Some(pos) = search_slice.rfind("\n\n") {
        return search_start + pos + 2;
    }
    // Priority 2: JSON object/array closing
    if let Some(pos) = search_slice.rfind("\n}") {
        return search_start + pos + 2;
    }
    if let Some(pos) = search_slice.rfind("\n]") {
        return search_start + pos + 2;
    }
    // Priority 3: Code block ending
    if let Some(pos) = search_slice.rfind("\n```") {
        return search_start + pos + 4;
    }
    // Priority 4: Regular newline
    if let Some(pos) = search_slice.rfind('\n') {
        return search_start + pos + 1;
    }
    // Fallback: raw position
    target_pos.min(text.len())
}

/// Find a forward-looking clean cut point near target_pos.
fn find_structure_boundary_forward(text: &str, target_pos: usize, search_range: f64) -> usize {
    let search_start = target_pos.min(text.len());
    let max_search = (text.len() as f64 * search_range) as usize;
    let search_end = (search_start + max_search).min(text.len());
    if search_start >= search_end {
        return search_start;
    }
    let search_slice = &text[search_start..search_end];

    // Find first newline after target
    if let Some(pos) = search_slice.find('\n') {
        return search_start + pos + 1;
    }
    search_start
}

/// Head+tail truncation with structure-aware cut points.
/// Reference: openclaw truncateToolResultText()
fn head_tail_truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let budget = max_chars
        .saturating_sub(TRUNCATION_SUFFIX.len())
        .max(MIN_KEEP_CHARS);

    if has_important_tail(text) && budget > MIN_KEEP_CHARS * 2 {
        // Head+Tail mode: tail gets 30% but max 4000 chars
        let tail_budget = (budget * 3 / 10).min(4_000);
        let head_budget = budget
            .saturating_sub(tail_budget)
            .saturating_sub(MIDDLE_OMISSION_MARKER.len());
        if head_budget > MIN_KEEP_CHARS {
            let head_cut = find_structure_boundary(text, head_budget, 0.2);
            let tail_start = text.len().saturating_sub(tail_budget);
            let tail_cut = find_structure_boundary_forward(text, tail_start, 0.2);
            return format!(
                "{}{}{}{}",
                &text[..head_cut],
                MIDDLE_OMISSION_MARKER,
                &text[tail_cut..],
                TRUNCATION_SUFFIX
            );
        }
    }

    // Default: keep head only
    let cut = find_structure_boundary(text, budget, 0.2);
    format!("{}{}", &text[..cut], TRUNCATION_SUFFIX)
}

/// Calculate max chars for a single tool result based on context window.
fn calculate_max_tool_result_chars(context_window_tokens: u32) -> usize {
    let max_tokens = (context_window_tokens as f64 * MAX_TOOL_RESULT_CONTEXT_SHARE) as usize;
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    max_chars.min(HARD_MAX_TOOL_RESULT_CHARS)
}

/// Compute prune priority for a tool result (higher = prune first).
/// Improvement over openclaw: uses age × size instead of pure age.
fn prune_priority(msg_index: usize, total_messages: usize, content_chars: usize) -> f64 {
    let age = 1.0 - (msg_index as f64 / total_messages.max(1) as f64);
    let size = (content_chars as f64 / 100_000.0).min(1.0);
    age * 0.6 + size * 0.4
}

// ── Tier 1: Tool Result Truncation ──

/// Truncate individual tool results that exceed the per-result budget.
/// Works across all 3 API formats.
pub fn truncate_tool_results(
    messages: &mut [Value],
    context_window: u32,
    _config: &CompactConfig,
) -> usize {
    let max_chars = calculate_max_tool_result_chars(context_window);
    let mut truncated_count = 0;

    for msg in messages.iter_mut() {
        if !is_tool_result(msg) {
            continue;
        }
        if let Some(text) = get_tool_result_text(msg) {
            if text.len() > max_chars {
                let truncated = head_tail_truncate(&text, max_chars);
                set_tool_result_text(msg, &truncated);
                truncated_count += 1;
            }
        }
    }

    truncated_count
}

// ── Tier 2: Context Pruning ──

/// Result of a prune operation.
pub struct PruneResult {
    pub soft_trimmed: usize,
    pub hard_cleared: usize,
    pub chars_freed: usize,
}

/// Find the cutoff index: messages at or after this index are protected.
/// Returns None if not enough assistant messages exist.
fn find_assistant_cutoff_index(messages: &[Value], keep_last: usize) -> Option<usize> {
    let mut assistant_count = 0;
    for (i, msg) in messages.iter().enumerate().rev() {
        if is_assistant_message(msg) {
            assistant_count += 1;
            if assistant_count >= keep_last {
                return Some(i);
            }
        }
    }
    None // Not enough assistant messages
}

/// Find the first user message index (protects bootstrap context).
fn find_first_user_index(messages: &[Value]) -> Option<usize> {
    messages.iter().position(|m| is_user_message(m))
}

/// Collect info about tool results in the prunable range.
fn collect_prunable_tool_results(
    messages: &[Value],
    prune_start: usize,
    cutoff: usize,
    deny_list: &[String],
) -> Vec<ToolResultInfo> {
    let mut results = Vec::new();
    for i in prune_start..cutoff {
        let msg = &messages[i];
        if !is_tool_result(msg) {
            continue;
        }
        let tool_name = extract_tool_name(msg);
        if let Some(ref name) = tool_name {
            if is_tool_denied(name, deny_list) {
                continue;
            }
        }
        let content_chars = get_tool_result_text(msg)
            .map(|t| t.len())
            .unwrap_or(0);
        results.push(ToolResultInfo {
            msg_index: i,
            tool_name,
            content_chars,
        });
    }
    results
}

/// Tier 2: Prune old context based on usage ratio.
pub fn prune_old_context(
    messages: &mut Vec<Value>,
    system_prompt: &str,
    context_window: u32,
    max_output_tokens: u32,
    config: &CompactConfig,
) -> PruneResult {
    let mut result = PruneResult {
        soft_trimmed: 0,
        hard_cleared: 0,
        chars_freed: 0,
    };

    let char_window = context_window as usize * CHARS_PER_TOKEN;
    if char_window == 0 {
        return result;
    }

    // Step 1: Find protected boundary
    let cutoff = match find_assistant_cutoff_index(messages, config.keep_last_assistants) {
        Some(idx) => idx,
        None => return result, // Not enough assistants, skip
    };

    // Step 2: Find first user message (protect bootstrap)
    let prune_start = find_first_user_index(messages).unwrap_or(messages.len());
    if prune_start >= cutoff {
        return result; // No prunable range
    }

    // Step 3: Calculate current ratio
    let total_chars = system_prompt.len()
        + messages.iter().map(|m| estimate_message_chars(m)).sum::<usize>()
        + (max_output_tokens as usize * CHARS_PER_TOKEN);
    let ratio = total_chars as f64 / char_window as f64;

    if ratio <= config.soft_trim_ratio {
        return result; // Below threshold
    }

    // Step 4: Collect prunable tool results, sorted by priority (highest first)
    let mut prunable = collect_prunable_tool_results(messages, prune_start, cutoff, &config.tools_deny_prune);
    let total_msgs = messages.len();
    prunable.sort_by(|a, b| {
        let pa = prune_priority(a.msg_index, total_msgs, a.content_chars);
        let pb = prune_priority(b.msg_index, total_msgs, b.content_chars);
        pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Step 5: Soft trim phase
    let mut current_chars = total_chars;
    for info in &prunable {
        if info.content_chars <= config.soft_trim_max_chars {
            continue; // Too small to trim
        }
        let target_size = config.soft_trim_head_chars + config.soft_trim_tail_chars + 200; // 200 for markers
        if let Some(text) = get_tool_result_text(&messages[info.msg_index]) {
            let original_len = text.len();
            if original_len <= target_size {
                continue;
            }
            let trimmed = head_tail_truncate(&text, target_size);
            let freed = original_len - trimmed.len();
            set_tool_result_text(&mut messages[info.msg_index], &trimmed);
            current_chars = current_chars.saturating_sub(freed);
            result.soft_trimmed += 1;
            result.chars_freed += freed;

            // Re-check ratio
            let new_ratio = current_chars as f64 / char_window as f64;
            if new_ratio <= config.hard_clear_ratio {
                return result;
            }
        }
    }

    // Step 6: Hard clear phase
    if !config.hard_clear_enabled {
        return result;
    }

    let total_prunable_chars: usize = prunable.iter().map(|i| {
        get_tool_result_text(&messages[i.msg_index])
            .map(|t| t.len())
            .unwrap_or(0)
    }).sum();

    if total_prunable_chars < config.min_prunable_tool_chars {
        return result; // Not enough benefit
    }

    for info in &prunable {
        let current_ratio = current_chars as f64 / char_window as f64;
        if current_ratio <= config.hard_clear_ratio {
            break;
        }
        if let Some(text) = get_tool_result_text(&messages[info.msg_index]) {
            let original_len = text.len();
            if original_len <= config.hard_clear_placeholder.len() {
                continue; // Already cleared or too small
            }
            set_tool_result_text(&mut messages[info.msg_index], &config.hard_clear_placeholder);
            let freed = original_len - config.hard_clear_placeholder.len();
            current_chars = current_chars.saturating_sub(freed);
            result.hard_cleared += 1;
            result.chars_freed += freed;
        }
    }

    result
}

// ── Tier 4: Emergency Compaction ──

/// Aggressively compact context when ContextOverflow occurs.
/// 1. Replace ALL tool result contents with placeholders
/// 2. Keep only the last N user turns
pub fn emergency_compact(
    messages: &mut Vec<Value>,
    config: &CompactConfig,
) -> CompactResult {
    let tokens_before = messages.iter().map(|m| estimate_tokens(m)).sum::<u32>();
    let mut affected = 0;

    // Phase 1: Clear all tool results
    for msg in messages.iter_mut() {
        if is_tool_result(msg) {
            if let Some(text) = get_tool_result_text(msg) {
                if text.len() > config.hard_clear_placeholder.len() + 10 {
                    set_tool_result_text(msg, &config.hard_clear_placeholder);
                    affected += 1;
                }
            }
        }
    }

    // Phase 2: Keep only last N user turns
    let preserve = config.preserve_recent_turns.min(12).max(1);
    let mut user_count = 0;
    let mut keep_from = 0;
    for (i, msg) in messages.iter().enumerate().rev() {
        if is_user_message(msg) {
            user_count += 1;
            if user_count >= preserve {
                keep_from = i;
                break;
            }
        }
    }

    if keep_from > 0 && keep_from < messages.len() {
        let removed = keep_from;
        messages.drain(..keep_from);
        affected += removed;
    }

    let tokens_after = messages.iter().map(|m| estimate_tokens(m)).sum::<u32>();

    CompactResult {
        tier_applied: 4,
        tokens_before,
        tokens_after,
        messages_affected: affected,
        description: "emergency_compact".to_string(),
        details: Some(CompactDetails {
            tool_results_truncated: 0,
            tool_results_soft_trimmed: 0,
            tool_results_hard_cleared: affected,
            messages_summarized: 0,
            summary_tokens: None,
        }),
    }
}

// ── Main Entry Point ──

/// Apply compaction tiers as needed based on context usage.
/// This is the main entry point called before each API request.
/// Tiers 1 & 2 are synchronous. Tier 3 (LLM summarization) requires
/// async and is handled separately in agent.rs.
pub fn compact_if_needed(
    messages: &mut Vec<Value>,
    system_prompt: &str,
    context_window: u32,
    max_output_tokens: u32,
    config: &CompactConfig,
) -> CompactResult {
    if !config.enabled || context_window == 0 || messages.is_empty() {
        return CompactResult {
            tier_applied: 0,
            tokens_before: 0,
            tokens_after: 0,
            messages_affected: 0,
            description: "no_op".to_string(),
            details: None,
        };
    }

    let tokens_before = estimate_request_tokens(system_prompt, messages, max_output_tokens);
    let usage_ratio = tokens_before as f64 / context_window as f64;

    // Quick exit if well below any threshold
    if usage_ratio < config.soft_trim_ratio.min(0.3) {
        return CompactResult {
            tier_applied: 0,
            tokens_before,
            tokens_after: tokens_before,
            messages_affected: 0,
            description: "below_threshold".to_string(),
            details: None,
        };
    }

    // Tier 1: Truncate individual oversized tool results
    let tier1_count = truncate_tool_results(messages, context_window, config);

    let tokens_after_t1 = estimate_request_tokens(system_prompt, messages, max_output_tokens);
    let ratio_after_t1 = tokens_after_t1 as f64 / context_window as f64;

    if tier1_count > 0 && ratio_after_t1 < config.soft_trim_ratio {
        return CompactResult {
            tier_applied: 1,
            tokens_before,
            tokens_after: tokens_after_t1,
            messages_affected: tier1_count,
            description: "tool_results_truncated".to_string(),
            details: Some(CompactDetails {
                tool_results_truncated: tier1_count,
                tool_results_soft_trimmed: 0,
                tool_results_hard_cleared: 0,
                messages_summarized: 0,
                summary_tokens: None,
            }),
        };
    }

    // Tier 2: Context pruning (soft trim + hard clear)
    if ratio_after_t1 >= config.soft_trim_ratio {
        let prune = prune_old_context(messages, system_prompt, context_window, max_output_tokens, config);
        let tokens_after_t2 = estimate_request_tokens(system_prompt, messages, max_output_tokens);
        let ratio_after_t2 = tokens_after_t2 as f64 / context_window as f64;

        if prune.soft_trimmed > 0 || prune.hard_cleared > 0 {
            if ratio_after_t2 < config.summarization_threshold {
                return CompactResult {
                    tier_applied: 2,
                    tokens_before,
                    tokens_after: tokens_after_t2,
                    messages_affected: tier1_count + prune.soft_trimmed + prune.hard_cleared,
                    description: "context_pruned".to_string(),
                    details: Some(CompactDetails {
                        tool_results_truncated: tier1_count,
                        tool_results_soft_trimmed: prune.soft_trimmed,
                        tool_results_hard_cleared: prune.hard_cleared,
                        messages_summarized: 0,
                        summary_tokens: None,
                    }),
                };
            }
        }

        // Tier 3 needed but requires async — return a signal
        if ratio_after_t2 >= config.summarization_threshold {
            return CompactResult {
                tier_applied: 3,
                tokens_before,
                tokens_after: tokens_after_t2,
                messages_affected: tier1_count + prune.soft_trimmed + prune.hard_cleared,
                description: "summarization_needed".to_string(),
                details: Some(CompactDetails {
                    tool_results_truncated: tier1_count,
                    tool_results_soft_trimmed: prune.soft_trimmed,
                    tool_results_hard_cleared: prune.hard_cleared,
                    messages_summarized: 0,
                    summary_tokens: None,
                }),
            };
        }
    }

    // Return Tier 1 result if only truncation was done
    if tier1_count > 0 {
        return CompactResult {
            tier_applied: 1,
            tokens_before,
            tokens_after: estimate_request_tokens(system_prompt, messages, max_output_tokens),
            messages_affected: tier1_count,
            description: "tool_results_truncated".to_string(),
            details: Some(CompactDetails {
                tool_results_truncated: tier1_count,
                tool_results_soft_trimmed: 0,
                tool_results_hard_cleared: 0,
                messages_summarized: 0,
                summary_tokens: None,
            }),
        };
    }

    CompactResult {
        tier_applied: 0,
        tokens_before,
        tokens_after: tokens_before,
        messages_affected: 0,
        description: "no_action_needed".to_string(),
        details: None,
    }
}

// ── Tier 3: Summarization Helpers (used by agent.rs) ──

/// Split messages into summarizable (old) and preserved (recent) portions.
pub fn split_for_summarization(
    messages: &[Value],
    config: &CompactConfig,
) -> Option<SummarizationSplit> {
    let preserve = config.preserve_recent_turns.min(12).max(1);
    let mut user_count = 0;
    let mut boundary_index = 0;

    // Find the Nth-from-last user message as boundary
    for (i, msg) in messages.iter().enumerate().rev() {
        if is_user_message(msg) {
            user_count += 1;
            if user_count >= preserve {
                boundary_index = i;
                break;
            }
        }
    }

    if boundary_index == 0 || user_count < preserve {
        return None; // Not enough turns to summarize
    }

    let summarizable = messages[..boundary_index].to_vec();
    let preserved = messages[boundary_index..].to_vec();

    if summarizable.is_empty() {
        return None;
    }

    Some(SummarizationSplit {
        summarizable,
        preserved,
        preserved_start_index: boundary_index,
    })
}

/// Result of splitting messages for summarization.
pub struct SummarizationSplit {
    pub summarizable: Vec<Value>,
    pub preserved: Vec<Value>,
    pub preserved_start_index: usize,
}

/// Build a summarization prompt from messages to summarize.
pub fn build_summarization_prompt(
    messages_to_summarize: &[Value],
    previous_summary: Option<&str>,
    config: &CompactConfig,
) -> String {
    let mut prompt = String::new();

    // Add previous summary if exists
    if let Some(prev) = previous_summary {
        prompt.push_str("Previous conversation summary:\n");
        prompt.push_str(prev);
        prompt.push_str("\n\n---\n\n");
    }

    prompt.push_str("Conversation to summarize:\n\n");

    // Serialize messages in a readable format
    for msg in messages_to_summarize {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");

        if is_tool_result(msg) {
            if let Some(text) = get_tool_result_text(msg) {
                let preview = if text.len() > 500 {
                    format!("{}... [{}+ chars]", crate::truncate_utf8(&text, 500), text.len())
                } else {
                    text
                };
                prompt.push_str(&format!("[tool_result]: {}\n", preview));
            }
        } else if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            prompt.push_str(&format!("[{}]: {}\n", role, content));
        }
    }

    // Add identifier preservation instructions
    if config.identifier_policy != "off" {
        let instructions = if config.identifier_policy == "custom" {
            config
                .identifier_instructions
                .as_deref()
                .unwrap_or(IDENTIFIER_PRESERVATION_INSTRUCTIONS)
        } else {
            IDENTIFIER_PRESERVATION_INSTRUCTIONS
        };
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(instructions);
    }

    // Add custom instructions
    if let Some(ref custom) = config.custom_instructions {
        prompt.push_str("\n\n");
        prompt.push_str(custom);
    }

    prompt
}

/// Apply a summary: replace old messages with a summary message + preserved messages.
pub fn apply_summary(
    messages: &mut Vec<Value>,
    summary: &str,
    preserved_start_index: usize,
    _config: &CompactConfig,
) {
    // Cap summary length
    let capped_summary = if summary.len() > MAX_COMPACTION_SUMMARY_CHARS {
        let budget = MAX_COMPACTION_SUMMARY_CHARS - SUMMARY_TRUNCATED_MARKER.len();
        format!(
            "{}{}",
            &summary[..budget.min(summary.len())],
            SUMMARY_TRUNCATED_MARKER
        )
    } else {
        summary.to_string()
    };

    // Build summary message
    let summary_msg = serde_json::json!({
        "role": "user",
        "content": format!("[Previous conversation summary]\n\n{}", capped_summary)
    });

    // Keep preserved messages
    let preserved: Vec<Value> = if preserved_start_index < messages.len() {
        messages[preserved_start_index..].to_vec()
    } else {
        Vec::new()
    };

    // Replace messages
    messages.clear();
    messages.push(summary_msg);
    messages.extend(preserved);
}

/// Check if a single message is too large to safely include in a summarization call.
pub fn is_oversized_for_summary(msg: &Value, context_window: u32) -> bool {
    let tokens = estimate_tokens(msg) as f64 * SAFETY_MARGIN;
    tokens > context_window as f64 * 0.5
}

/// Compute adaptive chunk ratio based on average message size.
pub fn compute_adaptive_chunk_ratio(messages: &[Value], context_window: u32) -> f64 {
    if messages.is_empty() || context_window == 0 {
        return BASE_CHUNK_RATIO;
    }

    let total_tokens: u32 = messages.iter().map(|m| estimate_tokens(m)).sum();
    let avg_tokens = total_tokens as f64 / messages.len() as f64;
    let safe_avg = avg_tokens * SAFETY_MARGIN;
    let avg_ratio = safe_avg / context_window as f64;

    if avg_ratio > 0.1 {
        let reduction = (avg_ratio * 2.0).min(BASE_CHUNK_RATIO - MIN_CHUNK_RATIO);
        (BASE_CHUNK_RATIO - reduction).max(MIN_CHUNK_RATIO)
    } else {
        BASE_CHUNK_RATIO
    }
}

/// Split messages into chunks by token share.
pub fn split_messages_by_token_share(
    messages: &[Value],
    parts: usize,
) -> Vec<Vec<Value>> {
    if messages.is_empty() {
        return vec![];
    }
    let parts = parts.max(1).min(messages.len());
    if parts <= 1 {
        return vec![messages.to_vec()];
    }

    let total_tokens: u32 = messages.iter().map(|m| estimate_tokens(m)).sum();
    let target_tokens = total_tokens / parts as u32;
    let mut chunks: Vec<Vec<Value>> = Vec::new();
    let mut current: Vec<Value> = Vec::new();
    let mut current_tokens: u32 = 0;

    for msg in messages {
        let msg_tokens = estimate_tokens(msg);
        if chunks.len() < parts - 1
            && !current.is_empty()
            && current_tokens + msg_tokens > target_tokens
        {
            chunks.push(current);
            current = Vec::new();
            current_tokens = 0;
        }
        current.push(msg.clone());
        current_tokens += msg_tokens;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}
