use crate::browser_ui;
use crate::commands::CmdError;

#[tauri::command]
pub async fn browser_get_status() -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::get_status().await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_list_profiles() -> Result<Vec<browser_ui::BrowserProfileInfo>, CmdError> {
    browser_ui::list_profiles().await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_create_profile(
    name: String,
) -> Result<browser_ui::BrowserProfileInfo, CmdError> {
    browser_ui::create_profile(&name).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_delete_profile(name: String) -> Result<(), CmdError> {
    browser_ui::delete_profile(&name).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_launch(
    options: browser_ui::LaunchOptions,
) -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::launch(options).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_connect(url: String) -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::connect(&url).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_disconnect() -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::disconnect().await.map_err(Into::into)
}

/// Snapshot the active tab as a JPEG frame for the chat BrowserPanel mirror.
///
/// Returns `None` when no backend is currently active (the panel renders an
/// empty state in that case). Frame quality is fixed at JPEG~70 — paying the
/// SSIM hit is worth it at 1Hz polling for ~50–200KB payloads.
#[tauri::command]
pub async fn browser_capture_frame(
) -> Result<Option<ha_core::browser::frame::BrowserFramePayload>, CmdError> {
    ha_core::browser::frame::capture_frame()
        .await
        .map_err(Into::into)
}

/// Spawn the user's daily Chrome into hope-agent's user-attach profile, then
/// hand the debug URL back so the frontend can immediately follow up with
/// `browser_connect`. See [`ha_core::browser::user_attach`].
#[tauri::command]
pub async fn browser_spawn_user_chrome(
    args: ha_core::browser::user_attach::SpawnUserChromeArgs,
) -> Result<ha_core::browser::user_attach::SpawnUserChromeResult, CmdError> {
    ha_core::browser::user_attach::spawn_user_chrome(args)
        .await
        .map_err(Into::into)
}

/// Single combined doctor report: Node toolchain, current backend
/// preference, active backend, debug-port probe, and "is Chrome already
/// running" hint. The settings panel refreshes this in one round-trip.
#[tauri::command]
pub async fn browser_doctor() -> Result<ha_core::browser::user_attach::BrowserDoctorReport, CmdError>
{
    Ok(ha_core::browser::user_attach::browser_doctor().await)
}

/// Read `AppConfig.browser` for the settings panel.
#[tauri::command]
pub async fn browser_get_config() -> Result<ha_core::browser::BrowserConfig, CmdError> {
    Ok(ha_core::config::cached_config()
        .browser
        .clone()
        .unwrap_or_default())
}

/// Persist `AppConfig.browser` from the settings panel. Resets the
/// active-backend cache so a `backend` preference change takes effect on
/// the very next `acquire_backend()` call — otherwise users would have to
/// disconnect/reconnect to pick up the new choice.
#[tauri::command]
pub async fn browser_set_config(config: ha_core::browser::BrowserConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config::<_, ()>(("browser", "settings-ui"), |cfg| {
        cfg.browser = Some(config);
        Ok(())
    })?;
    ha_core::browser::reset_backend().await;
    Ok(())
}

/// Download + unpack the pinned Chromium snapshot for systems with no
/// Chrome installed. Idempotent. Progress is emitted on the
/// `browser:chromium_download_progress` EventBus channel so the settings
/// panel can render a progress bar.
#[tauri::command]
pub async fn browser_install_chromium_runtime() -> Result<ChromiumRuntimeResult, CmdError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let last_percent = Arc::new(AtomicU64::new(u64::MAX));
    let progress_last_percent = Arc::clone(&last_percent);
    let progress = move |downloaded: u64, total: Option<u64>| {
        let percent = total
            .and_then(|t| downloaded.checked_mul(100).and_then(|n| n.checked_div(t)))
            .map(|p| p.min(100));
        let report_pct = percent.unwrap_or(u64::MAX);
        let prev = progress_last_percent.load(Ordering::Relaxed);
        if prev == u64::MAX || (report_pct != u64::MAX && report_pct != prev) {
            progress_last_percent.store(report_pct, Ordering::Relaxed);
            if let Some(bus) = ha_core::globals::EVENT_BUS.get() {
                bus.emit(
                    "browser:chromium_download_progress",
                    serde_json::json!({
                        "stage": "downloading",
                        "percent": percent,
                        "downloadedBytes": downloaded,
                        "totalBytes": total,
                    }),
                );
            }
        }
    };

    let binary = ha_core::browser::runtime::ensure_chromium(progress).await?;
    if let Some(bus) = ha_core::globals::EVENT_BUS.get() {
        bus.emit(
            "browser:chromium_download_progress",
            serde_json::json!({
                "stage": "ready",
                "percent": 100,
                "binaryPath": binary.display().to_string(),
            }),
        );
    }
    Ok(ChromiumRuntimeResult {
        binary_path: binary.display().to_string(),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromiumRuntimeResult {
    pub binary_path: String,
}
