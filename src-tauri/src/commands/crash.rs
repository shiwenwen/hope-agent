use crate::paths;
use crate::crash_journal;
use crate::backup;

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
pub async fn get_guardian_enabled() -> Result<bool, String> {
    let config_path = paths::config_path().map_err(|e| e.to_string())?;
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let config: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
    Ok(config
        .get("guardian")
        .and_then(|g| g.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true))
}

#[tauri::command]
pub async fn set_guardian_enabled(enabled: bool) -> Result<(), String> {
    let config_path = paths::config_path().map_err(|e| e.to_string())?;
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut config: serde_json::Value = serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
    config["guardian"] = serde_json::json!({ "enabled": enabled });
    let json_str = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&config_path, json_str).map_err(|e| format!("Failed to save config: {}", e))
}
