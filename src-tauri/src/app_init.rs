use oc_core::config::AppConfig;
use crate::AppState;

/// Initialize all databases, subsystems, and construct the `AppState`.
/// Delegates to oc-core's `init_app_state` which sets up all OnceLocks.
pub(crate) fn init_tauri_app_state(initial_store: AppConfig) -> AppState {
    oc_core::init_app_state(initial_store)
}
