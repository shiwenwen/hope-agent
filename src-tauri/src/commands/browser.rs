use crate::browser_ui;
use crate::commands::CmdError;

#[tauri::command]
pub async fn browser_get_status() -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::get_status().await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_extension_status() -> Result<ha_core::browser::BrowserExtensionStatus, CmdError>
{
    Ok(browser_ui::extension_status())
}

#[tauri::command]
pub async fn browser_install_native_host_manifest(
    request: ha_core::browser::NativeHostInstallRequest,
) -> Result<ha_core::browser::NativeHostInstallResult, CmdError> {
    browser_ui::install_native_host_manifest(request).map_err(Into::into)
}

#[tauri::command]
pub async fn browser_extension_stop_control(
) -> Result<ha_core::browser::BrowserExtensionStopResult, CmdError> {
    Ok(browser_ui::stop_extension_control().await)
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
    session_id: Option<String>,
) -> Result<Option<ha_core::browser::frame::BrowserFramePayload>, CmdError> {
    ha_core::browser::frame::capture_frame(session_id.as_deref())
        .await
        .map_err(Into::into)
}

/// Panel quick-bar navigation (`go` / `back` / `reload`) for the mirrored tab.
#[tauri::command]
pub async fn browser_panel_navigate(
    op: String,
    url: Option<String>,
    session_id: Option<String>,
) -> Result<(), CmdError> {
    ha_core::browser_ui::panel_navigate(&op, url.as_deref(), session_id.as_deref())
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
/// Chrome installed. Idempotent. Progress events flow through
/// [`ha_core::browser::runtime::install_with_event_bus_progress`] on the
/// `browser:chromium_download_progress` channel so the settings panel
/// can render a progress bar.
#[tauri::command]
pub async fn browser_install_chromium_runtime() -> Result<ChromiumRuntimeResult, CmdError> {
    let binary = ha_core::browser::runtime::install_with_event_bus_progress().await?;
    Ok(ChromiumRuntimeResult {
        binary_path: binary.display().to_string(),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromiumRuntimeResult {
    pub binary_path: String,
}
