//! Chrome Extension + Native Messaging integration.
//!
//! The native host is only a local transport bridge. Runtime policy, tab
//! ownership, approval, and backend selection stay in `ha-core`.

use serde::{Deserialize, Serialize};

pub mod backend;
pub mod broker;
pub mod diagnostics;
mod embedded;
pub mod events;
pub mod registry;

pub use backend::{
    cleanup_extension_session, schedule_extension_turn_finalize, stop_all_extension_control,
    BrowserExtensionStopResult, ExtensionBackend,
};
pub use broker::{BrokerStatus, BrowserBrokerDiscovery, BrowserExtensionBroker};
pub use diagnostics::{
    current_status, ensure_local_unpacked_extension, ensure_native_host_registered,
    install_native_host_manifest, BrowserExtensionStatus, BrowserExtensionStatusKind,
    NativeHostInstallRequest, NativeHostInstallResult,
};

/// Runtime context for a browser backend acquisition. It is intentionally
/// small for the first slice; future broker work will use the same fields to
/// scope claimed tabs, frame events, observe cursors, and pending requests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserBackendContext {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub source: Option<String>,
}

/// Whether a browser action may fall back to CDP when the Chrome Extension is
/// missing. Real user-Chrome state must never silently fall back to a managed
/// CDP profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserBackendRequirement {
    /// Needs real Chrome tabs / logged-in user state.
    ExtensionRequired,
    /// Prefer real Chrome, but CDP is semantically acceptable.
    #[default]
    ExtensionPreferred,
    /// CDP-specific lifecycle work such as profile launch/connect.
    CdpAllowed,
}

impl BrowserBackendRequirement {
    pub fn as_event_str(self) -> &'static str {
        match self {
            Self::ExtensionRequired => "extension_required",
            Self::ExtensionPreferred => "extension_preferred",
            Self::CdpAllowed => "cdp_allowed",
        }
    }
}

/// Config for the Chrome Extension + Native Messaging backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extension_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_control_overlay: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_raw_cdp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_secs: Option<u32>,
}

impl Default for BrowserExtensionConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            native_host_name: Some(DEFAULT_NATIVE_HOST_NAME.to_string()),
            extension_ids: Vec::new(),
            store_url: None,
            show_control_overlay: Some(true),
            allow_raw_cdp: Some(true),
            heartbeat_interval_secs: Some(15),
        }
    }
}

impl BrowserExtensionConfig {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn native_host_name(&self) -> &str {
        self.native_host_name
            .as_deref()
            .unwrap_or(DEFAULT_NATIVE_HOST_NAME)
    }

    /// Whether the `control.raw_cdp` escape hatch is permitted. Defaults to
    /// `true` when unset. Setting it to `false` is a hard kill switch enforced
    /// in `control_raw_cdp` — the agent cannot send raw DevTools Protocol at all.
    pub fn allow_raw_cdp(&self) -> bool {
        self.allow_raw_cdp.unwrap_or(true)
    }
}

pub const DEFAULT_NATIVE_HOST_NAME: &str = "com.hope_agent.chrome";
