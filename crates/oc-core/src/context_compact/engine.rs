// ── Context Engine Trait ─────────────────────────────────────────────
//
//  Pluggable context compression engine.
//  Default implementation wraps the existing 5-tier system unchanged.

use serde_json::Value;

use super::config::CompactConfig;
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
}

/// Pluggable context compression engine.
///
/// Future implementations (Active Memory, custom compaction providers,
/// etc.) can replace individual methods or the entire engine.
pub trait ContextEngine: Send + Sync {
    /// Synchronous compaction: Tiers 0, 1, 2.
    ///
    /// If the returned `CompactResult.description` equals
    /// `"summarization_needed"`, the caller is responsible for
    /// executing Tier 3 (async LLM summarization) separately.
    fn compact_sync(
        &self,
        messages: &mut Vec<Value>,
        ctx: &CompactionContext<'_>,
    ) -> CompactResult;

    /// Emergency compaction (Tier 4): called on ContextOverflow.
    fn emergency_compact(
        &self,
        messages: &mut Vec<Value>,
        config: &CompactConfig,
    ) -> CompactResult;

    /// Optional system-prompt addition injected by the engine.
    /// A future Active Memory engine would return recall context here.
    fn system_prompt_addition(&self) -> Option<String> {
        None
    }
}

/// Default engine: delegates to the existing 5-tier free functions.
pub struct DefaultContextEngine;

impl ContextEngine for DefaultContextEngine {
    fn compact_sync(
        &self,
        messages: &mut Vec<Value>,
        ctx: &CompactionContext<'_>,
    ) -> CompactResult {
        // When throttled (cache-TTL active, non-emergency), set Tier 2+
        // thresholds to infinity so only Tier 0/1 run.
        if ctx.cache_ttl_throttled && !ctx.cache_ttl_emergency {
            let mut throttled = ctx.config.clone();
            throttled.soft_trim_ratio = f64::INFINITY;
            throttled.hard_clear_ratio = f64::INFINITY;
            throttled.summarization_threshold = f64::INFINITY;
            super::compact_if_needed(
                messages,
                ctx.system_prompt,
                ctx.context_window,
                ctx.max_output_tokens,
                &throttled,
            )
        } else {
            super::compact_if_needed(
                messages,
                ctx.system_prompt,
                ctx.context_window,
                ctx.max_output_tokens,
                ctx.config,
            )
        }
    }

    fn emergency_compact(
        &self,
        messages: &mut Vec<Value>,
        config: &CompactConfig,
    ) -> CompactResult {
        super::emergency_compact(messages, config)
    }
}
