// Tauri-specific global: AppHandle for window management and event emission.
// All other globals (APP_LOGGER, MEMORY_BACKEND, etc.) are in ha-core.

use std::sync::OnceLock;

pub(crate) static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Get stored AppHandle for Tauri window management
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}
