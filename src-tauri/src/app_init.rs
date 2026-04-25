use crate::AppState;

/// Initialize all databases, subsystems, and construct the `AppState`.
/// `init_runtime` sets every OnceLock; `build_app_state` reads them back
/// and assembles the desktop-only `AppState` value.
pub(crate) fn init_tauri_app_state() -> AppState {
    ha_core::init_runtime();
    ha_core::build_app_state()
}
