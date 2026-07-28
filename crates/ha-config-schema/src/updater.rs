//! Auto-update configuration (`AppConfig.auto_update`).
//!
//! Single source of truth shared by both update paths:
//!
//! - **Desktop** reads it from the frontend (via `get_auto_update_config`) to
//!   drive the periodic `@tauri-apps/plugin-updater` check + silent download.
//! - **Headless / server** reads it via `config::cached_config` in the
//!   primary-gated background loop (`updater::auto_check`).
//!
//! Risk class is HIGH (network exposure + service restart + binary swap), so
//! the `ha-settings` skill must confirm before writing.

use serde::{Deserialize, Serialize};

/// Lower / upper clamps for the periodic check interval. A half-hour floor
/// allows quick release pickup without turning a bad value into a tight loop;
/// one week ceiling keeps "enabled" meaningful.
pub const MIN_CHECK_INTERVAL_HOURS: f64 = 0.5;
pub const MAX_CHECK_INTERVAL_HOURS: f64 = 168.0;

fn default_check_interval_hours() -> f64 {
    MIN_CHECK_INTERVAL_HOURS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdateConfig {
    /// Run a periodic background check for new releases. Default `true`.
    #[serde(default = "crate::default_true")]
    pub check_enabled: bool,
    /// Hours between background checks. Clamped to
    /// `[MIN_CHECK_INTERVAL_HOURS, MAX_CHECK_INTERVAL_HOURS]`. Default 0.5.
    #[serde(default = "default_check_interval_hours")]
    pub check_interval_hours: f64,
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
    pub fn clamped_interval_hours(&self) -> f64 {
        let hours = if self.check_interval_hours.is_finite() {
            self.check_interval_hours
        } else {
            default_check_interval_hours()
        };
        hours.clamp(MIN_CHECK_INTERVAL_HOURS, MAX_CHECK_INTERVAL_HOURS)
    }

    pub fn clamped_interval_secs(&self) -> u64 {
        (self.clamped_interval_hours() * 3600.0).round() as u64
    }
}
