//! Auto-review configuration (`AppConfig.skills.auto_review`).
//!
//! Five-gate waterfall pipeline; gate enforcement lives in `triggers.rs`
//! (gate 1), `heuristics.rs` (gates 2 & 5), and `pipeline.rs` (gates 3 & 4).
//! Defaults skew strict — surface false-negatives in the UI rather than ship
//! false-positive drafts to the user.

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
    // ── Master ──────────────────────────────────────────────────────────
    /// Master switch. Default `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Where created/patched skills land.
    #[serde(default)]
    pub promotion: AutoReviewPromotion,

    // ── Gate 1 (trigger) ────────────────────────────────────────────────
    /// Cooldown between auto-fires for the same session.
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    /// Accumulated-token threshold since last review to consider firing.
    #[serde(default = "default_token_threshold")]
    pub token_threshold: usize,
    /// Accumulated-message threshold since last review to consider firing.
    #[serde(default = "default_message_threshold")]
    pub message_threshold: usize,
    /// Per-session accumulated tool-use count threshold. 0 disables this
    /// signal. Default 3 — the dominant hard gate that keeps pure-chat
    /// conversations from triggering review at all.
    #[serde(default = "default_tool_use_threshold")]
    pub tool_use_threshold: usize,
    /// When true, the "two user messages within 30s" correction proxy can
    /// fire the trigger on its own.
    #[serde(default = "default_true")]
    pub correction_signal_enabled: bool,
    /// When true, gate 1 refuses to fire on turns with zero tool use, even
    /// if other thresholds are met. The correction signal still wins.
    #[serde(default = "default_true")]
    pub require_tool_use: bool,

    // ── Gate 2 (pre-LLM heuristics) ─────────────────────────────────────
    /// Minimum total messages in the recent transcript window before any
    /// LLM call is made.
    #[serde(default = "default_min_message_count")]
    pub min_message_count: usize,
    /// Days of `skill_discarded` history to treat as a topical blacklist.
    /// 0 disables.
    #[serde(default = "default_discard_blacklist_days")]
    pub discard_blacklist_days: u64,

    // ── Gate 3 (LLM review) ─────────────────────────────────────────────
    /// Top-K existing skills (by lightweight Jaccard) whose full bodies get
    /// fed into the dedup prompt.
    #[serde(default = "default_top_k_for_dedup")]
    pub top_k_for_dedup: usize,
    /// Optional `provider:model` override for the review side_query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_model: Option<String>,
    /// Max recent messages passed into the review prompt.
    #[serde(default = "default_candidate_limit")]
    pub candidate_limit: usize,
    /// Hard timeout on the side_query roundtrip.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Optional full-text override of the built-in REVIEW_SYSTEM prompt.
    /// `None` (default) uses the built-in. Power-users only; gates 2/4/5
    /// still apply unconditionally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_system_override: Option<String>,
    /// Extra free-form reject categories appended verbatim to the built-in
    /// 6-category list inside the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_reject_categories: Vec<String>,

    // ── Gate 4 (self-score hard floor) ──────────────────────────────────
    /// Minimum model-reported `reuse_probability` to admit a create.
    #[serde(default = "default_min_reuse_probability")]
    pub min_reuse_probability: f32,

    // ── Gate 5 (post-LLM lint) ──────────────────────────────────────────
    /// Number of session-recap markers (`今天` / `本次` / `this conversation`
    /// / etc.) that flips a body into the recap reject bucket.
    #[serde(default = "default_session_recap_threshold")]
    pub session_recap_threshold: usize,
    /// Minimum step count in the body's procedural section.
    #[serde(default = "default_min_steps")]
    pub min_steps: usize,
    /// Maximum step count (overly long bodies are usually session
    /// transcripts dressed up as a procedure).
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,

    // ── Curator (draft consolidation) ───────────────────────────────────
    /// Schedule the periodic consolidation pass on its own timer.
    #[serde(default)]
    pub auto_curator_enabled: bool,
    /// Days between consolidation passes when `auto_curator_enabled`.
    #[serde(default = "default_auto_curator_interval_days")]
    pub auto_curator_interval_days: u64,

    // ── Retention ───────────────────────────────────────────────────────
    /// Retention window for `learning_events` rows. 0 = never prune.
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
            tool_use_threshold: default_tool_use_threshold(),
            correction_signal_enabled: true,
            require_tool_use: true,
            min_message_count: default_min_message_count(),
            discard_blacklist_days: default_discard_blacklist_days(),
            top_k_for_dedup: default_top_k_for_dedup(),
            review_model: None,
            candidate_limit: default_candidate_limit(),
            timeout_secs: default_timeout_secs(),
            review_system_override: None,
            extra_reject_categories: Vec::new(),
            min_reuse_probability: default_min_reuse_probability(),
            session_recap_threshold: default_session_recap_threshold(),
            min_steps: default_min_steps(),
            max_steps: default_max_steps(),
            auto_curator_enabled: false,
            auto_curator_interval_days: default_auto_curator_interval_days(),
            retention_days: default_retention_days(),
        }
    }
}

fn default_cooldown_secs() -> u64 {
    900
}
fn default_token_threshold() -> usize {
    12_000
}
fn default_message_threshold() -> usize {
    20
}
fn default_tool_use_threshold() -> usize {
    3
}
fn default_min_message_count() -> usize {
    4
}
fn default_discard_blacklist_days() -> u64 {
    30
}
fn default_top_k_for_dedup() -> usize {
    5
}
fn default_candidate_limit() -> usize {
    24
}
fn default_timeout_secs() -> u64 {
    90
}
fn default_min_reuse_probability() -> f32 {
    0.7
}
fn default_session_recap_threshold() -> usize {
    2
}
fn default_min_steps() -> usize {
    2
}
fn default_max_steps() -> usize {
    12
}
fn default_auto_curator_interval_days() -> u64 {
    7
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
        self.min_message_count = self.min_message_count.clamp(0, 100);
        self.top_k_for_dedup = self.top_k_for_dedup.clamp(1, 20);
        self.min_reuse_probability = self.min_reuse_probability.clamp(0.0, 1.0);
        self.session_recap_threshold = self.session_recap_threshold.clamp(0, 10);
        if self.max_steps < self.min_steps {
            self.max_steps = self.min_steps;
        }
        self.min_steps = self.min_steps.clamp(0, 50);
        self.max_steps = self.max_steps.clamp(1, 50);
        self.discard_blacklist_days = self.discard_blacklist_days.min(365);
        self.auto_curator_interval_days = self.auto_curator_interval_days.clamp(1, 90);
        self
    }

    /// Per-field reset helper. `None` means "reset every field"; otherwise
    /// only the listed snake_case keys are restored. Unknown keys are
    /// silently ignored (the API layer can validate up front if it cares).
    pub fn reset_fields(&mut self, fields: Option<&[String]>) {
        let d = Self::default();
        let want = |key: &str| match fields {
            None => true,
            Some(list) => list.iter().any(|k| k == key),
        };
        if want("enabled") {
            self.enabled = d.enabled;
        }
        if want("promotion") {
            self.promotion = d.promotion;
        }
        if want("cooldown_secs") {
            self.cooldown_secs = d.cooldown_secs;
        }
        if want("token_threshold") {
            self.token_threshold = d.token_threshold;
        }
        if want("message_threshold") {
            self.message_threshold = d.message_threshold;
        }
        if want("tool_use_threshold") {
            self.tool_use_threshold = d.tool_use_threshold;
        }
        if want("correction_signal_enabled") {
            self.correction_signal_enabled = d.correction_signal_enabled;
        }
        if want("require_tool_use") {
            self.require_tool_use = d.require_tool_use;
        }
        if want("min_message_count") {
            self.min_message_count = d.min_message_count;
        }
        if want("discard_blacklist_days") {
            self.discard_blacklist_days = d.discard_blacklist_days;
        }
        if want("top_k_for_dedup") {
            self.top_k_for_dedup = d.top_k_for_dedup;
        }
        if want("review_model") {
            self.review_model = d.review_model;
        }
        if want("candidate_limit") {
            self.candidate_limit = d.candidate_limit;
        }
        if want("timeout_secs") {
            self.timeout_secs = d.timeout_secs;
        }
        if want("review_system_override") {
            self.review_system_override = d.review_system_override;
        }
        if want("extra_reject_categories") {
            self.extra_reject_categories = d.extra_reject_categories;
        }
        if want("min_reuse_probability") {
            self.min_reuse_probability = d.min_reuse_probability;
        }
        if want("session_recap_threshold") {
            self.session_recap_threshold = d.session_recap_threshold;
        }
        if want("min_steps") {
            self.min_steps = d.min_steps;
        }
        if want("max_steps") {
            self.max_steps = d.max_steps;
        }
        if want("auto_curator_enabled") {
            self.auto_curator_enabled = d.auto_curator_enabled;
        }
        if want("auto_curator_interval_days") {
            self.auto_curator_interval_days = d.auto_curator_interval_days;
        }
        if want("retention_days") {
            self.retention_days = d.retention_days;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_clamps_max_smaller_than_min_steps() {
        let c = SkillsAutoReviewConfig {
            min_steps: 8,
            max_steps: 3,
            ..Default::default()
        };
        let c = c.sanitize();
        assert_eq!(c.min_steps, 8);
        assert_eq!(c.max_steps, 8);
    }

    #[test]
    fn reset_fields_single() {
        let mut c = SkillsAutoReviewConfig {
            min_reuse_probability: 0.1,
            cooldown_secs: 60,
            ..Default::default()
        };
        c.reset_fields(Some(&["min_reuse_probability".to_string()]));
        assert!((c.min_reuse_probability - 0.7).abs() < 1e-6);
        assert_eq!(c.cooldown_secs, 60, "untouched field should remain");
    }

    #[test]
    fn reset_fields_all() {
        let mut c = SkillsAutoReviewConfig {
            min_reuse_probability: 0.1,
            cooldown_secs: 60,
            review_system_override: Some("custom".to_string()),
            ..Default::default()
        };
        c.reset_fields(None);
        let d = SkillsAutoReviewConfig::default();
        assert!((c.min_reuse_probability - d.min_reuse_probability).abs() < 1e-6);
        assert_eq!(c.cooldown_secs, d.cooldown_secs);
        assert!(c.review_system_override.is_none());
    }
}
