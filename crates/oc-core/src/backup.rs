use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::paths;

const MAX_BACKUPS: usize = 5;

/// How many automatic config snapshots to retain. Separate budget from the
/// manual `backup_*` snapshots so a flurry of settings edits can't evict the
/// last user-requested full backup.
const MAX_AUTOSAVES: usize = 50;

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
pub fn scope_save_reason(category: impl Into<String>, source: impl Into<String>) -> SaveReasonGuard {
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
            app_warn!(
                "backup",
                "autosave",
                "Cannot resolve autosave dir: {}",
                e
            );
            let _ = take_save_reason();
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app_warn!(
            "backup",
            "autosave",
            "Cannot create autosave dir: {}",
            e
        );
        let _ = take_save_reason();
        return;
    }
    let reason = take_save_reason();
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S-%3f").to_string();
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
        app_warn!(
            "backup",
            "autosave",
            "Rotation failed: {}",
            e
        );
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
                bus.emit(
                    "config:changed",
                    serde_json::json!({ "category": "user" }),
                );
            }
        }
        _ => {}
    }
    Ok(entry)
}
