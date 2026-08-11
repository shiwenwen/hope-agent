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
/// automatically falls back to the default side_query / direct HTTP path.
#[async_trait::async_trait]
pub trait CompactionProvider: Send + Sync {
    /// Summarize conversation content into a concise summary.
    async fn summarize(&self, prompt: &str, max_tokens: u32) -> anyhow::Result<String>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}
