//! Chrome Extension + Native Messaging integration.
//!
//! The native host is only a local transport bridge. Runtime policy, tab
//! ownership, approval, and backend selection stay in `ha-core`.

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
// 类型已下沉 ha-config-schema：wire 配置原地再导出，
// `crate::browser::extension::BrowserExtensionConfig` 等既有路径不变。
pub use ha_config_schema::browser::extension::{BrowserExtensionConfig, DEFAULT_NATIVE_HOST_NAME};

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
