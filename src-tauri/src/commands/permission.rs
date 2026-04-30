//! Tauri commands for the permission system v2 — three user-editable
//! pattern lists (protected paths / dangerous / edit commands), Smart mode
//! configuration, and Global YOLO state.

use ha_core::permission::{dangerous_commands, edit_commands, protected_paths, SmartModeConfig};
use serde::Serialize;

use crate::commands::CmdError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternListPayload {
    /// User-customized list (or defaults when no on-disk override exists).
    pub current: Vec<String>,
    /// Compile-time defaults for the "Restore defaults" button preview.
    pub defaults: Vec<&'static str>,
}

// ── Protected paths ──────────────────────────────────────────────

#[tauri::command]
pub fn get_protected_paths() -> Result<PatternListPayload, CmdError> {
    Ok(PatternListPayload {
        current: (*protected_paths::current_patterns()).clone(),
        defaults: protected_paths::defaults().to_vec(),
    })
}

#[tauri::command]
pub fn set_protected_paths(patterns: Vec<String>) -> Result<(), CmdError> {
    protected_paths::save_patterns(&patterns)?;
    Ok(())
}

#[tauri::command]
pub fn reset_protected_paths() -> Result<Vec<String>, CmdError> {
    Ok(protected_paths::reset_defaults()?)
}

// ── Dangerous commands ───────────────────────────────────────────

#[tauri::command]
pub fn get_dangerous_commands() -> Result<PatternListPayload, CmdError> {
    Ok(PatternListPayload {
        current: (*dangerous_commands::current_patterns()).clone(),
        defaults: dangerous_commands::defaults().to_vec(),
    })
}

#[tauri::command]
pub fn set_dangerous_commands(patterns: Vec<String>) -> Result<(), CmdError> {
    dangerous_commands::save_patterns(&patterns)?;
    Ok(())
}

#[tauri::command]
pub fn reset_dangerous_commands() -> Result<Vec<String>, CmdError> {
    Ok(dangerous_commands::reset_defaults()?)
}

// ── Edit commands ────────────────────────────────────────────────

#[tauri::command]
pub fn get_edit_commands() -> Result<PatternListPayload, CmdError> {
    Ok(PatternListPayload {
        current: (*edit_commands::current_patterns()).clone(),
        defaults: edit_commands::defaults().to_vec(),
    })
}

#[tauri::command]
pub fn set_edit_commands(patterns: Vec<String>) -> Result<(), CmdError> {
    edit_commands::save_patterns(&patterns)?;
    Ok(())
}

#[tauri::command]
pub fn reset_edit_commands() -> Result<Vec<String>, CmdError> {
    Ok(edit_commands::reset_defaults()?)
}

// ── Smart mode config ────────────────────────────────────────────

#[tauri::command]
pub async fn get_smart_mode_config() -> Result<SmartModeConfig, CmdError> {
    Ok(ha_core::config::cached_config().permission.smart.clone())
}

#[tauri::command]
pub async fn set_smart_mode_config(config: SmartModeConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("permission.smart", "settings-ui"), |store| {
        store.permission.smart = config;
        Ok(())
    })?;
    Ok(())
}

// ── Global YOLO read accessor ────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalYoloStatus {
    pub cli_flag: bool,
    pub config_flag: bool,
    pub active: bool,
}

#[tauri::command]
pub fn get_global_yolo_status() -> GlobalYoloStatus {
    let s = ha_core::security::dangerous::status();
    GlobalYoloStatus {
        cli_flag: s.cli_flag,
        config_flag: s.config_flag,
        active: s.active,
    }
}
