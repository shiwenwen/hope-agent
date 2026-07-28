//! Browser configuration (`AppConfig.browser`).
//!
//! Wire 类型自 ha-core `browser/mod.rs`（含 `browser/extension/mod.rs`）下沉；
//! 运行时逻辑（backend / profile / launch 等）仍留在 ha-core。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod extension;

pub use extension::{BrowserExtensionConfig, DEFAULT_NATIVE_HOST_NAME};

/// UI-only preference: which tab the settings BrowserPanel opens on
/// (Standalone vs. Take-over-user-Chrome). The actual runtime path is
/// decided by *which profile the user picks*. No backend code reads
/// `default_mode`; treat it as remembered UI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    #[default]
    Managed,
    UserAttach,
}

/// Browser backend preference. `ExtensionFirst` is the product default:
/// use the Chrome Extension backend when it is connected, and fall back to
/// CDP only for actions that do not require the user's real Chrome state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBackendPreference {
    #[default]
    ExtensionFirst,
    CdpOnly,
    ExtensionOnly,
}

/// Persisted browser configuration. Stored under `AppConfig.browser`.
///
/// All fields are optional so omitting the block in `config.json` yields
/// the same zero-config defaults the legacy version had.
///
/// Schema evolution notes:
/// - A previous `backend` field selected between CDP and an external bridge;
///   the external backend was removed and any leftover `"backend"` key in old
///   `config.json` is silently ignored by serde.
/// - A previous `userAttach: { lastSpawnedPort }` field tracked the
///   user-attach Chrome port bookkeeping; user_attach is now a first-class
///   entry in [`Self::profiles`] (always port 9222), so the legacy key is
///   silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConfig {
    /// Runtime backend preference. `None` = `ExtensionFirst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_preference: Option<BrowserBackendPreference>,
    /// Chrome Extension + Native Messaging integration config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<BrowserExtensionConfig>,
    /// UI-only opening tab. `None` = `Managed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<BrowserMode>,
    /// Default profile name when `profile.op=launch` is called with no
    /// `profile=` argument. `None` = `"managed"` (ephemeral).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// User-defined Chrome profiles. Two well-known names — `"managed"`
    /// and `"user_attach"` — are always present (synthesised at resolve
    /// time when absent here); user entries override their defaults and
    /// can add arbitrary new profiles.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, BrowserProfileConfig>,
    /// Heartbeat probe interval in seconds. `None` / `Some(0)` use the
    /// default (120s). Clamped to `[30, 600]` at read time. Defeats Chrome's
    /// ~4-minute WebSocket idle close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_secs: Option<u32>,
    /// Launch failure circuit breaker thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_circuit: Option<LaunchCircuitConfig>,
}

/// Per-profile launch circuit breaker tuning. Defaults: 3 consecutive
/// failures → 60s cooldown. `failure_threshold = 0` disables the breaker.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchCircuitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_threshold: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_secs: Option<u64>,
}

/// Per-profile launch configuration. All fields optional — absent means
/// "use the built-in default for this profile name". See
/// `crate::browser::profile::resolve_profile` (ha-core) for the resolution
/// table.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfileConfig {
    /// Absolute or `~`-relative path. None → derive from profile name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<String>,
    /// Fixed `--remote-debugging-port`. None means OS-pick for managed,
    /// 9222 for user_attach, OS-pick for user-defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Chrome / Chromium binary override. None = platform auto-probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    /// Launch headless. None = environment default (headed on desktop,
    /// headless for Docker / no-display Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,
    /// Verbatim extra Chrome args (e.g. `["--proxy-server=..."]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
    /// UI tint (CSS color). Pure UI hint, no behaviour impact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}
