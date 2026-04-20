use crate::AppState;

/// Initialize all databases, subsystems, and construct the `AppState`.
/// Delegates to ha-core's `init_app_state` which sets up all OnceLocks.
pub(crate) fn init_tauri_app_state() -> AppState {
    ha_core::init_app_state()
}
