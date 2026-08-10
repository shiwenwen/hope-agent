use std::path::{Path, PathBuf};

use crate::paths;

const MAX_BACKUPS: usize = 5;

/// Create a backup of all config files to ~/.hope-agent/backups/backup_{timestamp}/
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
    let files_to_backup = ["config.json", "user.json", "memory.md"];
    for file in &files_to_backup {
        let src = root.join(file);
        if src.exists() {
            let dst = backup_dir.join(file);
            if let Err(e) = crate::platform::copy_secure_file_atomic(&src, &dst) {
                app_warn!("backup", "create", "Failed to copy {}: {}", file, e);
            }
        }
    }

    // Backup credentials/auth.json
    let cred_src = root.join("credentials").join("auth.json");
    if cred_src.exists() {
        let cred_dst_dir = backup_dir.join("credentials");
        let _ = std::fs::create_dir_all(&cred_dst_dir);
        if let Err(e) = std::fs::copy(&cred_src, cred_dst_dir.join("auth.json")) {
            app_warn!(
                "backup",
                "create",
                "Failed to copy credentials/auth.json: {}",
                e
            );
        }
    }

    // Backup agents/ directory (recursive copy)
    let agents_src = root.join("agents");
    if agents_src.exists() && agents_src.is_dir() {
        let agents_dst = backup_dir.join("agents");
        if let Err(e) = copy_dir_recursive(&agents_src, &agents_dst) {
            app_warn!("backup", "create", "Failed to copy agents/: {}", e);
        }
    }

    // Canonical Global Core Memory. Agent Core Memory is already included in
    // agents/, while Project Core Memory is copied selectively below so large
    // project workspaces never enter a configuration backup.
    let global_memory_src = root.join("memory");
    if global_memory_src.is_dir() {
        if let Err(e) = copy_dir_recursive(&global_memory_src, &backup_dir.join("memory")) {
            app_warn!("backup", "create", "Failed to copy memory/: {}", e);
        }
    }
    if let Err(e) =
        copy_project_memory_dirs(&root.join("projects"), &backup_dir.join("projects"), false)
    {
        app_warn!(
            "backup",
            "create",
            "Failed to copy project Core Memory: {}",
            e
        );
    }

    // Rotate old backups
    if let Err(e) = rotate_backups_internal(&backups_dir, MAX_BACKUPS) {
        app_warn!("backup", "rotate", "Failed to rotate backups: {}", e);
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
    let files = ["config.json", "user.json", "memory.md"];
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

    let global_memory_src = backup_dir.join("memory");
    if global_memory_src.is_dir() {
        let global_memory_dst = root.join("memory");
        replace_dir_from_backup(&global_memory_src, &global_memory_dst)
            .map_err(|e| format!("Failed to restore memory/: {}", e))?;
    }
    copy_project_memory_dirs(&backup_dir.join("projects"), &root.join("projects"), true)
        .map_err(|e| format!("Failed to restore project Core Memory: {}", e))?;

    // Agent/Global/Project Core files were replaced outside the repository.
    // Existing chats must not retain stale in-memory snapshots after an
    // explicit full restore.
    crate::memory::core_repository::invalidate_all_session_snapshots();
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "memory:core_changed",
            serde_json::json!({ "scopeType": "all", "action": "restore_backup" }),
        );
    }

    // `config.json` was rewritten out-of-band above; drop the in-memory
    // snapshot so hot-path readers pick up the restored state.
    let _ = crate::config::reload_cache_from_disk();

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
                app_warn!(
                    "backup",
                    "rotate",
                    "Failed to remove old backup {:?}: {}",
                    path,
                    e
                );
            }
        }
    }

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    validate_copy_source_dir(src)?;
    std::fs::create_dir_all(dst).map_err(|e| format!("Cannot create dir {:?}: {}", dst, e))?;

    for entry in std::fs::read_dir(src).map_err(|e| format!("Cannot read dir {:?}: {}", src, e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&src_path)
            .map_err(|e| format!("Cannot inspect {:?}: {}", src_path, e))?;
        if metadata.file_type().is_symlink() {
            app_warn!(
                "backup",
                "copy",
                "Skipping symlink while creating/restoring backup: {}",
                src_path.display()
            );
            continue;
        }
        if metadata.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if metadata.is_file() {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Cannot copy {:?}: {}", src_path, e))?;
        }
    }
    Ok(())
}

fn validate_copy_source_dir(src: &Path) -> Result<(), String> {
    let src_meta = std::fs::symlink_metadata(src)
        .map_err(|e| format!("Cannot inspect dir {:?}: {}", src, e))?;
    if src_meta.file_type().is_symlink() || !src_meta.is_dir() {
        return Err(format!(
            "Refusing to copy non-directory or symlink {:?}",
            src
        ));
    }
    Ok(())
}

/// Stage a complete directory beside the destination before replacing it.
/// A malformed/tampered backup or a mid-copy failure therefore cannot delete
/// the currently working Core Memory directory.
fn replace_dir_from_backup(src: &Path, dst: &Path) -> Result<(), String> {
    validate_copy_source_dir(src)?;
    let parent = dst
        .parent()
        .ok_or_else(|| format!("Destination has no parent: {:?}", dst))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create restore parent {:?}: {}", parent, e))?;
    if let Ok(metadata) = std::fs::symlink_metadata(dst) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("Refusing Core Memory destination {:?}", dst));
        }
    }
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let base = dst
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("memory");
    let staged = parent.join(format!(".{base}.restore-{suffix}"));
    let previous = parent.join(format!(".{base}.previous-{suffix}"));
    if let Err(error) = copy_dir_recursive(src, &staged) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(error);
    }
    let had_previous = dst.exists();
    if had_previous {
        std::fs::rename(dst, &previous)
            .map_err(|e| format!("Cannot stage current {:?}: {}", dst, e))?;
    }
    if let Err(error) = std::fs::rename(&staged, dst) {
        if had_previous {
            let _ = std::fs::rename(&previous, dst);
        }
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("Cannot install restored {:?}: {}", dst, error));
    }
    if had_previous {
        let _ = std::fs::remove_dir_all(previous);
    }
    Ok(())
}

/// Copy only `projects/{uuid}/memory/`, never project workspaces. During
/// restore, replace the backed-up scope directory atomically at directory
/// granularity while leaving projects absent from the backup untouched.
fn copy_project_memory_dirs(src_root: &Path, dst_root: &Path, replace: bool) -> Result<(), String> {
    if !src_root.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(src_root)
        .map_err(|e| format!("Cannot read projects dir {:?}: {}", src_root, e))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let project_id = entry.file_name().to_string_lossy().to_string();
        if uuid::Uuid::parse_str(&project_id).is_err() || !entry.path().is_dir() {
            continue;
        }
        let src_memory = entry.path().join("memory");
        if !src_memory.is_dir() {
            continue;
        }
        validate_copy_source_dir(&src_memory)?;
        let dst_project = dst_root.join(&project_id);
        if let Ok(metadata) = std::fs::symlink_metadata(&dst_project) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "Refusing project Core Memory destination {:?}",
                    dst_project
                ));
            }
        }
        let dst_memory = dst_project.join("memory");
        if replace {
            replace_dir_from_backup(&src_memory, &dst_memory)?;
        } else {
            copy_dir_recursive(&src_memory, &dst_memory)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_core_backup_copies_only_memory_and_restore_replaces_scope() {
        let temp = tempfile::tempdir().unwrap();
        let project_id = "00000000-0000-0000-0000-000000000001";
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        let source_project = source.join(project_id);
        std::fs::create_dir_all(source_project.join("memory/topics")).unwrap();
        std::fs::create_dir_all(source_project.join("workspace")).unwrap();
        std::fs::write(source_project.join("memory/MEMORY.md"), "core").unwrap();
        std::fs::write(source_project.join("memory/topics/one.md"), "topic").unwrap();
        std::fs::write(source_project.join("workspace/private.txt"), "workspace").unwrap();

        copy_project_memory_dirs(&source, &destination, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join(project_id).join("memory/MEMORY.md")).unwrap(),
            "core"
        );
        assert!(!destination
            .join(project_id)
            .join("workspace/private.txt")
            .exists());

        std::fs::write(
            destination.join(project_id).join("memory/topics/stale.md"),
            "stale",
        )
        .unwrap();
        copy_project_memory_dirs(&source, &destination, true).unwrap();
        assert!(!destination
            .join(project_id)
            .join("memory/topics/stale.md")
            .exists());
        assert!(destination
            .join(project_id)
            .join("memory/topics/one.md")
            .exists());
    }

    #[test]
    fn invalid_restore_source_preserves_current_core_memory() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("invalid-source");
        let destination = temp.path().join("memory");
        std::fs::write(&source, "not a directory").unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("MEMORY.md"), "current").unwrap();

        let error = replace_dir_from_backup(&source, &destination).unwrap_err();

        assert!(error.contains("non-directory or symlink"));
        assert_eq!(
            std::fs::read_to_string(destination.join("MEMORY.md")).unwrap(),
            "current"
        );
    }

    #[test]
    fn legacy_token_scrub_accepts_bom_and_preserves_utf8() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let body = r#"{"server":{"apiKey":"legacy-token"},"label":"服务器（测试）"}"#;
        let bytes = [b"\xef\xbb\xbf".as_slice(), body.as_bytes()].concat();
        std::fs::write(&path, bytes).unwrap();

        scrub_legacy_server_token_file(&path).unwrap();

        let rewritten = std::fs::read(&path).unwrap();
        assert!(!rewritten.starts_with(b"\xef\xbb\xbf"));
        let value: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(value["label"], "服务器（测试）");
        assert!(value["server"].get("apiKey").is_none());
    }

    #[test]
    fn unparseable_backup_is_quarantined_without_stopping_the_scan() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let autosave = paths::autosave_dir().unwrap();
            std::fs::create_dir_all(&autosave).unwrap();
            let malformed_path =
                autosave.join("2026-08-08T00-00-00-000__config__server__test.json");
            let malformed = b"{\"server\":{\"apiKey\":\"legacy-token\"},\"label\":\xe4";
            std::fs::write(&malformed_path, malformed).unwrap();

            let valid_path = autosave.join("2026-08-08T00-00-01-000__config__server__test.json");
            std::fs::write(
                &valid_path,
                br#"{"server":{"apiKey":"second-token"},"theme":"dark"}"#,
            )
            .unwrap();

            scrub_legacy_server_tokens().unwrap();

            assert!(!malformed_path.exists());
            let quarantine = paths::credentials_dir().unwrap().join("quarantine");
            let quarantined = std::fs::read_dir(&quarantine)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            assert_eq!(quarantined.len(), 1);
            assert_eq!(std::fs::read(&quarantined[0]).unwrap(), malformed);
            assert_eq!(
                quarantined[0].extension().and_then(|ext| ext.to_str()),
                Some("corrupt")
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let quarantine_mode =
                    std::fs::metadata(&quarantine).unwrap().permissions().mode() & 0o777;
                assert_eq!(quarantine_mode, 0o700);
                let mode = std::fs::metadata(&quarantined[0])
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600);
            }

            let valid: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&valid_path).unwrap()).unwrap();
            assert_eq!(valid["theme"], "dark");
            assert!(valid["server"].get("apiKey").is_none());
        });
    }

    #[test]
    fn quarantine_failure_is_fail_closed_and_preserves_the_backup() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let autosave = paths::autosave_dir().unwrap();
            std::fs::create_dir_all(&autosave).unwrap();
            let malformed_path =
                autosave.join("2026-08-08T00-00-00-000__config__server__test.json");
            let malformed = b"{\"server\":{\"apiKey\":\"legacy-token\"}";
            std::fs::write(&malformed_path, malformed).unwrap();

            let credentials = paths::credentials_dir().unwrap();
            std::fs::create_dir_all(&credentials).unwrap();
            std::fs::write(credentials.join("quarantine"), b"not a directory").unwrap();

            let error = scrub_legacy_server_tokens().unwrap_err();
            assert!(error.contains("Cannot quarantine unparseable config backup"));
            assert_eq!(std::fs::read(&malformed_path).unwrap(), malformed);
        });
    }

    #[test]
    fn quarantine_detects_a_concurrent_replacement_and_restores_it() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let autosave = paths::autosave_dir().unwrap();
            std::fs::create_dir_all(&autosave).unwrap();
            let path = autosave.join("2026-08-08T00-00-00-000__config__server__test.json");
            let stale_bytes = b"{\"server\":{\"apiKey\":\"stale-token\"}";
            let replacement = br#"{"theme":"dark"}"#;
            std::fs::write(&path, replacement).unwrap();

            let error = quarantine_unparseable_legacy_token_backup(&path, stale_bytes)
                .expect_err("a changed source must not be quarantined as the stale parse result");

            assert!(error.contains("changed while it was being quarantined"));
            assert_eq!(std::fs::read(&path).unwrap(), replacement);
            let quarantine = paths::credentials_dir().unwrap().join("quarantine");
            assert_eq!(std::fs::read_dir(quarantine).unwrap().count(), 0);
        });
    }
}

// ── Auto-Snapshot on every config write ────────────────────────────
//
// autosave 原语已迁 `config::autosave`（persistence **直调**，写前快照必须
// 无条件执行——不能挂在只有 init_runtime 才注册的钩子上，`server setup` 等
// 入口在 init_runtime 之前/之外写 config）。此处再导出保持
// `crate::backup::scope_save_reason` 等既有调用路径不变；本文件只剩
// 完整备份 / 恢复 / autosave 列表与回滚。
pub use crate::config::autosave::{scope_save_reason, snapshot_before_write, SaveReasonGuard};

/// A single automatic snapshot entry, parsed from the filename.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutosaveEntry {
    /// Stable ID — the full filename (without extension). Use this with
    /// [`restore_autosave`].
    pub id: String,
    /// ISO-8601 timestamp captured at snapshot time.
    pub timestamp: String,
    /// "config" (→ config.json) or "user" (→ user.json).
    pub kind: String,
    /// Settings category that was being updated, or "unknown".
    pub category: String,
    /// Who triggered the save: "skill", "ui", "cli", or "unknown".
    pub source: String,
}

/// List automatic config snapshots, newest first.
pub fn list_autosaves() -> Result<Vec<AutosaveEntry>, String> {
    let dir = paths::autosave_dir().map_err(|e| e.to_string())?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<AutosaveEntry> = std::fs::read_dir(&dir)
        .map_err(|e| format!("Cannot read autosave dir: {}", e))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".json")?;
            let parts: Vec<&str> = stem.splitn(4, "__").collect();
            if parts.len() != 4 {
                return None;
            }
            Some(AutosaveEntry {
                id: stem.to_string(),
                timestamp: parts[0].to_string(),
                kind: parts[1].to_string(),
                category: parts[2].to_string(),
                source: parts[3].to_string(),
            })
        })
        .collect();
    // Newest first: timestamp is a lexicographically sortable prefix.
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(entries)
}

/// Restore a single automatic snapshot identified by its `id` (the filename
/// stem returned by [`list_autosaves`]). Creates a fresh snapshot of the
/// current state before overwriting, so the restore itself is reversible.
///
/// Emits the `config:changed` EventBus event so the frontend refreshes.
pub fn restore_autosave(id: &str) -> Result<AutosaveEntry, String> {
    let dir = paths::autosave_dir().map_err(|e| e.to_string())?;
    let src = dir.join(format!("{}.json", id));
    if !src.exists() {
        return Err(format!("Autosave '{}' not found", id));
    }
    let stem_parts: Vec<&str> = id.splitn(4, "__").collect();
    if stem_parts.len() != 4 {
        return Err(format!("Invalid autosave id: '{}'", id));
    }
    let entry = AutosaveEntry {
        id: id.to_string(),
        timestamp: stem_parts[0].to_string(),
        kind: stem_parts[1].to_string(),
        category: stem_parts[2].to_string(),
        source: stem_parts[3].to_string(),
    };

    // Pick destination path by kind.
    let dst = match entry.kind.as_str() {
        "config" => paths::config_path().map_err(|e| e.to_string())?,
        "user" => paths::user_config_path().map_err(|e| e.to_string())?,
        other => return Err(format!("Unknown snapshot kind: '{}'", other)),
    };

    // Snapshot current state first so the rollback is itself reversible.
    {
        let _g = scope_save_reason(format!("rollback-to:{}", entry.timestamp), "rollback");
        snapshot_before_write(&dst, &entry.kind);
    }

    // Overwrite in place.
    std::fs::copy(&src, &dst)
        .map_err(|e| format!("Failed to copy {:?} → {:?}: {}", src, dst, e))?;

    // Refresh in-memory caches and notify frontend.
    match entry.kind.as_str() {
        "config" => {
            let _ = crate::config::reload_cache_from_disk();
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "config:changed",
                    serde_json::json!({ "category": "__rollback__" }),
                );
            }
        }
        "user" => {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit("config:changed", serde_json::json!({ "category": "user" }));
            }
        }
        _ => {}
    }
    Ok(entry)
}

/// Remove the legacy server Owner Token from every config snapshot created
/// before the credential-store migration. These files are ordinary rollback
/// data (and may be copied off-host), so leaving `server.apiKey` in them would
/// defeat moving the live value to `credentials/server-auth.json`.
///
/// Symlinks are rejected rather than followed: the backup tree is data, not an
/// authority to rewrite arbitrary files outside `~/.hope-agent/backups`.
pub fn scrub_legacy_server_tokens() -> Result<(), String> {
    let backups = paths::backups_dir().map_err(|error| error.to_string())?;
    if !backups.exists() {
        return Ok(());
    }
    let backups_metadata = std::fs::symlink_metadata(&backups)
        .map_err(|error| format!("Cannot inspect backups dir: {error}"))?;
    if backups_metadata.file_type().is_symlink() || !backups_metadata.is_dir() {
        return Err(format!(
            "Refusing to inspect non-directory or symlink {:?}",
            backups
        ));
    }

    let autosave = backups.join("autosave");
    if autosave.exists() {
        let metadata = std::fs::symlink_metadata(&autosave)
            .map_err(|error| format!("Cannot inspect autosave dir: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Refusing to inspect non-directory or symlink {:?}",
                autosave
            ));
        }
        for entry in std::fs::read_dir(&autosave)
            .map_err(|error| format!("Cannot read autosave dir: {error}"))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().split("__").nth(1) == Some("config")
                && path.extension().and_then(|ext| ext.to_str()) == Some("json")
            {
                scrub_legacy_server_token_file(&path)?;
            }
        }
    }

    for entry in
        std::fs::read_dir(&backups).map_err(|error| format!("Cannot read backups dir: {error}"))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("backup_") {
            let metadata = std::fs::symlink_metadata(entry.path())
                .map_err(|error| format!("Cannot inspect {:?}: {error}", entry.path()))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "Refusing to inspect non-directory or symlink {:?}",
                    entry.path()
                ));
            }
            let config = entry.path().join("config.json");
            if config.exists() {
                scrub_legacy_server_token_file(&config)?;
            }
        }
    }
    Ok(())
}

fn scrub_legacy_server_token_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect {:?}: {error}", path))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Refusing to rewrite non-file or symlink {:?}",
            path
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("Cannot read {:?}: {error}", path))?;
    // Match the live config parser: Windows editors commonly prefix JSON with
    // EF BB BF, which is valid UTF-8 text but rejected by serde_json itself.
    let json = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let mut value: serde_json::Value = match serde_json::from_slice(json) {
        Ok(value) => value,
        Err(parse_error) => {
            // The file may still contain a plaintext legacy Owner Token. Move
            // it out of the portable backup tree into the credential area,
            // but never let one unusable rollback snapshot brick startup.
            let quarantined = quarantine_unparseable_legacy_token_backup(path, &bytes).map_err(
                |quarantine_error| {
                    format!(
                        "Cannot quarantine unparseable config backup {:?} (parse: {}; quarantine: {})",
                        path, parse_error, quarantine_error
                    )
                },
            )?;
            app_warn!(
                "backup",
                "scrub_legacy_server_token",
                "Quarantined unparseable config backup {} as {}: {}",
                path.display(),
                quarantined.display(),
                parse_error
            );
            return Ok(());
        }
    };
    let Some(server) = value
        .as_object_mut()
        .and_then(|root| root.get_mut("server"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(());
    };
    if server.remove("apiKey").is_none() {
        return Ok(());
    }
    let data = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("Cannot serialize {:?}: {error}", path))?;
    // Config snapshots can contain other sensitive settings even after the
    // legacy Owner Token is removed. Preserve the credential-grade 0600
    // posture while replacing the file atomically.
    crate::platform::write_secure_file(path, &data)
        .map_err(|error| format!("Cannot rewrite {:?}: {error}", path))
}

/// Preserve an unparseable backup under the credential boundary before
/// removing it from the portable backup tree. The opaque random filename
/// avoids exposing a credential-derived content fingerprint in logs.
fn quarantine_unparseable_legacy_token_backup(
    path: &Path,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let quarantine_dir = paths::credentials_dir()
        .map_err(|error| error.to_string())?
        .join("quarantine");
    std::fs::create_dir_all(&quarantine_dir)
        .map_err(|error| format!("Cannot create {:?}: {error}", quarantine_dir))?;
    let quarantine_metadata = std::fs::symlink_metadata(&quarantine_dir)
        .map_err(|error| format!("Cannot inspect {:?}: {error}", quarantine_dir))?;
    if quarantine_metadata.file_type().is_symlink() || !quarantine_metadata.is_dir() {
        return Err(format!(
            "Refusing credential quarantine directory {:?}",
            quarantine_dir
        ));
    }
    // A moved file keeps its original mode. Tighten the directory before the
    // rename so even a crash before the file-level 0600 rewrite cannot expose
    // a historical 0644 Owner Token to another local user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&quarantine_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Cannot secure {:?}: {error}", quarantine_dir))?;
    }

    let quarantine_id = uuid::Uuid::new_v4().simple();
    let quarantined =
        quarantine_dir.join(format!("legacy-config-backup-{quarantine_id}.json.corrupt"));
    // Claim the exact directory entry in one operation. Copying first and
    // deleting `path` later could unlink a replacement published by another
    // Hope Agent process between those two operations.
    std::fs::rename(path, &quarantined)
        .map_err(|error| format!("Cannot move {:?} to {:?}: {error}", path, quarantined))?;
    let claimed_bytes = match std::fs::read(&quarantined) {
        Ok(bytes) => bytes,
        Err(error) => {
            let rollback = restore_quarantined_backup(&quarantined, path);
            return Err(format_quarantine_rollback_error(
                format!("Cannot verify {:?}: {error}", quarantined),
                rollback,
                &quarantined,
                path,
            ));
        }
    };
    if claimed_bytes != bytes {
        let rollback = restore_quarantined_backup(&quarantined, path);
        return Err(format_quarantine_rollback_error(
            format!("Backup {:?} changed while it was being quarantined", path),
            rollback,
            &quarantined,
            path,
        ));
    }
    if let Err(error) = crate::platform::write_secure_file(&quarantined, &claimed_bytes) {
        let rollback = restore_quarantined_backup(&quarantined, path);
        return Err(match rollback {
            Ok(()) => format!("Cannot secure {:?}: {error}; move rolled back", quarantined),
            Err(rollback_error) => format!(
                "Cannot secure {:?}: {error}; cannot roll back to {:?}: {rollback_error}",
                quarantined, path
            ),
        });
    }
    Ok(quarantined)
}

/// Restore a quarantined file without overwriting a concurrently published
/// replacement. A hard link claims the absent original name atomically; only
/// then is the quarantine name removed.
fn restore_quarantined_backup(quarantined: &Path, original: &Path) -> std::io::Result<()> {
    std::fs::hard_link(quarantined, original)?;
    std::fs::remove_file(quarantined)
}

fn format_quarantine_rollback_error(
    error: String,
    rollback: std::io::Result<()>,
    quarantined: &Path,
    original: &Path,
) -> String {
    match rollback {
        Ok(()) => format!("{error}; move rolled back"),
        Err(rollback_error) => format!(
            "{error}; cannot roll back {:?} to {:?}: {rollback_error}",
            quarantined, original
        ),
    }
}
