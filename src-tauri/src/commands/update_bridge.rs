//! Desktop bridge wiring `tauri-plugin-updater` into
//! `ha_core::updater::UpdaterBridge`.
//!
//! Registered at startup ([`super::super::setup`]) so the `app_update`
//! tool transparently routes desktop installs through the same signed
//! installer flow `tauri-plugin-updater` already handled when the user
//! clicked "About → Check for Updates" in the menu.
//!
//! The bridge does NOT call `app.restart()` automatically — the model
//! asks the user when to relaunch via `ask_user_question` so an
//! in-flight chat turn doesn't get cut off mid-sentence.

use std::sync::Arc;

use async_trait::async_trait;
use ha_core::updater::UpdaterBridge;
use serde_json::json;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

pub struct TauriUpdaterBridge {
    handle: AppHandle,
}

impl TauriUpdaterBridge {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl UpdaterBridge for TauriUpdaterBridge {
    async fn install_and_restart(&self, job_id: &str) -> anyhow::Result<String> {
        let updater = self
            .handle
            .updater()
            .map_err(|e| anyhow::anyhow!("get updater: {e}"))?;
        let update = updater
            .check()
            .await
            .map_err(|e| anyhow::anyhow!("check updates: {e}"))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "tauri-plugin-updater reports no update available — the manifest may have rolled back since the agent confirmed"
                )
            })?;
        let version = update.version.clone();
        let job_id_owned = job_id.to_string();
        let job_for_progress = job_id_owned.clone();
        let job_for_done = job_id_owned.clone();
        update
            .download_and_install(
                move |chunk, total| {
                    if let Some(bus) = ha_core::globals::get_event_bus() {
                        bus.emit(
                            "app_update:progress",
                            json!({
                                "job_id": job_for_progress,
                                "label": "tauri_install",
                                "phase": "downloading",
                                "chunk": chunk,
                                "total": total,
                            }),
                        );
                    }
                },
                move || {
                    if let Some(bus) = ha_core::globals::get_event_bus() {
                        bus.emit(
                            "app_update:progress",
                            json!({
                                "job_id": job_for_done,
                                "label": "tauri_install",
                                "phase": "installed",
                            }),
                        );
                    }
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("download_and_install: {e}"))?;
        Ok(format!(
            "Hope Agent {version} installed via tauri-plugin-updater. Restart the app (Cmd/Ctrl+R from the desktop menu, or relaunch from the dock/taskbar) to complete the upgrade."
        ))
    }
}

/// Install the bridge into the global registry. Idempotent.
pub fn register(handle: AppHandle) {
    let bridge: Arc<dyn UpdaterBridge> = Arc::new(TauriUpdaterBridge::new(handle));
    ha_core::updater::set_updater_bridge(bridge);
}
