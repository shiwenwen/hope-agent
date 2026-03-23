use crate::paths;

/// Delete a file if it exists, logging the result.
fn remove_file_if_exists(path: &std::path::Path, label: &str) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| format!("Failed to delete {}: {}", label, e))?;
        app_info!("dev_tools", "clear", "Deleted {}: {}", label, path.display());
    }
    // Also remove WAL and SHM files for SQLite databases
    let wal = path.with_extension("db-wal");
    let shm = path.with_extension("db-shm");
    if wal.exists() {
        let _ = std::fs::remove_file(&wal);
    }
    if shm.exists() {
        let _ = std::fs::remove_file(&shm);
    }
    Ok(())
}

// ── Clear Sessions ──────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_sessions() -> Result<(), String> {
    // Delete sessions.db
    let db_path = crate::session::db_path().map_err(|e| e.to_string())?;
    remove_file_if_exists(&db_path, "sessions.db")?;

    // Delete attachments directory
    let root = paths::root_dir().map_err(|e| e.to_string())?;
    let attachments_dir = root.join("attachments");
    if attachments_dir.exists() {
        std::fs::remove_dir_all(&attachments_dir)
            .map_err(|e| format!("Failed to delete attachments: {}", e))?;
        app_info!("dev_tools", "clear", "Deleted attachments directory");
    }

    app_info!("dev_tools", "clear", "Sessions cleared successfully");
    Ok(())
}

// ── Clear Cron ──────────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_cron() -> Result<(), String> {
    let db_path = paths::cron_db_path().map_err(|e| e.to_string())?;
    remove_file_if_exists(&db_path, "cron.db")?;
    app_info!("dev_tools", "clear", "Cron jobs cleared successfully");
    Ok(())
}

// ── Clear Memory ────────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_memory() -> Result<(), String> {
    let db_path = paths::memory_db_path().map_err(|e| e.to_string())?;
    remove_file_if_exists(&db_path, "memory.db")?;
    app_info!("dev_tools", "clear", "Memory cleared successfully");
    Ok(())
}

// ── Reset Config ────────────────────────────────────────────────

#[tauri::command]
pub async fn dev_reset_config() -> Result<(), String> {
    let config_path = paths::config_path().map_err(|e| e.to_string())?;
    remove_file_if_exists(&config_path, "config.json")?;
    app_info!("dev_tools", "clear", "Config reset to defaults");
    Ok(())
}

// ── Clear All ───────────────────────────────────────────────────

#[tauri::command]
pub async fn dev_clear_all() -> Result<(), String> {
    dev_clear_sessions().await?;
    dev_clear_cron().await?;
    dev_clear_memory().await?;
    dev_reset_config().await?;
    app_info!("dev_tools", "clear", "All data cleared successfully");
    Ok(())
}
