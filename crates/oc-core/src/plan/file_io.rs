use anyhow::Result;

use super::store::store;
use super::types::{PlanStep, PlanStepStatus, PlanVersionInfo};

// ── Plan File I/O ───────────────────────────────────────────────
// Plans are stored in the workspace plan/ directory with readable names:
//   ~/.opencomputer/plans/plan-{short_id}-{timestamp}.md
//   ~/.opencomputer/plans/result-{short_id}-{timestamp}.md

pub(crate) fn plans_dir() -> Result<std::path::PathBuf> {
    crate::paths::plans_dir()
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
/// If no existing path, generates a new one with readable name.
pub(crate) fn plan_file_path(session_id: &str) -> Result<std::path::PathBuf> {
    // We need direct access to the OnceLock store to avoid circular dependency with store()
    // since store() calls PLAN_STORE.get_or_init which is the same thing.
    let store_ref = store();
    if let Ok(map) = store_ref.try_read() {
        if let Some(meta) = map.get(session_id) {
            if !meta.file_path.is_empty() {
                let p = std::path::PathBuf::from(&meta.file_path);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }
    // Generate new path: plan-{short_id}-{date}-{nano}.md
    // UTC + nanosecond suffix avoids same-second collisions across concurrent
    // sessions and stays stable across timezone changes.
    let short_id = crate::truncate_utf8(session_id, 8);
    let now = chrono::Utc::now();
    let filename = format!(
        "plan-{}-{}-{:09}.md",
        short_id,
        now.format("%Y%m%dT%H%M%SZ"),
        now.timestamp_subsec_nanos()
    );
    Ok(plans_dir()?.join(filename))
}

/// Build the result file path for a session.
fn result_file_path(session_id: &str) -> Result<std::path::PathBuf> {
    let short_id = crate::truncate_utf8(session_id, 8);
    let now = chrono::Utc::now();
    let filename = format!(
        "result-{}-{}-{:09}.md",
        short_id,
        now.format("%Y%m%dT%H%M%SZ"),
        now.timestamp_subsec_nanos()
    );
    Ok(plans_dir()?.join(filename))
}

pub fn save_plan_file(session_id: &str, content: &str) -> Result<String> {
    let dir = plans_dir()?;
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

/// Save execution result as a separate markdown file.
pub fn save_result_file(
    session_id: &str,
    plan_title: &str,
    steps: &[PlanStep],
    summary: &str,
) -> Result<String> {
    let dir = plans_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = result_file_path(session_id)?;

    let mut md = String::new();
    md.push_str(&format!("# 执行结果: {}\n\n", plan_title));
    md.push_str(&format!(
        "> 执行时间: {}\n\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    // Step results
    md.push_str("## 步骤执行情况\n\n");
    let mut current_phase = String::new();
    for step in steps {
        if step.phase != current_phase {
            current_phase = step.phase.clone();
            md.push_str(&format!("### {}\n\n", current_phase));
        }
        let icon = match step.status {
            PlanStepStatus::Completed => "✅",
            PlanStepStatus::Failed => "❌",
            PlanStepStatus::Skipped => "⏭️",
            PlanStepStatus::InProgress => "🔄",
            PlanStepStatus::Pending => "⭕",
        };
        let duration = step
            .duration_ms
            .map(|ms| format!(" ({}ms)", ms))
            .unwrap_or_default();
        md.push_str(&format!("- {} {}{}\n", icon, step.title, duration));
    }

    let completed = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Completed)
        .count();
    let failed = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Failed)
        .count();
    let skipped = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Skipped)
        .count();
    md.push_str(&format!(
        "\n## 统计\n\n- 完成: {}\n- 失败: {}\n- 跳过: {}\n- 总计: {}\n",
        completed,
        failed,
        skipped,
        steps.len()
    ));

    if !summary.is_empty() {
        md.push_str(&format!("\n## 总结\n\n{}\n", summary));
    }

    std::fs::write(&path, &md)?;
    Ok(path.to_string_lossy().to_string())
}

/// List available versions of a plan (including the current and all backups).
pub fn list_plan_versions(session_id: &str) -> Result<Vec<PlanVersionInfo>> {
    let dir = plans_dir()?;
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
    versions.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(versions)
}

/// Load content of a specific plan version.
pub fn load_plan_version(file_path: &str) -> Result<String> {
    Ok(std::fs::read_to_string(file_path)?)
}
