//! Auto-review configuration (AppConfig.skills.auto_review).

use serde::{Deserialize, Serialize};

use crate::util::{default_true, SECS_PER_HOUR};

/// Promotion behavior when the review agent decides `create` or `patch`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoReviewPromotion {
    /// Write the skill with `status: draft` and surface it in the UI for
    /// manual promotion. This is the safe default.
    #[default]
    Draft,
    /// Write the skill directly as active — skips the review buffer. Use only
    /// when you trust the review model and the repo is isolated.
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsAutoReviewConfig {
    /// Master switch. Default: true (Phase B' ships enabled).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Where created/patched skills land.
    #[serde(default)]
    pub promotion: AutoReviewPromotion,
    /// Cooldown between reviews for the same session. Default 600s (10min).
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    /// Accumulated-token threshold since last review to fire. Default 10000.
    #[serde(default = "default_token_threshold")]
    pub token_threshold: usize,
    /// Accumulated-message threshold since last review to fire. Default 15.
    #[serde(default = "default_message_threshold")]
    pub message_threshold: usize,
    /// Optional "provider:model" override for the review side_query.
    /// When None, falls back to `recap::report::build_analysis_agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_model: Option<String>,
    /// Max recent messages passed into the review prompt. Default 24.
    #[serde(default = "default_candidate_limit")]
    pub candidate_limit: usize,
    /// Hard timeout on the side_query roundtrip. Default 90s.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Retention window for `learning_events` rows. Default 180 days.
    /// 0 = never prune.
    #[serde(default = "default_retention_days")]
    pub retention_days: u64,
}

impl Default for SkillsAutoReviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            promotion: AutoReviewPromotion::Draft,
            cooldown_secs: default_cooldown_secs(),
            token_threshold: default_token_threshold(),
            message_threshold: default_message_threshold(),
            review_model: None,
            candidate_limit: default_candidate_limit(),
            timeout_secs: default_timeout_secs(),
            retention_days: default_retention_days(),
        }
    }
}

fn default_cooldown_secs() -> u64 {
    600
}
fn default_token_threshold() -> usize {
    10_000
}
fn default_message_threshold() -> usize {
    15
}
fn default_candidate_limit() -> usize {
    24
}
fn default_timeout_secs() -> u64 {
    90
}
fn default_retention_days() -> u64 {
    180
}

impl SkillsAutoReviewConfig {
    /// Clamp any abusive values users might hand-edit. Called on load.
    pub fn sanitize(mut self) -> Self {
        self.cooldown_secs = self.cooldown_secs.max(60).min(24 * SECS_PER_HOUR);
        self.timeout_secs = self.timeout_secs.clamp(10, 10 * 60);
        self.candidate_limit = self.candidate_limit.clamp(4, 64);
        self.token_threshold = self.token_threshold.max(1_000);
        self.message_threshold = self.message_threshold.max(3);
        self
    }
}
