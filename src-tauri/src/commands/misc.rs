use crate::commands::CmdError;
use anyhow::{anyhow, Context};
use ha_core::app_info;
use std::path::Path;
use tauri;

pub(crate) fn resolve_user_path(path: String) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path[2..]).to_string_lossy().to_string();
        }
    }
    path
}

fn ensure_existing_path(path: &str) -> Result<(), CmdError> {
    if Path::new(path).exists() {
        Ok(())
    } else {
        Err(CmdError::from(anyhow!("Path does not exist: {}", path)))
    }
}

#[tauri::command]
pub async fn open_directory(path: String) -> Result<(), CmdError> {
    let resolved = resolve_user_path(path);
    ensure_existing_path(&resolved)?;
    open::that(&resolved).context("Failed to open directory")?;
    Ok(())
}

/// Reflect the global desktop unread total on the app icon / Dock badge. `0`
/// clears it. macOS renders a red Dock badge; other platforms are best-effort
/// (no-op where unsupported). Desktop-only — the frontend gates on
/// `isTauriMode()`, so HTTP/web never reaches this.
#[tauri::command]
pub fn set_dock_badge_cmd(count: i64, app: tauri::AppHandle) -> Result<(), CmdError> {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        let badge = if count > 0 { Some(count) } else { None };
        window
            .set_badge_count(badge)
            .map_err(|e| anyhow!("Failed to set Dock badge count: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn reveal_in_folder(path: String) -> Result<(), CmdError> {
    let resolved = resolve_user_path(path);
    ensure_existing_path(&resolved)?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&resolved)
            .spawn()
            .context("Failed to reveal in Finder")?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", &resolved))
            .spawn()
            .context("Failed to reveal in Explorer")?;
    }
    #[cfg(target_os = "linux")]
    {
        // Fallback: open parent directory
        let parent = std::path::Path::new(&resolved)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(resolved);
        open::that(&parent).context("Failed to open folder")?;
    }
    Ok(())
}

/// Write user-exported bytes (base64) to a path the user just picked in the
/// native "Save As" dialog (Design Space exports). Desktop-only; there is
/// deliberately **no HTTP route** — remote clients save on their own machine via
/// the File System Access API / browser download, never to the server's disk
/// (that would be a remote-write / exfiltration surface).
#[tauri::command]
pub async fn save_exported_file(path: String, data_base64: String) -> Result<(), CmdError> {
    use base64::Engine;
    let resolved = resolve_user_path(path);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|e| CmdError::from(anyhow!("invalid base64 export payload: {e}")))?;
    std::fs::write(&resolved, &bytes)
        .with_context(|| format!("failed to write exported file: {resolved}"))?;
    app_info!(
        "design",
        "save_exported_file",
        "wrote export to {} ({} bytes)",
        resolved,
        bytes.len()
    );
    Ok(())
}

#[tauri::command]
pub async fn open_url(url: String) -> Result<(), CmdError> {
    // Browser-internal pages (chrome://, edge://, brave://, about:) cannot be
    // handed to the OS URL opener — no application claims those schemes, so
    // `open::that` fails with kLSApplicationNotFoundErr (macOS) / equivalent.
    // Route them to a resolved Chrome process, which understands them.
    if is_browser_internal_url(&url) {
        ha_core::browser::spawn::open_url_in_chrome(&url)
            .context("Failed to open browser-internal URL")?;
        return Ok(());
    }
    open::that(&url).context("Failed to open URL")?;
    Ok(())
}

/// Whether `url` is a Chromium-family internal page that only a browser process
/// can navigate to (not the OS URL handler).
fn is_browser_internal_url(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("chrome://")
        || lower.starts_with("edge://")
        || lower.starts_with("brave://")
        || lower.starts_with("about:")
}

/// Write exported content to a file (used by slash command /export).
#[tauri::command]
pub async fn write_export_file(path: String, content: String) -> Result<(), CmdError> {
    std::fs::write(&path, content).context("Failed to write export file")?;
    Ok(())
}

/// Query whether Dangerous Mode (skip ALL tool approvals) is active, and the
/// source(s) that activated it. The frontend consumes this to render the
/// persistent warning banner and the Settings toggle's read-only state when
/// the CLI flag is active.
#[tauri::command]
pub fn get_dangerous_mode_status() -> ha_core::security::dangerous::DangerousModeStatus {
    ha_core::security::dangerous::status()
}

/// Toggle the persisted `dangerousSkipAllApprovals` flag in `config.json`.
/// This controls one of the two OR'd sources that drive Dangerous Mode; the
/// CLI flag is independent and cannot be cleared via this command.
///
/// Follows the same autosave-backup path as other config writes and emits
/// `config:changed` so subscribed UIs refresh immediately.
#[tauri::command]
pub fn set_dangerous_skip_all_approvals(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("security.dangerous", "settings-ui"), |store| {
        store.permission.global_yolo = enabled;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub async fn set_window_theme(is_dark: bool, app_handle: tauri::AppHandle) -> Result<(), CmdError> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        if let Some(window) = app_handle.get_webview_window("main") {
            let _ = window.with_webview(move |webview| unsafe {
                let ns_window: &objc2_app_kit::NSWindow = &*webview.ns_window().cast();
                let (r, g, b) = if is_dark {
                    (15.0 / 255.0, 15.0 / 255.0, 15.0 / 255.0)
                } else {
                    (1.0, 1.0, 1.0)
                };
                let bg_color =
                    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
                ns_window.setBackgroundColor(Some(&bg_color));
            });
        }
    }
    Ok(())
}
