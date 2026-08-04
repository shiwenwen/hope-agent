//! Chrome Extension + Native Messaging 配置（`AppConfig.browser.extension`）。
//!
//! 仅 wire 类型；native host / broker / registry 等运行时逻辑留在 ha-core
//! `browser/extension/`。

use serde::{Deserialize, Serialize};

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
