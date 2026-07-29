//! Auto-update configuration (`AppConfig.auto_update`).
//!
//! 类型定义已下沉 [`ha_config_schema::updater`]，此处原地再导出保持
//! `crate::config::*` 路径不变；测试留在本 crate。

pub use ha_config_schema::updater::{
    AutoUpdateConfig, MAX_CHECK_INTERVAL_HOURS, MIN_CHECK_INTERVAL_HOURS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_opt_out() {
        let c = AutoUpdateConfig::default();
        assert!(c.check_enabled);
        assert!(c.auto_download);
        assert!(c.notify);
        assert_eq!(c.check_interval_hours, 0.5);
    }

    #[test]
    fn interval_clamps_both_ends() {
        let mk = |h: f64| AutoUpdateConfig {
            check_interval_hours: h,
            ..Default::default()
        };
        assert_eq!(mk(0.0).clamped_interval_hours(), MIN_CHECK_INTERVAL_HOURS);
        assert_eq!(
            mk(10_000.0).clamped_interval_hours(),
            MAX_CHECK_INTERVAL_HOURS
        );
        assert_eq!(mk(6.0).clamped_interval_hours(), 6.0);
        assert_eq!(mk(f64::NAN).clamped_interval_hours(), 0.5);
        assert_eq!(mk(0.5).clamped_interval_secs(), 1800);
    }

    #[test]
    fn empty_object_deserializes_to_defaults() {
        let c: AutoUpdateConfig = serde_json::from_str("{}").unwrap();
        assert!(c.check_enabled);
        assert_eq!(c.check_interval_hours, 0.5);
    }

    #[test]
    fn integer_and_float_intervals_both_deserialize() {
        for raw in ["12", "12.0", "0.5"] {
            let json = format!(r#"{{"checkIntervalHours":{raw}}}"#);
            let c: AutoUpdateConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(c.check_interval_hours, raw.parse::<f64>().unwrap());
        }
    }
}
