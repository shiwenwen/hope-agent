//! Global permission configuration — lives at `AppConfig.permission`.
//!
//! 类型定义已下沉 [`ha_config_schema::permission`]，此处原地再导出保持
//! `crate::permission::config::*` 路径不变；测试留在本 crate。

pub use ha_config_schema::permission::{
    default_approval_timeout_secs, default_im_approval_hint_throttle_secs, ApprovalTimeoutAction,
    PermissionGlobalConfig, UnattendedApprovalAction,
};

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
