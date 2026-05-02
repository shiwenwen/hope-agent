use anyhow::Result;

use super::store::store;
use super::types::PlanVersionInfo;

// ── Plan File I/O ───────────────────────────────────────────────
// Plans are stored under per-session subdirectories so a model `ls`-ing the
// plans tree only sees its own work — fixes the cross-session bleed that hit
// the snake-game session reading another session's leftover plan files.
//
// Layout: ~/.hope-agent/plans/<agent_id>/<session_id>/plan-{timestamp}.md
//                                                     plan-{timestamp}-v{N}.md
//
// `migration::migrate_flat_plans_to_subdirs` runs once at startup to move
// any legacy flat-layout files (`plan-{short_id}-...md`) into the right
// subdir based on a SessionDB lookup of the short_id prefix.

pub(crate) fn plans_dir() -> Result<std::path::PathBuf> {
    crate::paths::plans_dir()
}

/// Resolve the per-session plan directory by looking up the session's
/// agent_id in SessionDB. Falls back to a `_unknown_agent` bucket when the
/// session isn't in DB yet (rare — happens during very-first-message session
/// auto-create races) so writes never fail outright.
pub(crate) fn session_plans_dir_for(session_id: &str) -> Result<std::path::PathBuf> {
    let agent_id = crate::get_session_db()
        .and_then(|db| db.get_session(session_id).ok().flatten())
        .map(|meta| meta.agent_id)
        .unwrap_or_else(|| "_unknown_agent".to_string());
    crate::paths::session_plans_dir(&agent_id, session_id)
}

pub fn find_plan_file(session_id: &str) -> Result<Option<std::path::PathBuf>> {
    let store_ref = store();
    if let Ok(map) = store_ref.try_read() {
        if let Some(meta) = map.get(session_id) {
            if !meta.file_path.is_empty() {
                let path = std::path::PathBuf::from(&meta.file_path);
                if path.exists() {
                    return Ok(Some(path));
                }
            }
        }
    }

    let dir = session_plans_dir_for(session_id)?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(None);
    };
    let mut latest: Option<(String, std::path::PathBuf)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.starts_with("plan-") || !name.ends_with(".md") {
            continue;
        }
        let stem = name.trim_end_matches(".md");
        if stem
            .rsplit_once("-v")
            .is_some_and(|(_, suffix)| suffix.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        if latest
            .as_ref()
            .is_none_or(|(latest_name, _)| name > latest_name.as_str())
        {
            latest = Some((name.to_string(), path));
        }
    }

    Ok(latest.map(|(_, path)| path))
}

/// Scan `dir` for backups of `plan_path` (files named `{stem}-v{N}.md`) and
/// return the largest `N` found, or 0 when no backups exist. Used to seed
/// the version counter after a restart so we don't clobber older versions.
fn max_disk_version(dir: &std::path::Path, plan_path: &std::path::Path) -> u32 {
    let stem = match plan_path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s.to_string(),
        None => return 0,
    };
    let prefix = format!("{}-v", stem);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut max_version: u32 = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(num) = rest.strip_suffix(".md") else {
            continue;
        };
        if let Ok(v) = num.parse::<u32>() {
            if v > max_version {
                max_version = v;
            }
        }
    }
    max_version
}

/// Build the plan file path for a session. Uses a mapping stored in PlanMeta.file_path.
/// If no existing path, generates a new one under the per-session subdir.
pub(crate) fn plan_file_path(session_id: &str) -> Result<std::path::PathBuf> {
    if let Some(path) = find_plan_file(session_id)? {
        return Ok(path);
    }

    // The agent + session subdirs already namespace the file, so the filename
    // itself only needs a unique-within-session timestamp. UTC + nanosecond
    // suffix guards against same-second concurrent saves within one session.
    let now = chrono::Utc::now();
    let filename = format!(
        "plan-{}-{:09}.md",
        now.format("%Y%m%dT%H%M%SZ"),
        now.timestamp_subsec_nanos()
    );
    Ok(session_plans_dir_for(session_id)?.join(filename))
}

pub fn save_plan_file(session_id: &str, content: &str) -> Result<String> {
    let dir = session_plans_dir_for(session_id)?;
    std::fs::create_dir_all(&dir)?;
    let path = plan_file_path(session_id)?;

    // Version management: backup old version before overwriting
    if path.exists() {
        let mem_version = {
            let store_ref = store();
            if let Ok(map) = store_ref.try_read() {
                map.get(session_id).map(|m| m.version).unwrap_or(1)
            } else {
                1
            }
        };
        // On restart the in-memory counter resets to 1, which would overwrite
        // existing `plan-xxx-v1.md` backups. Scan the directory for existing
        // `-v{N}.md` siblings and bump past the highest one so new backups
        // land on a fresh slot.
        let current_version = mem_version.max(max_disk_version(&dir, &path) + 1);
        // Copy current file to versioned backup: plan-xxx-v{N}.md
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let backup_name = format!("{}-v{}.md", stem, current_version);
        let backup_path = dir.join(&backup_name);
        if let Err(e) = std::fs::copy(&path, &backup_path) {
            app_warn!(
                "plan",
                "version",
                "Failed to backup plan version {}: {}",
                current_version,
                e
            );
        }
        // Increment version counter in memory
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let store_ref = store();
                let mut map = store_ref.write().await;
                if let Some(meta) = map.get_mut(session_id) {
                    meta.version += 1;
                }
            });
        });
    }

    std::fs::write(&path, content)?;
    let path_str = path.to_string_lossy().to_string();
    // Update file_path in memory
    tokio::task::block_in_place(|| {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async {
            let mut map = store().write().await;
            if let Some(meta) = map.get_mut(session_id) {
                meta.file_path = path_str.clone();
            }
        });
    });
    Ok(path_str)
}

pub fn load_plan_file(session_id: &str) -> Result<Option<String>> {
    let path = plan_file_path(session_id)?;
    if path.exists() {
        return Ok(Some(std::fs::read_to_string(path)?));
    }
    Ok(None)
}

#[allow(dead_code)]
pub fn delete_plan_file(session_id: &str) -> Result<()> {
    let path = plan_file_path(session_id)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// List available versions of a plan (including the current and all backups).
pub fn list_plan_versions(session_id: &str) -> Result<Vec<PlanVersionInfo>> {
    let dir = session_plans_dir_for(session_id)?;
    let path = plan_file_path(session_id)?;
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut versions = Vec::new();

    // Current version
    if path.exists() {
        let meta = std::fs::metadata(&path)?;
        let modified = meta
            .modified()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_default();
        let current_version = {
            let store_ref = store();
            if let Ok(map) = store_ref.try_read() {
                map.get(session_id).map(|m| m.version).unwrap_or(1)
            } else {
                1
            }
        };
        versions.push(PlanVersionInfo {
            version: current_version,
            file_path: path.to_string_lossy().to_string(),
            modified_at: modified,
            is_current: true,
        });
    }

    // Backup versions
    if dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Match pattern: {stem}-v{N}.md
                if name.starts_with(&format!("{}-v", stem)) && name.ends_with(".md") {
                    let version_str = name
                        .trim_start_matches(&format!("{}-v", stem))
                        .trim_end_matches(".md");
                    if let Ok(v) = version_str.parse::<u32>() {
                        let meta = std::fs::metadata(entry.path()).ok();
                        let modified = meta
                            .and_then(|m| m.modified().ok())
                            .map(|t| {
                                let dt: chrono::DateTime<chrono::Local> = t.into();
                                dt.to_rfc3339()
                            })
                            .unwrap_or_default();
                        versions.push(PlanVersionInfo {
                            version: v,
                            file_path: entry.path().to_string_lossy().to_string(),
                            modified_at: modified,
                            is_current: false,
                        });
                    }
                }
            }
        }
    }

    // Sort by version descending (current first)
    versions.sort_by_key(|v| std::cmp::Reverse(v.version));
    Ok(versions)
}

/// Load content of a specific plan version.
pub fn load_plan_version(file_path: &str) -> Result<String> {
    Ok(std::fs::read_to_string(file_path)?)
}

/// One-time migration: move legacy flat-layout plan files
/// (`<plans>/plan-{short_id}-...md`) into the new per-session subdir
/// (`<plans>/<agent>/<session>/plan-{short_id}-...md`). Idempotent — already-
/// nested files are left alone, files with ambiguous / unknown short_id are
/// skipped with a warn so a human can inspect them.
///
/// Filenames are kept verbatim (including the now-redundant short_id segment)
/// to preserve any existing PlanMeta.file_path references that survived from
/// before the migration ran. New plans written post-migration use the simpler
/// `plan-{ts}-{nano}.md` form (see `plan_file_path`).
pub fn migrate_flat_plans_to_subdirs() {
    let plans = match plans_dir() {
        Ok(d) if d.exists() => d,
        _ => return,
    };
    let Ok(entries) = std::fs::read_dir(&plans) else {
        return;
    };
    let Some(db) = crate::get_session_db() else {
        return;
    };

    let (mut moved, mut skipped) = (0u32, 0u32);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".md") {
            continue;
        }
        let Some(rest) = name.strip_prefix("plan-") else {
            continue;
        };
        let Some((short_id, _)) = rest.split_once('-') else {
            continue;
        };
        if short_id.len() != 8 || !short_id.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }

        let matches = match db.find_sessions_by_id_prefix(short_id) {
            Ok(v) => v,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if matches.is_empty() {
            app_warn!(
                "plan",
                "migrate",
                "Skip {}: no session matches short_id {}",
                name,
                short_id
            );
            skipped += 1;
            continue;
        }
        if matches.len() > 1 {
            app_warn!(
                "plan",
                "migrate",
                "Skip {}: short_id {} ambiguous ({} matches)",
                name,
                short_id,
                matches.len()
            );
            skipped += 1;
            continue;
        }
        let (session_id, agent_id) = &matches[0];
        let target_dir = match crate::paths::session_plans_dir(agent_id, session_id) {
            Ok(d) => d,
            Err(e) => {
                app_warn!("plan", "migrate", "Resolve target dir failed: {}", e);
                continue;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            app_warn!(
                "plan",
                "migrate",
                "Create {} failed: {}",
                target_dir.display(),
                e
            );
            continue;
        }
        let target = target_dir.join(name);
        if let Err(e) = std::fs::rename(&path, &target) {
            app_warn!("plan", "migrate", "Move {} failed: {}", name, e);
        } else {
            moved += 1;
        }
    }

    if moved > 0 || skipped > 0 {
        app_info!(
            "plan",
            "migrate",
            "Plan files migration: {} moved to per-session subdirs, {} skipped",
            moved,
            skipped
        );
    }
}
