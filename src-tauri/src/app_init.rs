use crate::AppState;
use ha_core::config::AppConfig;

/// Initialize all databases, subsystems, and construct the `AppState`.
/// Delegates to ha-core's `init_app_state` which sets up all OnceLocks.
pub(crate) fn init_tauri_app_state(initial_store: AppConfig) -> AppState {
    ha_core::init_app_state(initial_store)
}
