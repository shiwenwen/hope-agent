//! Auto-update configuration (`AppConfig.auto_update`).
//!
//! Single source of truth shared by both update paths:
//!
//! - **Desktop** reads it from the frontend (via `get_auto_update_config`) to
//!   drive the periodic `@tauri-apps/plugin-updater` check + silent download.
//! - **Headless / server** reads it via [`crate::config::cached_config`] in the
//!   primary-gated background loop ([`super::auto_check`]).
//!
//! Risk class is HIGH (network exposure + service restart + binary swap), so
//! the `ha-settings` skill must confirm before writing.

use serde::{Deserialize, Serialize};

/// Lower / upper clamps for the periodic check interval. One hour floor keeps a
/// misconfigured value from hammering the release server; one week ceiling
/// keeps "enabled" meaningful.
pub const MIN_CHECK_INTERVAL_HOURS: u32 = 1;
pub const MAX_CHECK_INTERVAL_HOURS: u32 = 168;

fn default_check_interval_hours() -> u32 {
    12
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdateConfig {
    /// Run a periodic background check for new releases. Default `true`.
    #[serde(default = "crate::default_true")]
    pub check_enabled: bool,
    /// Hours between background checks. Clamped to
    /// `[MIN_CHECK_INTERVAL_HOURS, MAX_CHECK_INTERVAL_HOURS]`. Default 12.
    #[serde(default = "default_check_interval_hours")]
    pub check_interval_hours: u32,
    /// Silently pre-download + verify the new build when a check finds one, so
    /// installing is instant. Default `true`.
    #[serde(default = "crate::default_true")]
    pub auto_download: bool,
    /// Surface "update available" / "ready to install" to the user (desktop
    /// toast / headless log + event). Default `true`.
    #[serde(default = "crate::default_true")]
    pub notify: bool,
}

impl Default for AutoUpdateConfig {
    fn default() -> Self {
        Self {
            check_enabled: true,
            check_interval_hours: default_check_interval_hours(),
            auto_download: true,
            notify: true,
        }
    }
}

impl AutoUpdateConfig {
    /// Effective check interval in hours, clamped to the supported range.
    pub fn clamped_interval_hours(&self) -> u32 {
        self.check_interval_hours
            .clamp(MIN_CHECK_INTERVAL_HOURS, MAX_CHECK_INTERVAL_HOURS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_opt_out() {
        let c = AutoUpdateConfig::default();
        assert!(c.check_enabled);
        assert!(c.auto_download);
        assert!(c.notify);
        assert_eq!(c.check_interval_hours, 12);
    }

    #[test]
    fn interval_clamps_both_ends() {
        let mk = |h: u32| AutoUpdateConfig {
            check_interval_hours: h,
            ..Default::default()
        };
        assert_eq!(mk(0).clamped_interval_hours(), MIN_CHECK_INTERVAL_HOURS);
        assert_eq!(
            mk(10_000).clamped_interval_hours(),
            MAX_CHECK_INTERVAL_HOURS
        );
        assert_eq!(mk(6).clamped_interval_hours(), 6);
    }

    #[test]
    fn empty_object_deserializes_to_defaults() {
        let c: AutoUpdateConfig = serde_json::from_str("{}").unwrap();
        assert!(c.check_enabled);
        assert_eq!(c.check_interval_hours, 12);
    }
}
