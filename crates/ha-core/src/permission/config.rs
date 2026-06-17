//! Global permission configuration — lives at `AppConfig.permission`.

use serde::{Deserialize, Serialize};

use super::mode::SmartModeConfig;

/// Default approval timeout (seconds) used when auto-expiry is enabled.
pub fn default_approval_timeout_secs() -> u64 {
    300
}

/// Default throttle for the IM text-mode "you have N pending approvals"
/// hint. One nudge per (account, chat) per this many seconds.
pub fn default_im_approval_hint_throttle_secs() -> u64 {
    60
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

/// What to do when a tool approval is requested on an **unattended** surface —
/// a turn where no human can possibly respond (cron run, headless server with
/// no connected client and no IM-attached chat, ACP client without a permission
/// capability, or a subagent whose parent chain has no surface). Detected by
/// [`super::approval_surface::evaluate_approval_surface`] (Epic D).
///
/// The point of detecting this is to **stop hanging forever** — the old
/// behavior left the turn / HTTP request / cron run blocked on an approval
/// nobody could answer, then a generic whole-job timeout masked the real cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnattendedApprovalAction {
    /// Fail-closed: immediately deny the tool (no wait) with a structured
    /// "no one could approve" reason. The safe default.
    #[default]
    Deny,
    /// Auto-proceed: run the tool as if approved. For deliberate headless
    /// automation; narrower than global YOLO (only fires when the surface is
    /// genuinely unattended, not on every interactive approval).
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

    /// Whether pending approval dialogs automatically expire.
    #[serde(default)]
    pub approval_timeout_enabled: bool,

    /// Timeout for approval dialogs (seconds). 0 = wait forever even when
    /// `approval_timeout_enabled` is true.
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,

    /// What to do when an approval times out.
    #[serde(default)]
    pub approval_timeout_action: ApprovalTimeoutAction,

    /// What to do when an approval is requested on an **unattended** surface
    /// (no human can respond — cron / headless-no-client / ACP-no-capability /
    /// subagent-no-parent-surface). Default [`UnattendedApprovalAction::Deny`]
    /// (fail-closed, no hang). See [`super::approval_surface`].
    #[serde(default)]
    pub unattended_approval_action: UnattendedApprovalAction,

    /// Throttle (seconds) for the IM text-mode "you have N pending
    /// approvals" hint. Only consumed by button-less channels; one nudge
    /// per (account, chat) per interval so casual chitchat during a
    /// pending approval window doesn't spam the user. Default 60s.
    #[serde(default = "default_im_approval_hint_throttle_secs")]
    pub im_approval_hint_throttle_secs: u64,
}

impl Default for PermissionGlobalConfig {
    fn default() -> Self {
        Self {
            global_yolo: false,
            smart: SmartModeConfig::default(),
            approval_timeout_enabled: false,
            approval_timeout_secs: default_approval_timeout_secs(),
            approval_timeout_action: ApprovalTimeoutAction::default(),
            unattended_approval_action: UnattendedApprovalAction::default(),
            im_approval_hint_throttle_secs: default_im_approval_hint_throttle_secs(),
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
    fn default_timeout_disabled() {
        assert!(!PermissionGlobalConfig::default().approval_timeout_enabled);
    }

    #[test]
    fn timeout_action_default_deny() {
        assert_eq!(
            PermissionGlobalConfig::default().approval_timeout_action,
            ApprovalTimeoutAction::Deny
        );
    }
}
