//! Dreaming configuration — persisted under `AppConfig.dreaming`.
//!
//! 类型定义已下沉 [`ha_config_schema::memory::dreaming`]，此处原地再导出保持
//! `crate::memory::dreaming::*` 路径不变；测试留在本 crate。

pub use ha_config_schema::memory::dreaming::{
    CronTriggerConfig, DeepResolverConfig, DreamingConfig, IdleTriggerConfig,
    ProfileSynthesisConfig, PromotionThresholds,
};

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
