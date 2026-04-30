use crate::commands::CmdError;
use anyhow::Context;
use tauri;

#[tauri::command]
pub async fn open_directory(path: String) -> Result<(), CmdError> {
    // Resolve ~ to home directory
    let resolved = if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..]).to_string_lossy().to_string()
        } else {
            path
        }
    } else {
        path
    };
    open::that(&resolved).context("Failed to open directory")?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_in_folder(path: String) -> Result<(), CmdError> {
    let resolved = if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..]).to_string_lossy().to_string()
        } else {
            path
        }
    } else {
        path
    };
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

#[tauri::command]
pub async fn open_url(url: String) -> Result<(), CmdError> {
    open::that(&url).context("Failed to open URL")?;
    Ok(())
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
