// ── Main Entry Point + Tier 0/4 Compaction ──

use std::collections::HashMap;

use super::config::CompactConfig;
use super::estimation::{
    estimate_request_tokens, estimate_tokens, get_tool_result_text, is_assistant_message,
    is_tool_result, is_user_message, set_tool_result_text,
};
use super::pruning::prune_old_context;
use super::truncation::truncate_tool_results;
use super::types::{CompactDetails, CompactResult};
use serde_json::Value;

// ── Tier 0: Microcompaction ──

/// Zero-cost clearing of ephemeral tool results (ls, grep, find, etc.)
/// that are older than the protected boundary. No LLM call required.
///
/// Returns the number of tool results cleared.
pub fn microcompact(messages: &mut [Value], config: &CompactConfig) -> usize {
    if !config.microcompact_enabled || config.microcompact_tools.is_empty() {
        return 0;
    }

    // Build a map from tool_use_id → tool_name by scanning assistant messages.
    // This handles all provider formats (Anthropic tool_use blocks, OpenAI function_call).
    let mut tool_id_to_name: HashMap<String, String> = HashMap::new();
    for msg in messages.iter() {
        if !is_assistant_message(msg) {
            continue;
        }
        // Anthropic: content array with tool_use blocks
        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    if let (Some(id), Some(name)) = (
                        block.get("id").and_then(|v| v.as_str()),
                        block.get("name").and_then(|v| v.as_str()),
                    ) {
                        tool_id_to_name.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }
        // OpenAI Chat: tool_calls array
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
            for tc in tool_calls {
                if let (Some(id), Some(name)) = (
                    tc.get("id").and_then(|v| v.as_str()),
                    tc.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str()),
                ) {
                    tool_id_to_name.insert(id.to_string(), name.to_string());
                }
            }
        }
        // OpenAI Responses: type=function_call with call_id
        if msg.get("type").and_then(|t| t.as_str()) == Some("function_call") {
            if let (Some(id), Some(name)) = (
                msg.get("call_id").and_then(|v| v.as_str()),
                msg.get("name").and_then(|v| v.as_str()),
            ) {
                tool_id_to_name.insert(id.to_string(), name.to_string());
            }
        }
    }

    // Find the protection boundary: skip last N assistant messages
    let mut assistant_count = 0;
    let mut boundary = messages.len();
    for (i, msg) in messages.iter().enumerate().rev() {
        if is_assistant_message(msg) {
            assistant_count += 1;
            if assistant_count >= config.keep_last_assistants {
                boundary = i;
                break;
            }
        }
    }

    let placeholder = "[Ephemeral tool result cleared]";
    let mut cleared = 0;

    // Clear ephemeral tool results before the boundary
    for msg in &mut messages[..boundary] {
        if !is_tool_result(msg) {
            continue;
        }

        // Extract tool_use_id from the tool result message
        let tool_name = get_tool_name_for_result(msg, &tool_id_to_name);
        let is_ephemeral = match &tool_name {
            Some(name) => config.microcompact_tools.iter().any(|t| t == name),
            None => false,
        };

        if is_ephemeral {
            if let Some(text) = get_tool_result_text(msg) {
                if text.len() > placeholder.len() + 10 {
                    set_tool_result_text(msg, placeholder);
                    cleared += 1;
                }
            }
        }
    }

    cleared
}

/// Extract the tool name for a tool_result message using the tool_use_id→name map.
fn get_tool_name_for_result(
    msg: &Value,
    id_to_name: &HashMap<String, String>,
) -> Option<String> {
    // OpenAI Chat: role=tool with name field directly
    if let Some(name) = msg.get("name").and_then(|n| n.as_str()) {
        return Some(name.to_string());
    }

    // Try tool_call_id (OpenAI Chat) or call_id (OpenAI Responses)
    let tool_id = msg
        .get("tool_call_id")
        .or_else(|| msg.get("call_id"))
        .and_then(|v| v.as_str());
    if let Some(id) = tool_id {
        return id_to_name.get(id).cloned();
    }

    // Anthropic: content array with tool_result blocks containing tool_use_id
    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                if let Some(id) = block.get("tool_use_id").and_then(|v| v.as_str()) {
                    return id_to_name.get(id).cloned();
                }
            }
        }
    }

    None
}

// ── Tier 4: Emergency Compaction ──

/// Aggressively compact context when ContextOverflow occurs.
/// 1. Replace ALL tool result contents with placeholders
/// 2. Keep only the last N user turns
pub fn emergency_compact(messages: &mut Vec<Value>, config: &CompactConfig) -> CompactResult {
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

    // Adjust to a round-safe boundary so we never orphan tool_result messages
    keep_from = super::round_grouping::find_round_safe_boundary_forward(messages, keep_from);

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

    // Tier 0: Microcompact ephemeral tool results (zero cost, always runs first)
    let tier0_count = microcompact(messages, config);

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
        let prune = prune_old_context(
            messages,
            system_prompt,
            context_window,
            max_output_tokens,
            config,
        );
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
