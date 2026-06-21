// ── Main Entry Point + Tier 0/4 Compaction ──

use super::config::CompactConfig;
use super::estimation::{
    build_tool_id_to_name_map, estimate_request_tokens, estimate_tokens, get_tool_name_for_result,
    get_tool_result_text, is_tool_result, set_tool_result_text,
};
use super::pruning::prune_old_context_with_boundary;
use super::round_grouping::{is_recovered_round, ROUND_KEY};
use super::truncation::truncate_tool_results;
use super::types::{CompactDetails, CompactResult};
use super::{
    boundary_snapshot, build_runtime_ledger_message, BoundaryMode, CompactionManifest,
    RecentBoundary, RuntimeLedgerSnapshot,
};
use serde_json::Value;

// ── Tier 0: Microcompaction ──

fn attach_manifest_with_boundary(
    mut result: CompactResult,
    boundary: &RecentBoundary,
    trigger: &str,
) -> CompactResult {
    result.manifest = Some(CompactionManifest::for_result_with_boundary(
        result.tier_applied,
        trigger,
        result.tokens_before,
        result.tokens_after,
        result.details.as_ref(),
        boundary,
    ));
    result
}

/// Zero-cost clearing of ephemeral tool results (ls, grep, find, etc.)
/// that are older than the protected boundary. No LLM call required.
///
/// Returns the number of tool results cleared.
pub fn microcompact(messages: &mut [Value], config: &CompactConfig) -> usize {
    let boundary = super::recent_boundary(messages, config.preserve_recent_rounds);
    microcompact_with_boundary(messages, config, boundary.protected_start_index)
}

fn microcompact_with_boundary(
    messages: &mut [Value],
    config: &CompactConfig,
    protected_start_index: usize,
) -> usize {
    if config.eager_tools().is_empty() {
        return 0;
    }

    let tool_id_to_name = build_tool_id_to_name_map(messages);

    let placeholder = "[Ephemeral tool result cleared]";
    let mut cleared = 0;

    // Clear ephemeral tool results before the boundary
    for msg in &mut messages[..protected_start_index] {
        if !is_tool_result(msg) {
            continue;
        }

        // Extract tool_use_id from the tool result message
        let tool_name = get_tool_name_for_result(msg, &tool_id_to_name);
        let is_ephemeral = match &tool_name {
            Some(name) => config.is_eager(name),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RecoveredToolCleanup {
    pub(crate) hard_cleared: usize,
    pub(crate) image_markers_materialized: usize,
}

impl RecoveredToolCleanup {
    pub(crate) fn messages_affected(self) -> usize {
        self.hard_cleared
            .saturating_add(self.image_markers_materialized)
    }

    pub(crate) fn changed(self) -> bool {
        self.messages_affected() > 0
    }
}

pub(crate) fn compact_oversized_recovered_tool_results(
    messages: &mut [Value],
    min_content_chars: usize,
    session_id: Option<&str>,
    materialize_image_markers: bool,
) -> RecoveredToolCleanup {
    let threshold = min_content_chars.max(8 * 1024);
    let mut cleanup = RecoveredToolCleanup::default();

    for msg in messages {
        let recovered = msg
            .get(ROUND_KEY)
            .and_then(|v| v.as_str())
            .is_some_and(is_recovered_round);
        if !recovered || !is_tool_result(msg) {
            continue;
        }

        let Some(text) = get_tool_result_text(msg) else {
            continue;
        };
        if text.len() <= threshold {
            continue;
        }

        if crate::tools::image_markers::contains_image_marker(&text) {
            if crate::tools::image_markers::has_valid_image_markers(&text) {
                if materialize_image_markers {
                    if let Ok(Some(compacted)) =
                        crate::tools::image_markers::materialize_base64_image_markers(
                            &text, session_id,
                        )
                    {
                        set_tool_result_text(msg, &compacted);
                        cleanup.image_markers_materialized =
                            cleanup.image_markers_materialized.saturating_add(1);
                    }
                }
                continue;
            }

            let placeholder = format!(
                "[Invalid or truncated recovered image tool result cleared during manual context compaction: original content was {} chars. Re-run the tool if exact output is needed.]",
                text.len()
            );
            set_tool_result_text(msg, &placeholder);
            cleanup.hard_cleared = cleanup.hard_cleared.saturating_add(1);
            continue;
        }

        let placeholder = format!(
            "[Recovered tool result cleared during manual context compaction: original content was {} chars. Re-run the tool if exact output is needed.]",
            text.len()
        );
        set_tool_result_text(msg, &placeholder);
        cleanup.hard_cleared = cleanup.hard_cleared.saturating_add(1);
    }

    cleanup
}

// ── Tier 4: Emergency Compaction ──

/// Aggressively compact context when ContextOverflow occurs.
/// 1. Replace ALL tool result contents with placeholders
/// 2. Keep only the last N user turns
pub fn emergency_compact(
    messages: &mut Vec<Value>,
    config: &CompactConfig,
    runtime_ledger: Option<&RuntimeLedgerSnapshot>,
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

    // Tier 4 is the last-resort safety net: unlike Tier 0/2/3, fail-closing to
    // "protect everything" (boundary == 0) is unacceptable here — the request
    // MUST shrink or the very next API call overflows again. When the unified
    // boundary protects the whole history (too few live rounds to leave a
    // prunable prefix), fall back to keeping only the most recent round and
    // dropping all older history.
    let snapshot = boundary_snapshot(messages, config.preserve_recent_rounds);
    let boundary = snapshot.boundary(messages, BoundaryMode::Emergency);
    let keep_from = boundary.protected_start_index;

    if keep_from > 0 && keep_from < messages.len() {
        let removed = keep_from;
        messages.drain(..keep_from);
        affected += removed;
    }

    if let Some(snapshot) = runtime_ledger {
        if let Some(ledger_msg) = build_runtime_ledger_message(snapshot, &[], 4_000) {
            messages.insert(0, ledger_msg);
        }
    }

    let tokens_after = messages.iter().map(|m| estimate_tokens(m)).sum::<u32>();

    attach_manifest_with_boundary(
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
            manifest: None,
        },
        &boundary,
        "emergency",
    )
}

// ── Main Entry Point ──

/// Apply compaction tiers as needed based on context usage.
/// This is the main entry point called before each API request.
/// Tiers 1 & 2 are synchronous. Tier 3 (LLM summarization) requires
/// async and is handled separately in agent.rs.
pub fn compact_if_needed(
    messages: &mut [Value],
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
            manifest: None,
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
            manifest: None,
        };
    }

    let boundary = boundary_snapshot(messages, config.preserve_recent_rounds)
        .boundary(messages, BoundaryMode::ProtectRecent);

    // Tier 0: Microcompact ephemeral tool results (zero cost, always runs first)
    let _tier0_count = microcompact_with_boundary(messages, config, boundary.protected_start_index);

    // Tier 1: Truncate individual oversized tool results
    let tier1_count = truncate_tool_results(messages, context_window, config);

    let tokens_after_t1 = estimate_request_tokens(system_prompt, messages, max_output_tokens);
    let ratio_after_t1 = tokens_after_t1 as f64 / context_window as f64;

    if tier1_count > 0 && ratio_after_t1 < config.soft_trim_ratio {
        return attach_manifest_with_boundary(
            CompactResult {
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
                manifest: None,
            },
            &boundary,
            "sync",
        );
    }

    // Tier 2: Context pruning (soft trim + hard clear)
    if ratio_after_t1 >= config.soft_trim_ratio {
        let prune = prune_old_context_with_boundary(
            messages,
            system_prompt,
            context_window,
            max_output_tokens,
            config,
            &boundary,
        );
        let tokens_after_t2 = estimate_request_tokens(system_prompt, messages, max_output_tokens);
        let ratio_after_t2 = tokens_after_t2 as f64 / context_window as f64;

        if prune.soft_trimmed > 0 || prune.hard_cleared > 0 {
            if ratio_after_t2 < config.summarization_threshold {
                return attach_manifest_with_boundary(
                    CompactResult {
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
                        manifest: None,
                    },
                    &boundary,
                    "sync",
                );
            }
        }

        // Tier 3 needed but requires async — return a signal
        if ratio_after_t2 >= config.summarization_threshold {
            return attach_manifest_with_boundary(
                CompactResult {
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
                    manifest: None,
                },
                &boundary,
                "sync",
            );
        }
    }

    // Return Tier 1 result if only truncation was done
    if tier1_count > 0 {
        return attach_manifest_with_boundary(
            CompactResult {
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
                manifest: None,
            },
            &boundary,
            "sync",
        );
    }

    CompactResult {
        tier_applied: 0,
        tokens_before,
        tokens_after: tokens_before,
        messages_affected: 0,
        description: "no_action_needed".to_string(),
        details: None,
        manifest: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use serde_json::json;

    #[test]
    fn microcompact_resolves_responses_tool_name_from_call_id() {
        let mut messages = vec![
            json!({ "role": "user", "content": "search the workspace" }),
            json!({
                "type": "function_call",
                "call_id": "fc_1",
                "name": "tool_search",
                "arguments": "{\"query\":\"context compact\"}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "fc_1",
                "output": "stale searchable result ".repeat(20)
            }),
            json!({ "role": "user", "content": "latest request" }),
            json!({ "role": "assistant", "content": "latest reply" }),
        ];

        let config = CompactConfig {
            preserve_recent_rounds: 1,
            ..CompactConfig::default()
        };

        let cleared = microcompact(&mut messages, &config);

        assert_eq!(cleared, 1);
        assert_eq!(
            messages[2].get("output").and_then(|v| v.as_str()),
            Some("[Ephemeral tool result cleared]")
        );
    }

    #[test]
    fn emergency_compact_makes_progress_when_boundary_fail_closes() {
        // Few large non-tool rounds → recent_boundary fail-closes (live rounds
        // <= preserve_recent_rounds) and returns protected_start_index = 0.
        // Tier 4 must still drop older history rather than leave everything.
        let big = "x".repeat(5_000);
        let mut messages = vec![
            json!({ "role": "user", "content": big.clone() }),
            json!({ "role": "assistant", "content": big.clone() }),
            json!({ "role": "user", "content": "latest" }),
            json!({ "role": "assistant", "content": "reply" }),
        ];
        let before = messages.len();

        let result = emergency_compact(&mut messages, &CompactConfig::default(), None);

        assert_eq!(result.tier_applied, 4);
        assert!(
            messages.len() < before,
            "emergency_compact must shrink history even when the boundary fail-closes"
        );
    }

    #[test]
    fn manual_recovered_cleanup_clears_large_recovered_tool_result() {
        let mut messages = vec![
            json!({
                "role": "tool",
                "_oc_round": "recovered-123",
                "content": "x".repeat(12_000),
            }),
            json!({
                "role": "tool",
                "_oc_round": "r0",
                "content": "x".repeat(12_000),
            }),
            json!({
                "role": "tool",
                "_oc_round": "recovered-456",
                "content": "small",
            }),
        ];

        let cleanup =
            compact_oversized_recovered_tool_results(&mut messages, 8_000, Some("s1"), false);

        assert_eq!(cleanup.hard_cleared, 1);
        assert_eq!(cleanup.image_markers_materialized, 0);
        assert_eq!(cleanup.messages_affected(), 1);
        let cleared_text = messages[0].get("content").and_then(|v| v.as_str()).unwrap();
        assert!(cleared_text.contains("Recovered tool result cleared"));
        assert_eq!(
            messages[1]
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap()
                .len(),
            12_000
        );
        assert_eq!(
            messages[2].get("content").and_then(|v| v.as_str()),
            Some("small")
        );
    }

    #[test]
    fn manual_recovered_cleanup_preserves_valid_image_markers_when_not_materializing() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(vec![0_u8; 9_000]);
        let image_marker = crate::tools::image_markers::build_image_base64_marker(
            "image/png",
            &b64,
            "Screenshot captured.",
        );
        let mut messages = vec![json!({
            "role": "tool",
            "_oc_round": "recovered-image",
            "content": image_marker,
        })];
        let original = messages[0]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();

        let cleanup =
            compact_oversized_recovered_tool_results(&mut messages, 8_000, Some("s1"), false);

        assert_eq!(cleanup, RecoveredToolCleanup::default());
        assert_eq!(
            messages[0].get("content").and_then(|v| v.as_str()),
            Some(original.as_str())
        );
    }

    #[test]
    fn manual_recovered_cleanup_clears_truncated_image_markers() {
        let truncated_marker = format!(
            "{}image/png__{}",
            crate::tools::browser::IMAGE_BASE64_PREFIX,
            "a".repeat(12_000)
        );
        let mut messages = vec![json!({
            "role": "tool",
            "_oc_round": "recovered-image",
            "content": truncated_marker,
        })];

        let cleanup =
            compact_oversized_recovered_tool_results(&mut messages, 8_000, Some("s1"), false);

        assert_eq!(cleanup.hard_cleared, 1);
        let cleared_text = messages[0].get("content").and_then(|v| v.as_str()).unwrap();
        assert!(cleared_text.contains("Invalid or truncated recovered image tool result cleared"));
    }
}
