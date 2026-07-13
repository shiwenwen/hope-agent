use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::paths;

const MAX_BACKUPS: usize = 5;

/// How many automatic config snapshots to retain. Separate budget from the
/// manual `backup_*` snapshots so a flurry of settings edits can't evict the
/// last user-requested full backup.
const MAX_AUTOSAVES: usize = 50;

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
            if let Err(e) = std::fs::copy(&src, &dst) {
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
}

// ── Auto-Snapshot on every config write ────────────────────────────
//
// Every call to `config::save_config` and `user_config::save_user_config_to_disk`
// first copies the current on-disk file into `backups/autosave/` so that any
// settings change — whether triggered from the UI, from the `update_settings`
// tool, or from a CLI path — can be rolled back. Snapshot files are named
// `{timestamp}__{kind}__{category}__{source}.json` so metadata is embedded in
// the filename; no sidecar index is needed.

thread_local! {
    /// Optional reason label set by the caller (e.g. the settings tool) that
    /// describes why the next `save_config` / `save_user_config_to_disk` call
    /// is happening. Consumed — and reset — by the very next snapshot.
    static NEXT_SAVE_REASON: RefCell<Option<SaveReason>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone)]
struct SaveReason {
    /// Settings category being updated (e.g. "theme", "proxy", "user").
    category: String,
    /// Who triggered it: "skill", "ui", "cli", ...
    source: String,
}

/// RAII guard set by callers to label the next `save_*` snapshot.
/// Dropping it clears the label even if the save never happens, so a stale
/// label can't contaminate an unrelated subsequent write.
pub struct SaveReasonGuard {
    _private: (),
}

impl Drop for SaveReasonGuard {
    fn drop(&mut self) {
        NEXT_SAVE_REASON.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// Label the next config/user_config save so its autosave snapshot records
/// *why* the change happened. Returns a guard — hold it until after the save.
///
/// Example:
/// ```ignore
/// let _g = backup::scope_save_reason("theme", "skill");
/// config::save_config(&store)?; // snapshot tagged "theme/skill"
/// ```
pub fn scope_save_reason(
    category: impl Into<String>,
    source: impl Into<String>,
) -> SaveReasonGuard {
    NEXT_SAVE_REASON.with(|slot| {
        *slot.borrow_mut() = Some(SaveReason {
            category: category.into(),
            source: source.into(),
        });
    });
    SaveReasonGuard { _private: () }
}

fn take_save_reason() -> SaveReason {
    NEXT_SAVE_REASON
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(|| SaveReason {
            category: "unknown".into(),
            source: "unknown".into(),
        })
}

/// Snapshot `src` (if it exists) into `backups/autosave/` before it gets
/// overwritten. `kind` is "config" or "user". Errors are logged but never
/// bubbled up — a failed snapshot must not block a legitimate write.
pub fn snapshot_before_write(src: &Path, kind: &str) {
    if !src.exists() {
        // First-ever save — nothing to snapshot.
        // Still consume the reason so it doesn't leak to an unrelated save.
        let _ = take_save_reason();
        return;
    }
    let dir = match paths::autosave_dir() {
        Ok(d) => d,
        Err(e) => {
            app_warn!("backup", "autosave", "Cannot resolve autosave dir: {}", e);
            let _ = take_save_reason();
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app_warn!("backup", "autosave", "Cannot create autosave dir: {}", e);
        let _ = take_save_reason();
        return;
    }
    let reason = take_save_reason();
    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let safe_cat = sanitize_slug(&reason.category);
    let safe_src = sanitize_slug(&reason.source);
    let filename = format!("{}__{}__{}__{}.json", ts, kind, safe_cat, safe_src);
    let dst = dir.join(&filename);
    if let Err(e) = std::fs::copy(src, &dst) {
        app_warn!(
            "backup",
            "autosave",
            "Failed to snapshot {:?} → {:?}: {}",
            src,
            dst,
            e
        );
        return;
    }
    if let Err(e) = rotate_autosaves(&dir, MAX_AUTOSAVES) {
        app_warn!("backup", "autosave", "Rotation failed: {}", e);
    }
}

fn sanitize_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn rotate_autosaves(dir: &Path, keep: usize) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json") {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    // Names are timestamp-prefixed, so ascending sort = oldest first.
    entries.sort();
    if entries.len() > keep {
        let drop_count = entries.len() - keep;
        for p in entries.iter().take(drop_count) {
            if let Err(e) = std::fs::remove_file(p) {
                app_warn!(
                    "backup",
                    "autosave",
                    "Failed to drop old autosave {:?}: {}",
                    p,
                    e
                );
            }
        }
    }
    Ok(())
}

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
