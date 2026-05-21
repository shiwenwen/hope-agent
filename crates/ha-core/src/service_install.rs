//! Compatibility wrappers for Hope Agent user-service management.
//!
//! The public API lives here because existing CLI / updater / Tauri call sites
//! import `ha_core::service_install`. OS-specific launchd / systemd /
//! Task Scheduler behavior lives in `crate::platform::service`.

use anyhow::Result;

/// Install Hope Agent as a user-level background service.
pub fn install_service(bind_addr: &str, api_key: Option<&str>) -> Result<String> {
    crate::platform::service::install_service(bind_addr, api_key)
}

/// Uninstall the Hope Agent system service.
pub fn uninstall_service() -> Result<()> {
    crate::platform::service::uninstall_service()
}

/// Query the current status of the Hope Agent system service.
pub fn service_status() -> Result<String> {
    crate::platform::service::service_status()
}

/// Stop the running Hope Agent server.
pub fn stop_server() -> Result<()> {
    crate::platform::service::stop_server()
}
