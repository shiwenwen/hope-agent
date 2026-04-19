use tauri;

#[tauri::command]
pub async fn open_directory(path: String) -> Result<(), String> {
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
    open::that(&resolved).map_err(|e| format!("Failed to open directory: {}", e))
}

#[tauri::command]
pub async fn reveal_in_folder(path: String) -> Result<(), String> {
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
            .map_err(|e| format!("Failed to reveal in Finder: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", &resolved))
            .spawn()
            .map_err(|e| format!("Failed to reveal in Explorer: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        // Fallback: open parent directory
        let parent = std::path::Path::new(&resolved)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(resolved);
        open::that(&parent).map_err(|e| format!("Failed to open folder: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    open::that(&url).map_err(|e| format!("Failed to open URL: {}", e))
}

/// Write exported content to a file (used by slash command /export).
#[tauri::command]
pub async fn write_export_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("Failed to write export file: {}", e))
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
pub fn set_dangerous_skip_all_approvals(enabled: bool) -> Result<(), String> {
    let mut store = ha_core::config::load_config().map_err(|e| e.to_string())?;
    store.dangerous_skip_all_approvals = enabled;
    let _reason = ha_core::backup::scope_save_reason("security", "ui");
    ha_core::config::save_config(&store).map_err(|e| e.to_string())?;
    drop(_reason);
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": "security" }),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn set_window_theme(is_dark: bool, app_handle: tauri::AppHandle) -> Result<(), String> {
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
