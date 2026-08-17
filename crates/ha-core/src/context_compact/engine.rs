// ── Context Engine Trait ─────────────────────────────────────────────
//
//  Pluggable context compression engine.
//  Default implementation wraps the existing 5-tier system unchanged.

use serde_json::Value;

use super::config::CompactConfig;
use super::ledger::RuntimeLedgerSnapshot;
use super::types::CompactResult;

/// Read-only context for compaction decisions.
/// Bundles what the engine needs without exposing AssistantAgent internals.
pub struct CompactionContext<'a> {
    pub system_prompt: &'a str,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub config: &'a CompactConfig,
    /// Whether the cache-TTL throttle is active (Tier 2+ should be skipped).
    pub cache_ttl_throttled: bool,
    /// Whether the emergency override is triggered (usage ≥ 95%).
    pub cache_ttl_emergency: bool,
    /// Current provider/model tokenizer snapshot. Pure synchronous counting;
    /// compaction never performs Provider IO.
    pub token_counter: Option<&'a crate::token_accounting::CompactionTokenCounter<'a>>,
}

/// Read-only context for emergency compaction (Tier 4).
pub struct EmergencyCompactionContext<'a> {
    pub config: &'a CompactConfig,
    pub runtime_ledger: Option<&'a RuntimeLedgerSnapshot>,
}

/// Pluggable context compression engine.
///
/// Future implementations (Active Memory, custom compaction providers,
/// etc.) can replace individual methods or the entire engine.
pub trait ContextEngine: Send + Sync {
    /// Routine, pre-request compaction policy.
    ///
    /// The default preserves the legacy plug-in contract by delegating to
    /// `compact_sync`. The built-in engine overrides this entry point with the
    /// cache-aware policy: keep the existing prefix below the Tier-3 high
    /// watermark and request one semantic summary at/above it. Deterministic
    /// Tier 0/2 projection is reserved for exact capacity recovery.
    fn compact_routine(
        &self,
        messages: &mut Vec<Value>,
        ctx: &CompactionContext<'_>,
    ) -> CompactResult {
        self.compact_sync(messages, ctx)
    }

    /// Synchronous compaction: Tiers 0, 1, 2.
    ///
    /// If the returned `CompactResult.description` equals
    /// `"summarization_needed"`, the caller is responsible for
    /// executing Tier 3 (async LLM summarization) separately.
    fn compact_sync(&self, messages: &mut Vec<Value>, ctx: &CompactionContext<'_>)
        -> CompactResult;

    /// Emergency compaction (Tier 4): called on ContextOverflow.
    fn emergency_compact(
        &self,
        messages: &mut Vec<Value>,
        ctx: &EmergencyCompactionContext<'_>,
    ) -> CompactResult;

    /// Optional stable, trusted behavior contract supplied by the engine.
    /// Implementations must not return recall results, user/project content,
    /// mutable status, or any other turn-dependent data here; those belong in
    /// the agent's dynamic user-data lanes so they cannot gain system authority
    /// or invalidate the stable prompt prefix.
    fn stable_system_prompt_addition(&self) -> Option<String> {
        None
    }
}

/// Default engine: delegates to the existing 5-tier free functions.
pub struct DefaultContextEngine;

impl ContextEngine for DefaultContextEngine {
    fn compact_routine(
        &self,
        messages: &mut Vec<Value>,
        ctx: &CompactionContext<'_>,
    ) -> CompactResult {
        let tokens_before = ctx.token_counter.map_or_else(
            || super::estimate_request_tokens(ctx.system_prompt, messages, ctx.max_output_tokens),
            |counter| {
                counter.count_request_upper(ctx.system_prompt, messages, ctx.max_output_tokens)
            },
        );
        if !ctx.config.enabled || ctx.context_window == 0 || messages.is_empty() {
            return CompactResult {
                tier_applied: 0,
                tokens_before,
                tokens_after: tokens_before,
                messages_affected: 0,
                description: "no_op".to_string(),
                details: None,
                manifest: None,
            };
        }

        let plan = super::plan_routine_compaction(
            u64::from(tokens_before),
            ctx.context_window,
            ctx.config.summarization_threshold,
            false,
        );
        let summary_needed = matches!(plan.decision, super::CacheCompactionDecision::SummaryOnce);
        let details = summary_needed.then_some(super::CompactDetails {
            tool_results_truncated: 0,
            tool_results_soft_trimmed: 0,
            tool_results_hard_cleared: 0,
            messages_summarized: 0,
            summary_tokens: None,
        });
        let manifest = summary_needed.then(|| {
            let boundary = super::boundary_snapshot(messages, ctx.config.preserve_recent_rounds)
                .boundary(messages, super::BoundaryMode::SummarizeUnderPressure);
            super::CompactionManifest::for_result_with_boundary(
                3,
                "routine",
                tokens_before,
                tokens_before,
                details.as_ref(),
                &boundary,
            )
        });
        CompactResult {
            tier_applied: if summary_needed { 3 } else { 0 },
            tokens_before,
            tokens_after: tokens_before,
            messages_affected: 0,
            description: if summary_needed {
                "summarization_needed".to_string()
            } else {
                "cache_prefix_preserved".to_string()
            },
            details,
            manifest,
        }
    }

    fn compact_sync(
        &self,
        messages: &mut Vec<Value>,
        ctx: &CompactionContext<'_>,
    ) -> CompactResult {
        // When throttled (cache-TTL active, non-emergency), set Tier 2+
        // thresholds to infinity so only Tier 0/1 run.
        let mut result = if ctx.cache_ttl_throttled && !ctx.cache_ttl_emergency {
            let mut throttled = ctx.config.clone();
            throttled.soft_trim_ratio = f64::INFINITY;
            throttled.hard_clear_ratio = f64::INFINITY;
            throttled.summarization_threshold = f64::INFINITY;
            super::compact_if_needed_with_counter(
                messages,
                ctx.system_prompt,
                ctx.context_window,
                ctx.max_output_tokens,
                &throttled,
                ctx.token_counter,
            )
        } else {
            super::compact_if_needed_with_counter(
                messages,
                ctx.system_prompt,
                ctx.context_window,
                ctx.max_output_tokens,
                ctx.config,
                ctx.token_counter,
            )
        };
        if let Some(manifest) = result.manifest.take() {
            result.manifest = Some(
                manifest
                    .with_cache_ttl_throttled(ctx.cache_ttl_throttled && !ctx.cache_ttl_emergency),
            );
        }
        result
    }

    fn emergency_compact(
        &self,
        messages: &mut Vec<Value>,
        ctx: &EmergencyCompactionContext<'_>,
    ) -> CompactResult {
        super::emergency_compact(messages, ctx.config, ctx.runtime_ledger)
    }
}

// ── Compaction Provider (pluggable Tier 3 summarization) ─────────────

/// Pluggable summarization provider for Tier 3 compaction.
///
/// When configured, tried first for summarization; on failure the caller
/// falls back to an independent one-shot request using the conversation model.
#[async_trait::async_trait]
pub trait CompactionProvider: Send + Sync {
    /// Summarize conversation content into a concise summary.
    async fn summarize(&self, prompt: &str, max_tokens: u32) -> anyhow::Result<String>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;

    /// Context window of the summarization model when known. The caller uses
    /// the smallest applicable window to reject an oversized summary request
    /// before network I/O. Third-party providers may leave this unknown.
    fn context_window(&self) -> Option<u32> {
        None
    }
}
