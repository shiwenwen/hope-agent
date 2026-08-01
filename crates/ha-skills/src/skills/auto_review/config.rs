//! Auto-review configuration (`AppConfig.skills.auto_review`).
//!
//! 类型定义已下沉 [`ha_config_schema::skills`]，此处原地再导出保持
//! `super::config::*` / `crate::skills::auto_review::config::*` 路径不变；
//! 测试留在本 crate。
//!
//! Five-gate waterfall pipeline; gate enforcement lives in `triggers.rs`
//! (gate 1), `heuristics.rs` (gates 2 & 5), and `pipeline.rs` (gates 3 & 4).
//! Defaults skew strict — surface false-negatives in the UI rather than ship
//! false-positive drafts to the user.

pub use ha_config_schema::skills::{AutoReviewPromotion, SkillsAutoReviewConfig};

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
