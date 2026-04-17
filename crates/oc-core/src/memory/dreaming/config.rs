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
    "0 3 * * *".to_string()
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

    /// Optional dedicated `provider_id:model_id` for the narrative call.
    /// None = use the active chat agent (cache-friendly side_query).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative_model: Option<String>,
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
        }
    }
}
