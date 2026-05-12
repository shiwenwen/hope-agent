//! Desktop bridge wiring Tauri's `AppHandle::exit(42)` into
//! `ha_core::lifecycle::AppLifecycleBridge`.
//!
//! Registered at startup ([`super::super::setup`]) so the `app_restart`
//! tool / `/restart` slash command / "重启 App" GUI button all funnel
//! through one place. The `exit(42)` code is the same one
//! [`crate::commands::crash::request_app_restart`] uses — the guardian
//! process (see `ha_core::guardian::run_guardian`) catches it and
//! re-spawns the GUI child.

use std::sync::Arc;

use ha_core::lifecycle::AppLifecycleBridge;
use tauri::AppHandle;

pub struct TauriLifecycleBridge {
    handle: AppHandle,
}

impl TauriLifecycleBridge {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle }
    }
}

impl AppLifecycleBridge for TauriLifecycleBridge {
    fn restart_desktop(&self) -> anyhow::Result<()> {
        // `app.exit(code)` is async-fire-and-forget — control returns to us
        // before the runtime actually tears down. The lifecycle module's
        // contract is exactly that: queue the exit and return.
        self.handle.exit(42);
        Ok(())
    }
}

/// Install the bridge into the global registry. Idempotent.
pub fn register(handle: AppHandle) {
    let bridge: Arc<dyn AppLifecycleBridge> = Arc::new(TauriLifecycleBridge::new(handle));
    ha_core::lifecycle::set_lifecycle_bridge(bridge);
}
