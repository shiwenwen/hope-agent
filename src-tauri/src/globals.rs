// Tauri-specific global: AppHandle for window management and event emission.
// All other globals (APP_LOGGER, MEMORY_BACKEND, etc.) are in ha-core.

use std::sync::{Arc, OnceLock};

pub(crate) static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Get stored AppHandle for Tauri window management
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

// Exposed for read-only status queries that need an active-session count
// without pulling `ha_server::AppContext` into Tauri command land.
static CHAT_STREAM_REGISTRY: OnceLock<Arc<ha_server::ws::chat_stream::ChatStreamRegistry>> =
    OnceLock::new();

pub(crate) fn set_chat_stream_registry(r: Arc<ha_server::ws::chat_stream::ChatStreamRegistry>) {
    let _ = CHAT_STREAM_REGISTRY.set(r);
}

pub fn chat_stream_registry() -> Option<Arc<ha_server::ws::chat_stream::ChatStreamRegistry>> {
    CHAT_STREAM_REGISTRY.get().cloned()
}
