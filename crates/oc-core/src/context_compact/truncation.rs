// ── Tier 1: Tool Result Truncation ──

use super::config::CompactConfig;
use super::estimation::{get_tool_result_text, is_tool_result, set_tool_result_text};
use super::{
    CHARS_PER_TOKEN, HARD_MAX_TOOL_RESULT_CHARS, MAX_TOOL_RESULT_CONTEXT_SHARE,
    MIDDLE_OMISSION_MARKER, MIN_KEEP_CHARS, TRUNCATION_SUFFIX,
};
use serde_json::Value;

/// Detect if text tail contains important content (errors, JSON closing, results).
/// Reference: openclaw hasImportantTail()
fn has_important_tail(text: &str) -> bool {
    let tail_start = text.len().saturating_sub(2000);
    let tail = &text[tail_start..];
    let lower = tail.to_lowercase();

    // Error patterns
    let error_patterns = [
        "error",
        "exception",
        "failed",
        "fatal",
        "traceback",
        "panic",
        "stack trace",
        "errno",
        "exit code",
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
pub(super) fn find_structure_boundary(text: &str, target_pos: usize, search_range: f64) -> usize {
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
pub(super) fn head_tail_truncate(text: &str, max_chars: usize) -> String {
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
