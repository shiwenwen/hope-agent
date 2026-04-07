// Tauri-specific global: AppHandle for window management and event emission.
// All other globals (APP_LOGGER, MEMORY_BACKEND, etc.) are in oc-core.

pub(crate) static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Get stored AppHandle for Tauri window management
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}
