// ── Tier 1: Tool Result Truncation ──

use super::config::CompactConfig;
use super::estimation::{set_tool_result_unit_text, tool_result_units};
use super::{
    CHARS_PER_TOKEN, HARD_MAX_TOOL_RESULT_CHARS, MIDDLE_OMISSION_MARKER, MIN_KEEP_CHARS,
    TRUNCATION_SUFFIX,
};
use serde_json::Value;

/// Detect if text tail contains important content (errors, JSON closing, results).
/// Reference: openclaw hasImportantTail()
fn has_important_tail(text: &str) -> bool {
    let tail = crate::truncate_utf8_tail(text, 2000);
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

/// Snap `pos` down to the nearest valid UTF-8 char boundary (≤ pos).
fn floor_char_boundary(text: &str, pos: usize) -> usize {
    let mut p = pos.min(text.len());
    while p > 0 && !text.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Snap `pos` up to the nearest valid UTF-8 char boundary (≥ pos).
fn ceil_char_boundary(text: &str, pos: usize) -> usize {
    let mut p = pos.min(text.len());
    while p < text.len() && !text.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// Find a clean cut point near target_pos, preferring structure boundaries.
/// Improvement over openclaw: recognizes JSON, code blocks, and paragraph boundaries.
/// Guaranteed to return a valid UTF-8 char boundary.
pub(super) fn find_structure_boundary(text: &str, target_pos: usize, search_range: f64) -> usize {
    let raw_start = (target_pos as f64 * (1.0 - search_range)) as usize;
    let search_start = floor_char_boundary(text, raw_start);
    let search_end = floor_char_boundary(text, target_pos.min(text.len()));
    if search_start >= search_end {
        return search_end;
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
    // Fallback: snap down to char boundary to avoid slicing mid-codepoint.
    floor_char_boundary(text, target_pos)
}

/// Find a forward-looking clean cut point near target_pos.
/// Guaranteed to return a valid UTF-8 char boundary.
fn find_structure_boundary_forward(text: &str, target_pos: usize, search_range: f64) -> usize {
    let search_start = ceil_char_boundary(text, target_pos.min(text.len()));
    let max_search = (text.len() as f64 * search_range) as usize;
    let search_end = ceil_char_boundary(text, (search_start + max_search).min(text.len()));
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

    // Never make a result larger than the configured ceiling. The previous
    // `max(MIN_KEEP_CHARS)` behavior expanded short results when a model had a
    // tiny or missing context window, and could append the suffix repeatedly.
    if max_chars <= TRUNCATION_SUFFIX.len() {
        return crate::truncate_utf8(text, max_chars).to_string();
    }
    let budget = max_chars.saturating_sub(TRUNCATION_SUFFIX.len());

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
fn calculate_max_tool_result_chars(context_window_tokens: u32, config: &CompactConfig) -> usize {
    let share = config.max_tool_result_context_share.clamp(0.1, 0.6);
    let max_tokens = (context_window_tokens as f64 * share) as usize;
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    max_chars.min(HARD_MAX_TOOL_RESULT_CHARS)
}

/// Truncate individual tool results that exceed the per-result budget.
/// Works across all 3 API formats.
pub fn truncate_tool_results(
    messages: &mut [Value],
    context_window: u32,
    config: &CompactConfig,
) -> usize {
    // A zero window means the model configuration is unresolved. It is not a
    // valid zero-byte result budget; fail closed here and let request capacity
    // validation report the configuration error.
    if context_window == 0 {
        return 0;
    }
    let max_chars = calculate_max_tool_result_chars(context_window, config);
    let mut truncated_count = 0;

    for msg in messages.iter_mut() {
        let units = tool_result_units(msg);
        for unit in units {
            if let Some(text) = unit.text {
                if text.len() > max_chars {
                    if crate::tools::image_markers::contains_image_marker(&text) {
                        if crate::tools::image_markers::has_valid_image_markers(&text) {
                            continue;
                        }
                        if set_tool_result_unit_text(
                            msg,
                            unit.locator,
                            "[Invalid or truncated image tool result omitted]",
                        ) {
                            truncated_count += 1;
                        }
                        continue;
                    }
                    let truncated = head_tail_truncate(&text, max_chars);
                    if set_tool_result_unit_text(msg, unit.locator, &truncated) {
                        truncated_count += 1;
                    }
                }
            }
        }
    }

    truncated_count
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncates_every_oversized_anthropic_result_unit_independently() {
        let mut messages = vec![json!({
            "role": "user",
            "content": [
                {"type":"tool_result","tool_use_id":"toolu_1","content":"a".repeat(5_000)},
                {"type":"text","text":"container note"},
                {
                    "type":"tool_result",
                    "tool_use_id":"toolu_2",
                    "content":[
                        {"type":"text","text":"b".repeat(5_000)},
                        {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}
                    ]
                }
            ]
        })];
        let config = CompactConfig {
            max_tool_result_context_share: 0.1,
            ..CompactConfig::default()
        };

        let count = truncate_tool_results(&mut messages, 1_000, &config);

        assert_eq!(count, 2);
        let blocks = messages[0]["content"].as_array().unwrap();
        assert!(blocks[0]["content"].as_str().unwrap().len() < 5_000);
        assert_eq!(blocks[1]["text"], "container note");
        let second_content = blocks[2]["content"].as_array().unwrap();
        assert!(second_content[0]["text"].as_str().unwrap().len() < 5_000);
        assert_eq!(second_content[1]["type"], "image");
    }

    #[test]
    fn truncates_openai_chat_and_responses_result_units() {
        let mut messages = vec![
            json!({"role":"tool","tool_call_id":"call_1","content":"a".repeat(5_000)}),
            json!({"type":"function_call_output","call_id":"fc_1","output":"b".repeat(5_000)}),
        ];
        let config = CompactConfig {
            max_tool_result_context_share: 0.1,
            ..CompactConfig::default()
        };

        let count = truncate_tool_results(&mut messages, 1_000, &config);

        assert_eq!(count, 2);
        assert!(messages[0]["content"].as_str().unwrap().len() < 5_000);
        assert!(messages[1]["output"].as_str().unwrap().len() < 5_000);
        assert_eq!(messages[0]["tool_call_id"], "call_1");
        assert_eq!(messages[1]["call_id"], "fc_1");
    }

    #[test]
    fn tiny_budget_never_expands_a_tool_result() {
        let original = "0123456789".repeat(100);
        let tiny_budget = TRUNCATION_SUFFIX.len().saturating_sub(1);

        let truncated = head_tail_truncate(&original, tiny_budget);

        assert!(truncated.len() <= tiny_budget);
        assert!(truncated.len() < original.len());
    }

    #[test]
    fn unresolved_zero_context_window_does_not_mutate_results() {
        let mut messages = vec![
            json!({"role":"tool","tool_call_id":"call_1","content":"a".repeat(5_000)}),
            json!({"type":"function_call_output","call_id":"fc_1","output":"b".repeat(5_000)}),
        ];
        let original = messages.clone();

        let count = truncate_tool_results(&mut messages, 0, &CompactConfig::default());

        assert_eq!(count, 0);
        assert_eq!(messages, original);
    }
}
