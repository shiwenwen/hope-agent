use crate::backup;
use crate::commands::CmdError;
use crate::crash_journal;
use crate::paths;

#[tauri::command]
pub async fn get_crash_recovery_info() -> Result<serde_json::Value, CmdError> {
    let recovered = std::env::var("HOPE_AGENT_RECOVERED").is_ok();
    let crash_count: u32 = std::env::var("HOPE_AGENT_CRASH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut info = serde_json::json!({
        "recovered": recovered,
        "crashCount": crash_count,
    });

    // If recovered, load the latest diagnosis from crash journal
    if recovered {
        if let Ok(path) = paths::crash_journal_path() {
            let journal = crash_journal::CrashJournal::load(&path);
            if let Some(last) = journal.crashes.last() {
                if let Some(ref diagnosis) = last.diagnosis_result {
                    info["diagnosis"] = serde_json::to_value(diagnosis).unwrap_or_default();
                }
            }
        }
    }

    Ok(info)
}

#[tauri::command]
pub async fn get_crash_history() -> Result<serde_json::Value, CmdError> {
    let path = paths::crash_journal_path()?;
    let journal = crash_journal::CrashJournal::load(&path);
    Ok(serde_json::to_value(&journal)?)
}

#[tauri::command]
pub async fn clear_crash_history() -> Result<(), CmdError> {
    let path = paths::crash_journal_path()?;
    let mut journal = crash_journal::CrashJournal::load(&path);
    journal.clear();
    journal.save(&path).map_err(CmdError::msg)
}

#[tauri::command]
pub async fn request_app_restart(app: tauri::AppHandle) -> Result<(), CmdError> {
    app.exit(42);
    Ok(())
}

#[tauri::command]
pub async fn list_backups_cmd() -> Result<Vec<backup::BackupInfo>, CmdError> {
    backup::list_backups().map_err(CmdError::msg)
}

#[tauri::command]
pub async fn restore_backup_cmd(name: String) -> Result<(), CmdError> {
    backup::restore_backup(&name).map_err(CmdError::msg)
}

#[tauri::command]
pub async fn create_backup_cmd() -> Result<String, CmdError> {
    backup::create_backup().map_err(CmdError::msg)
}

#[tauri::command]
pub async fn list_settings_backups_cmd() -> Result<Vec<backup::AutosaveEntry>, CmdError> {
    backup::list_autosaves().map_err(CmdError::msg)
}

#[tauri::command]
pub async fn restore_settings_backup_cmd(id: String) -> Result<backup::AutosaveEntry, CmdError> {
    backup::restore_autosave(&id).map_err(CmdError::msg)
}

#[tauri::command]
pub async fn get_guardian_enabled() -> Result<bool, CmdError> {
    Ok(crate::guardian::get_enabled_from_config()?)
}

#[tauri::command]
pub async fn set_guardian_enabled(enabled: bool) -> Result<(), CmdError> {
    crate::guardian::set_enabled_in_config(enabled)?;
    Ok(())
}
