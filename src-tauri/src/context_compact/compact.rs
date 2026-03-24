// ── Main Entry Point + Tier 4: Emergency Compaction ──

use serde_json::Value;
use super::config::CompactConfig;
use super::estimation::{estimate_tokens, estimate_request_tokens, is_tool_result, is_user_message, get_tool_result_text, set_tool_result_text};
use super::truncation::truncate_tool_results;
use super::pruning::prune_old_context;
use super::types::{CompactResult, CompactDetails};

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
