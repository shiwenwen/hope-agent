//! Pure cache-aware policy for routine context compaction.
//!
//! Capacity safety remains outside this module. The policy only decides
//! whether a request that is still safe should keep its existing prompt
//! prefix or replace the summarizable prefix once at the configured high
//! watermark. It never reads configuration files, databases, prices, or
//! Provider state.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheCompactionDecision {
    KeepPrefix,
    SummaryOnce,
    CapacityRecovery,
    Emergency,
    CompatibilityProjection,
}

impl CacheCompactionDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeepPrefix => "keep_prefix",
            Self::SummaryOnce => "summary_once",
            Self::CapacityRecovery => "capacity_recovery",
            Self::Emergency => "emergency",
            Self::CompatibilityProjection => "compatibility_projection",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionSummaryReason {
    HighWatermark,
    RequiredAfterRecovery,
    Manual,
    EmergencyFollowup,
}

impl CompactionSummaryReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HighWatermark => "high_watermark",
            Self::RequiredAfterRecovery => "required_after_recovery",
            Self::Manual => "manual",
            Self::EmergencyFollowup => "emergency_followup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineCompactionPlan {
    pub decision: CacheCompactionDecision,
    pub input_upper: u64,
    pub high_watermark_tokens: u64,
}

/// Decide the routine, non-emergency action from one immutable capacity
/// snapshot. Crossing a lower compatibility threshold is intentionally not an
/// input: below the summary high watermark the old prefix remains byte-stable.
pub fn plan_routine_compaction(
    input_upper: u64,
    context_window: u32,
    summarization_threshold: f64,
    force_summary: bool,
) -> RoutineCompactionPlan {
    let threshold = if summarization_threshold.is_finite() {
        summarization_threshold.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let high_watermark_tokens =
        ((f64::from(context_window) * threshold).ceil() as u64).min(u64::from(context_window));
    let summary_needed =
        force_summary || (context_window > 0 && input_upper >= high_watermark_tokens.max(1));
    RoutineCompactionPlan {
        decision: if summary_needed {
            CacheCompactionDecision::SummaryOnce
        } else {
            CacheCompactionDecision::KeepPrefix
        },
        input_upper,
        high_watermark_tokens,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionEconomics {
    pub reclaimed_tokens_upper: u64,
    pub invalidated_suffix_tokens_upper: u64,
    pub break_even_turns: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelativeCacheCosts {
    pub read_units: u64,
    pub rewrite_units: u64,
    pub compaction_call_units: u64,
}

/// Shadow-only economics. A missing Provider-specific cost snapshot keeps the
/// break-even value unknown and must never authorize a proactive rewrite.
pub fn projection_economics(
    canonical_input_upper: u64,
    projected_input_upper: u64,
    invalidated_suffix_tokens_upper: u64,
    costs: Option<RelativeCacheCosts>,
) -> ProjectionEconomics {
    let reclaimed_tokens_upper = canonical_input_upper.saturating_sub(projected_input_upper);
    let break_even_turns = costs.and_then(|costs| {
        if reclaimed_tokens_upper == 0 || costs.read_units == 0 {
            return None;
        }
        let rewrite_delta = costs.rewrite_units.saturating_sub(costs.read_units);
        let penalty = invalidated_suffix_tokens_upper
            .saturating_mul(rewrite_delta)
            .saturating_add(costs.compaction_call_units);
        let saving = reclaimed_tokens_upper.saturating_mul(costs.read_units);
        Some(penalty.div_ceil(saving.max(1)))
    });
    ProjectionEconomics {
        reclaimed_tokens_upper,
        invalidated_suffix_tokens_upper,
        break_even_turns,
    }
}

/// First provider-history item whose exact JSON value differs. Appended tail
/// items do not count as a prefix rewrite; only differences inside the shared
/// prefix create an invalidated suffix.
pub fn first_rewritten_item(canonical: &[Value], projected: &[Value]) -> Option<usize> {
    let shared = canonical.len().min(projected.len());
    canonical
        .iter()
        .zip(projected)
        .take(shared)
        .position(|(left, right)| left != right)
        .or_else(|| (projected.len() < canonical.len()).then_some(shared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routine_policy_keeps_prefix_below_high_watermark() {
        let plan = plan_routine_compaction(8_499, 10_000, 0.85, false);
        assert_eq!(plan.decision, CacheCompactionDecision::KeepPrefix);
        assert_eq!(plan.high_watermark_tokens, 8_500);
        assert_eq!(
            plan_routine_compaction(8_500, 10_000, 0.85, false).decision,
            CacheCompactionDecision::SummaryOnce
        );
    }

    #[test]
    fn shadow_economics_never_invents_unknown_prices() {
        let unknown = projection_economics(100_000, 90_000, 60_000, None);
        assert_eq!(unknown.reclaimed_tokens_upper, 10_000);
        assert_eq!(unknown.break_even_turns, None);
        let known = projection_economics(
            100_000,
            90_000,
            60_000,
            Some(RelativeCacheCosts {
                read_units: 100,
                rewrite_units: 1_250,
                compaction_call_units: 0,
            }),
        );
        assert_eq!(known.break_even_turns, Some(69));
    }

    #[test]
    fn appended_tail_is_not_a_prefix_rewrite() {
        let canonical = vec![json!({"role":"user","content":"a"})];
        let mut appended = canonical.clone();
        appended.push(json!({"role":"assistant","content":"b"}));
        assert_eq!(first_rewritten_item(&canonical, &appended), None);
        let rewritten = vec![json!({"role":"user","content":"changed"})];
        assert_eq!(first_rewritten_item(&canonical, &rewritten), Some(0));
    }
}
