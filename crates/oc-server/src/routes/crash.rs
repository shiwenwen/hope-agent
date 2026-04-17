use axum::Json;
use serde_json::{json, Value};

use oc_core::{backup, crash_journal, guardian, paths};

use crate::error::AppError;

/// `GET /api/crash/recovery-info`
pub async fn get_crash_recovery_info() -> Result<Json<Value>, AppError> {
    let recovered = std::env::var("OPENCOMPUTER_RECOVERED").is_ok();
    let crash_count: u32 = std::env::var("OPENCOMPUTER_CRASH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut info = json!({
        "recovered": recovered,
        "crashCount": crash_count,
    });

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

    Ok(Json(info))
}

/// `GET /api/crash/history`
pub async fn get_crash_history() -> Result<Json<Value>, AppError> {
    let path = paths::crash_journal_path()?;
    let journal = crash_journal::CrashJournal::load(&path);
    Ok(Json(serde_json::to_value(&journal)?))
}

/// `DELETE /api/crash/history`
pub async fn clear_crash_history() -> Result<Json<Value>, AppError> {
    let path = paths::crash_journal_path()?;
    let mut journal = crash_journal::CrashJournal::load(&path);
    journal.clear();
    journal
        .save(&path)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "cleared": true })))
}

/// `GET /api/crash/backups`
pub async fn list_backups() -> Result<Json<Vec<backup::BackupInfo>>, AppError> {
    Ok(Json(backup::list_backups().map_err(AppError::internal)?))
}

#[derive(serde::Deserialize)]
pub struct RestoreBody {
    pub name: String,
}

/// `POST /api/crash/backups/restore`
pub async fn restore_backup(Json(body): Json<RestoreBody>) -> Result<Json<Value>, AppError> {
    backup::restore_backup(&body.name).map_err(AppError::internal)?;
    Ok(Json(json!({ "restored": true })))
}

/// `POST /api/crash/backups`
pub async fn create_backup() -> Result<Json<Value>, AppError> {
    let name = backup::create_backup().map_err(AppError::internal)?;
    Ok(Json(json!({ "name": name })))
}

/// `GET /api/settings/backups` — list automatic config snapshots
pub async fn list_settings_backups() -> Result<Json<Vec<backup::AutosaveEntry>>, AppError> {
    Ok(Json(backup::list_autosaves().map_err(AppError::internal)?))
}

#[derive(serde::Deserialize)]
pub struct RestoreAutosaveBody {
    pub id: String,
}

/// `POST /api/settings/backups/restore` — roll back to an autosave entry
pub async fn restore_settings_backup(
    Json(body): Json<RestoreAutosaveBody>,
) -> Result<Json<backup::AutosaveEntry>, AppError> {
    let entry = backup::restore_autosave(&body.id).map_err(AppError::internal)?;
    Ok(Json(entry))
}

/// `GET /api/crash/guardian`
pub async fn get_guardian_enabled() -> Result<Json<Value>, AppError> {
    Ok(Json(
        json!({ "enabled": guardian::get_enabled_from_config()? }),
    ))
}

#[derive(serde::Deserialize)]
pub struct GuardianBody {
    pub enabled: bool,
}

/// `PUT /api/crash/guardian`
pub async fn set_guardian_enabled(Json(body): Json<GuardianBody>) -> Result<Json<Value>, AppError> {
    guardian::set_enabled_in_config(body.enabled)?;
    Ok(Json(json!({ "saved": true })))
}
