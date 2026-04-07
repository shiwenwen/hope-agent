// ── Tier 2: Context Pruning ──

use super::config::CompactConfig;
use super::estimation::{
    estimate_message_chars, extract_tool_name, get_tool_result_text, is_assistant_message,
    is_tool_result, is_user_message, set_tool_result_text,
};
use super::truncation::head_tail_truncate;
use super::types::{PruneResult, ToolResultInfo};
use super::CHARS_PER_TOKEN;
use serde_json::Value;

/// Compute prune priority for a tool result (higher = prune first).
/// Improvement over openclaw: uses age x size instead of pure age.
fn prune_priority(msg_index: usize, total_messages: usize, content_chars: usize) -> f64 {
    let age = 1.0 - (msg_index as f64 / total_messages.max(1) as f64);
    let size = (content_chars as f64 / 100_000.0).min(1.0);
    age * 0.6 + size * 0.4
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
    config: &CompactConfig,
) -> Vec<ToolResultInfo> {
    let mut results = Vec::new();
    for i in prune_start..cutoff {
        let msg = &messages[i];
        if !is_tool_result(msg) {
            continue;
        }
        let tool_name = extract_tool_name(msg);
        if let Some(ref name) = tool_name {
            if config.is_protected(name) {
                continue;
            }
        }
        let content_chars = get_tool_result_text(msg).map(|t| t.len()).unwrap_or(0);
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
        + messages
            .iter()
            .map(|m| estimate_message_chars(m))
            .sum::<usize>()
        + (max_output_tokens as usize * CHARS_PER_TOKEN);
    let ratio = total_chars as f64 / char_window as f64;

    if ratio <= config.soft_trim_ratio {
        return result; // Below threshold
    }

    // Step 4: Collect prunable tool results, sorted by priority (highest first)
    let mut prunable =
        collect_prunable_tool_results(messages, prune_start, cutoff, config);
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

    let total_prunable_chars: usize = prunable
        .iter()
        .map(|i| {
            get_tool_result_text(&messages[i.msg_index])
                .map(|t| t.len())
                .unwrap_or(0)
        })
        .sum();

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
            set_tool_result_text(
                &mut messages[info.msg_index],
                &config.hard_clear_placeholder,
            );
            let freed = original_len - config.hard_clear_placeholder.len();
            current_chars = current_chars.saturating_sub(freed);
            result.hard_cleared += 1;
            result.chars_freed += freed;
        }
    }

    result
}
