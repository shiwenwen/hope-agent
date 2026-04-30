//! Global permission configuration — lives at `AppConfig.permission`.

use serde::{Deserialize, Serialize};

use super::mode::SmartModeConfig;

/// Default approval timeout (seconds) — 5 minutes.
pub fn default_approval_timeout_secs() -> u64 {
    300
}

/// What to do when a tool approval request times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutAction {
    /// Block tool execution when approval timed out.
    #[default]
    Deny,
    /// Continue tool execution when approval timed out.
    Proceed,
}

/// Top-level permission configuration block, nested under `AppConfig.permission`.
///
/// Replaces the legacy top-level fields:
/// - `dangerous_skip_all_approvals` → `permission.global_yolo`
/// - `approval_timeout_secs` → `permission.approval_timeout_secs`
/// - `approval_timeout_action` → `permission.approval_timeout_action`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionGlobalConfig {
    /// ⚠️ Global YOLO — bypasses ALL tool approvals (with `app_warn!` audit
    /// logs). Even matches against the protected-paths / dangerous-commands /
    /// edit-commands lists are silently allowed. Only Plan Mode can still
    /// block. Settable via GUI ("Settings → Permission → Global YOLO") or via
    /// the `--dangerously-skip-all-approvals` CLI flag.
    #[serde(default)]
    pub global_yolo: bool,

    /// Smart mode configuration (used when a session's `permission_mode = smart`).
    #[serde(default)]
    pub smart: SmartModeConfig,

    /// Timeout for approval dialogs (seconds). 0 = wait forever.
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,

    /// What to do when an approval times out.
    #[serde(default)]
    pub approval_timeout_action: ApprovalTimeoutAction,
}

impl Default for PermissionGlobalConfig {
    fn default() -> Self {
        Self {
            global_yolo: false,
            smart: SmartModeConfig::default(),
            approval_timeout_secs: default_approval_timeout_secs(),
            approval_timeout_action: ApprovalTimeoutAction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_yolo_off() {
        assert!(!PermissionGlobalConfig::default().global_yolo);
    }

    #[test]
    fn default_timeout_300s() {
        assert_eq!(PermissionGlobalConfig::default().approval_timeout_secs, 300);
    }

    #[test]
    fn timeout_action_default_deny() {
        assert_eq!(
            PermissionGlobalConfig::default().approval_timeout_action,
            ApprovalTimeoutAction::Deny
        );
    }
}
