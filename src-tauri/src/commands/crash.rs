use crate::backup;
use crate::crash_journal;
use crate::paths;

#[tauri::command]
pub async fn get_crash_recovery_info() -> Result<serde_json::Value, String> {
    let recovered = std::env::var("OPENCOMPUTER_RECOVERED").is_ok();
    let crash_count: u32 = std::env::var("OPENCOMPUTER_CRASH_COUNT")
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
pub async fn get_crash_history() -> Result<serde_json::Value, String> {
    let path = paths::crash_journal_path().map_err(|e| e.to_string())?;
    let journal = crash_journal::CrashJournal::load(&path);
    serde_json::to_value(&journal).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_crash_history() -> Result<(), String> {
    let path = paths::crash_journal_path().map_err(|e| e.to_string())?;
    let mut journal = crash_journal::CrashJournal::load(&path);
    journal.clear();
    journal.save(&path)
}

#[tauri::command]
pub async fn request_app_restart(app: tauri::AppHandle) -> Result<(), String> {
    app.exit(42);
    Ok(())
}

#[tauri::command]
pub async fn list_backups_cmd() -> Result<Vec<backup::BackupInfo>, String> {
    backup::list_backups()
}

#[tauri::command]
pub async fn restore_backup_cmd(name: String) -> Result<(), String> {
    backup::restore_backup(&name)
}

#[tauri::command]
pub async fn create_backup_cmd() -> Result<String, String> {
    backup::create_backup()
}

#[tauri::command]
pub async fn list_settings_backups_cmd() -> Result<Vec<backup::AutosaveEntry>, String> {
    backup::list_autosaves()
}

#[tauri::command]
pub async fn restore_settings_backup_cmd(id: String) -> Result<backup::AutosaveEntry, String> {
    backup::restore_autosave(&id)
}

#[tauri::command]
pub async fn get_guardian_enabled() -> Result<bool, String> {
    crate::guardian::get_enabled_from_config().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_guardian_enabled(enabled: bool) -> Result<(), String> {
    crate::guardian::set_enabled_in_config(enabled).map_err(|e| e.to_string())
}
