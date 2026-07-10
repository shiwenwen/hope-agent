//! Dreaming configuration — persisted under `AppConfig.dreaming`.

use serde::{Deserialize, Serialize};

fn default_scope_days() -> u32 {
    1
}
fn default_candidate_limit() -> usize {
    50
}
fn default_idle_minutes() -> u32 {
    30
}
fn default_cron_expr() -> String {
    // 6-field cron format consumed by the `cron` crate (sec min hour
    // day month weekday). 5-field POSIX expressions are rejected.
    "0 0 3 * * *".to_string()
}
fn default_promotion_min_score() -> f32 {
    0.75
}
fn default_promotion_max_promote() -> usize {
    5
}
fn default_narrative_max_tokens() -> u32 {
    2048
}
fn default_narrative_timeout_secs() -> u64 {
    60
}
fn default_true() -> bool {
    true
}
fn default_profile_max_lines() -> usize {
    12
}
fn default_auto_resolve_max_groups() -> usize {
    8
}
fn default_auto_resolve_min_confidence() -> f32 {
    0.92
}
fn default_auto_merge_similarity() -> f32 {
    0.84
}

/// Idle trigger: run when the app has been idle (no user turn) for this
/// many minutes. Consumed by `Guardian`'s heartbeat via
/// [`super::check_idle_trigger`]. `enabled=false` disables the path entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleTriggerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u32,
}

impl Default for IdleTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_minutes: default_idle_minutes(),
        }
    }
}

/// Cron trigger: run on a crontab-style schedule. Off by default so the
/// idle trigger doesn't get duplicated. Users who want deterministic
/// nightly cycles flip this on and (optionally) disable `idle.enabled`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronTriggerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cron_expr")]
    pub cron_expr: String,
}

impl Default for CronTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cron_expr: default_cron_expr(),
        }
    }
}

/// Promotion thresholds. Kept as a single struct so the LLM selector and
/// the post-filter use consistent cutoffs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionThresholds {
    /// Minimum per-candidate score (0.0–1.0) required to pin. Default 0.75.
    #[serde(default = "default_promotion_min_score")]
    pub min_score: f32,
    /// Hard cap on how many candidates are promoted per cycle. Default 5.
    #[serde(default = "default_promotion_max_promote")]
    pub max_promote: usize,
}

impl Default for PromotionThresholds {
    fn default() -> Self {
        Self {
            min_score: default_promotion_min_score(),
            max_promote: default_promotion_max_promote(),
        }
    }
}

/// Profile Synthesis (next-gen Dreaming Phase 4): synthesise a displayable +
/// injectable Memory Profile from active claims, layered by scope. **On by
/// default** — when disabled, no snapshot is produced and the system
/// prompt keeps rendering the legacy profile-tagged `## User Profile` section.
/// Idle / cron run a cheap rule-based aggregation (no side_query); manual runs
/// an LLM rewrite for fluency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSynthesisConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Max profile bullet lines kept per scope (rule-based aggregation cap;
    /// the LLM rewrite is asked to stay within roughly the same budget).
    #[serde(default = "default_profile_max_lines")]
    pub max_lines_per_scope: usize,
}

impl Default for ProfileSynthesisConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_lines_per_scope: default_profile_max_lines(),
        }
    }
}

/// Deep Resolver automation. Ordinary Light cycles always keep deterministic
/// expiry; a bounded graph-informed LLM pass may additionally route
/// high-confidence conflicts to review and merge only strongly corroborated
/// near-duplicates. It never auto-supersedes one conflicting fact with another.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepResolverConfig {
    #[serde(default = "default_true")]
    pub auto_expire_on_light_cycle: bool,
    #[serde(default = "default_true")]
    pub auto_resolve_on_light_cycle: bool,
    #[serde(default = "default_auto_resolve_max_groups")]
    pub auto_resolve_max_groups: usize,
    #[serde(default = "default_auto_resolve_min_confidence")]
    pub auto_resolve_min_confidence: f32,
    #[serde(default = "default_true")]
    pub auto_merge_near_duplicates: bool,
    #[serde(default = "default_auto_merge_similarity")]
    pub auto_merge_similarity: f32,
}

impl Default for DeepResolverConfig {
    fn default() -> Self {
        Self {
            auto_expire_on_light_cycle: true,
            auto_resolve_on_light_cycle: true,
            auto_resolve_max_groups: default_auto_resolve_max_groups(),
            auto_resolve_min_confidence: default_auto_resolve_min_confidence(),
            auto_merge_near_duplicates: true,
            auto_merge_similarity: default_auto_merge_similarity(),
        }
    }
}

impl DeepResolverConfig {
    pub fn auto_group_cap(&self) -> usize {
        self.auto_resolve_max_groups.clamp(1, 20)
    }

    pub fn auto_min_confidence(&self) -> f32 {
        if self.auto_resolve_min_confidence.is_finite() {
            self.auto_resolve_min_confidence.clamp(0.75, 0.99)
        } else {
            default_auto_resolve_min_confidence()
        }
    }

    pub fn auto_merge_similarity_threshold(&self) -> f32 {
        if self.auto_merge_similarity.is_finite() {
            self.auto_merge_similarity.clamp(0.7, 0.98)
        } else {
            default_auto_merge_similarity()
        }
    }
}

/// Top-level Dreaming configuration. Persisted under `AppConfig.dreaming`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingConfig {
    /// Master switch. When `false`, every trigger is a no-op.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Idle trigger configuration.
    #[serde(default)]
    pub idle_trigger: IdleTriggerConfig,

    /// Cron trigger configuration.
    #[serde(default)]
    pub cron_trigger: CronTriggerConfig,

    /// Whether the Dashboard "Run now" button is active. Separate from the
    /// master switch so a user can expose manual-only usage.
    #[serde(default = "default_true")]
    pub manual_enabled: bool,

    /// Promotion cutoffs.
    #[serde(default)]
    pub promotion: PromotionThresholds,

    /// Scan window in days. Default 1 day (Light phase).
    #[serde(default = "default_scope_days")]
    pub scope_days: u32,

    /// Maximum candidates fetched from the memory backend per cycle.
    /// Keeps prompt size bounded even on very active agents.
    #[serde(default = "default_candidate_limit")]
    pub candidate_limit: usize,

    /// Max tokens budget for the narrative side_query call.
    #[serde(default = "default_narrative_max_tokens")]
    pub narrative_max_tokens: u32,

    /// Narrative side_query timeout in seconds.
    #[serde(default = "default_narrative_timeout_secs")]
    pub narrative_timeout_secs: u64,

    /// Deprecated — superseded by `model_override`. Dedicated
    /// `provider_id:model_id` string for the narrative call. Kept for
    /// backward compatibility: still parsed when `model_override` is unset,
    /// but the GUI no longer writes this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative_model: Option<String>,
    /// Model chain override for the narrative call. `None` = fall through to
    /// the deprecated `narrative_model` (if still set) → `function_models.automation`
    /// → chat default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,

    /// Profile Synthesis (Phase 4). On by default.
    #[serde(default)]
    pub profile_synthesis: ProfileSynthesisConfig,

    /// Deep Resolver automation. Defaults to conservative graph + LLM review.
    #[serde(default)]
    pub deep_resolver: DeepResolverConfig,
}

impl Default for DreamingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_trigger: IdleTriggerConfig::default(),
            cron_trigger: CronTriggerConfig::default(),
            manual_enabled: true,
            promotion: PromotionThresholds::default(),
            scope_days: default_scope_days(),
            candidate_limit: default_candidate_limit(),
            narrative_max_tokens: default_narrative_max_tokens(),
            narrative_timeout_secs: default_narrative_timeout_secs(),
            narrative_model: None,
            model_override: None,
            profile_synthesis: ProfileSynthesisConfig::default(),
            deep_resolver: DeepResolverConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_resolver_defaults_to_bounded_conservative_automation() {
        let cfg = DreamingConfig::default();

        assert!(cfg.deep_resolver.auto_expire_on_light_cycle);
        assert!(cfg.deep_resolver.auto_resolve_on_light_cycle);
        assert_eq!(cfg.deep_resolver.auto_group_cap(), 8);
        assert_eq!(cfg.deep_resolver.auto_min_confidence(), 0.92);
        assert!(cfg.deep_resolver.auto_merge_near_duplicates);
        assert_eq!(cfg.deep_resolver.auto_merge_similarity_threshold(), 0.84);
    }

    #[test]
    fn dreaming_config_deserializes_legacy_without_deep_resolver() {
        let cfg: DreamingConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "manualEnabled": true
        }))
        .unwrap();

        assert!(cfg.deep_resolver.auto_expire_on_light_cycle);
        assert!(cfg.deep_resolver.auto_resolve_on_light_cycle);
    }
}
