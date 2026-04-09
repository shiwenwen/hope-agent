use std::path::{Path, PathBuf};

use crate::paths;

const MAX_BACKUPS: usize = 5;

/// Create a backup of all config files to ~/.opencomputer/backups/backup_{timestamp}/
/// Returns the backup directory path on success.
pub fn create_backup() -> Result<String, String> {
    let root = paths::root_dir().map_err(|e| format!("Cannot resolve root dir: {}", e))?;
    let backups_dir =
        paths::backups_dir().map_err(|e| format!("Cannot resolve backups dir: {}", e))?;

    // Create backups directory if it doesn't exist
    std::fs::create_dir_all(&backups_dir)
        .map_err(|e| format!("Cannot create backups dir: {}", e))?;

    // Generate timestamped backup directory name
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let backup_dir = backups_dir.join(format!("backup_{}", timestamp));
    std::fs::create_dir_all(&backup_dir).map_err(|e| format!("Cannot create backup dir: {}", e))?;

    // Backup individual files
    let files_to_backup = ["config.json", "user.json"];
    for file in &files_to_backup {
        let src = root.join(file);
        if src.exists() {
            let dst = backup_dir.join(file);
            if let Err(e) = std::fs::copy(&src, &dst) {
                eprintln!("[Backup] Warning: failed to copy {}: {}", file, e);
            }
        }
    }

    // Backup credentials/auth.json
    let cred_src = root.join("credentials").join("auth.json");
    if cred_src.exists() {
        let cred_dst_dir = backup_dir.join("credentials");
        let _ = std::fs::create_dir_all(&cred_dst_dir);
        if let Err(e) = std::fs::copy(&cred_src, cred_dst_dir.join("auth.json")) {
            eprintln!(
                "[Backup] Warning: failed to copy credentials/auth.json: {}",
                e
            );
        }
    }

    // Backup agents/ directory (recursive copy)
    let agents_src = root.join("agents");
    if agents_src.exists() && agents_src.is_dir() {
        let agents_dst = backup_dir.join("agents");
        if let Err(e) = copy_dir_recursive(&agents_src, &agents_dst) {
            eprintln!("[Backup] Warning: failed to copy agents/: {}", e);
        }
    }

    // Rotate old backups
    if let Err(e) = rotate_backups_internal(&backups_dir, MAX_BACKUPS) {
        eprintln!("[Backup] Warning: failed to rotate backups: {}", e);
    }

    Ok(backup_dir.to_string_lossy().to_string())
}

/// List available backups sorted by name (newest first)
pub fn list_backups() -> Result<Vec<BackupInfo>, String> {
    let backups_dir = paths::backups_dir().map_err(|e| e.to_string())?;
    if !backups_dir.exists() {
        return Ok(Vec::new());
    }

    let mut backups: Vec<BackupInfo> = std::fs::read_dir(&backups_dir)
        .map_err(|e| format!("Cannot read backups dir: {}", e))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("backup_") && entry.path().is_dir() {
                let metadata = entry.metadata().ok()?;
                let created = metadata
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                Some(BackupInfo {
                    name,
                    path: entry.path().to_string_lossy().to_string(),
                    created_at: created,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by name descending (newest first since names are timestamp-based)
    backups.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(backups)
}

/// Restore from a specific backup by copying files back to root
pub fn restore_backup(backup_name: &str) -> Result<(), String> {
    let backups_dir = paths::backups_dir().map_err(|e| e.to_string())?;
    let root = paths::root_dir().map_err(|e| e.to_string())?;
    let backup_dir = backups_dir.join(backup_name);

    if !backup_dir.exists() {
        return Err(format!("Backup '{}' not found", backup_name));
    }

    // Restore individual files
    let files = ["config.json", "user.json"];
    for file in &files {
        let src = backup_dir.join(file);
        if src.exists() {
            let dst = root.join(file);
            std::fs::copy(&src, &dst).map_err(|e| format!("Failed to restore {}: {}", file, e))?;
        }
    }

    // Restore credentials/auth.json
    let cred_src = backup_dir.join("credentials").join("auth.json");
    if cred_src.exists() {
        let cred_dst = root.join("credentials").join("auth.json");
        std::fs::copy(&cred_src, &cred_dst)
            .map_err(|e| format!("Failed to restore credentials/auth.json: {}", e))?;
    }

    // Restore agents/ directory
    let agents_src = backup_dir.join("agents");
    if agents_src.exists() && agents_src.is_dir() {
        let agents_dst = root.join("agents");
        // Remove existing agents dir and replace
        if agents_dst.exists() {
            let _ = std::fs::remove_dir_all(&agents_dst);
        }
        copy_dir_recursive(&agents_src, &agents_dst)
            .map_err(|e| format!("Failed to restore agents/: {}", e))?;
    }

    // `config.json` was rewritten out-of-band above; drop the in-memory
    // snapshot so hot-path readers pick up the restored state.
    let _ = crate::provider::reload_cache_from_disk();

    Ok(())
}

// ── Internal Helpers ───────────────────────────────────────────────

fn rotate_backups_internal(backups_dir: &Path, keep: usize) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(backups_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("backup_") && entry.path().is_dir() {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();

    // Sort by name ascending (oldest first)
    entries.sort();

    // Remove oldest entries if we exceed the limit
    if entries.len() > keep {
        let to_remove = entries.len() - keep;
        for path in entries.iter().take(to_remove) {
            if let Err(e) = std::fs::remove_dir_all(path) {
                eprintln!(
                    "[Backup] Warning: failed to remove old backup {:?}: {}",
                    path, e
                );
            }
        }
    }

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("Cannot create dir {:?}: {}", dst, e))?;

    for entry in std::fs::read_dir(src).map_err(|e| format!("Cannot read dir {:?}: {}", src, e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Cannot copy {:?}: {}", src_path, e))?;
        }
    }
    Ok(())
}

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub path: String,
    pub created_at: u64,
}
