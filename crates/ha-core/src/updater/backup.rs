//! Backup + rollback storage for the self-contained updater.
//!
//! Per-version layout under `~/.hope-agent/updater/backup/`:
//!
//! ```text
//! backup/
//!   v0.1.0/hope-agent
//!   v0.2.0/hope-agent
//! ```
//!
//! On `app_update install` we copy the current binary into
//! `backup/<old_version>/` before the swap. On `app_update rollback` we
//! pick the most recent backup, atomic-swap it back into the live path,
//! and restart the service.
//!
//! Cap retained backups at 2 — enough for "downgrade after a failed
//! upgrade", not so much that 5 MB binaries balloon to GBs across years.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const RETAINED_BACKUPS: usize = 2;

/// Copy the binary currently at `current_exe` to the backup slot for
/// `current_version`. Idempotent — re-running for the same version
/// overwrites the prior copy (which is fine: the most recent restorable
/// image is what we care about).
pub fn store(current_exe: &Path, current_version: &str) -> Result<PathBuf> {
    let dir = crate::paths::updater_backup_dir(current_version)?;
    fs::create_dir_all(&dir).with_context(|| format!("create backup dir {}", dir.display()))?;
    let dest = dir.join(binary_name());
    fs::copy(current_exe, &dest).with_context(|| {
        format!(
            "copy {} → {} for backup",
            current_exe.display(),
            dest.display()
        )
    })?;
    Ok(dest)
}

/// Trim the backup root down to [`RETAINED_BACKUPS`] most-recent entries
/// (sorted by directory mtime). Best-effort — any pruning IO error is
/// logged but does not fail the upgrade itself.
pub fn prune() {
    let root = match crate::paths::updater_dir() {
        Ok(p) => p.join("backup"),
        Err(_) => return,
    };
    if !root.is_dir() {
        return;
    }
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = match fs::read_dir(&root) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let mtime = e.metadata().ok()?.modified().ok()?;
                if path.is_dir() {
                    Some((path, mtime))
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| std::cmp::Reverse(e.1)); // newest first
    for (path, _) in entries.into_iter().skip(RETAINED_BACKUPS) {
        if let Err(e) = fs::remove_dir_all(&path) {
            app_warn!(
                "self_update",
                "prune",
                "Failed to prune backup {}: {}",
                path.display(),
                e
            );
        }
    }
}

/// Most-recent backup binary path, if any. Used by `app_update rollback`.
pub fn most_recent() -> Option<PathBuf> {
    let root = crate::paths::updater_dir().ok()?.join("backup");
    if !root.is_dir() {
        return None;
    }
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in fs::read_dir(&root).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let bin = path.join(binary_name());
        if !bin.is_file() {
            continue;
        }
        let mtime = match entry.metadata().ok().and_then(|m| m.modified().ok()) {
            Some(t) => t,
            None => continue,
        };
        if best.as_ref().map(|(_, t)| mtime > *t).unwrap_or(true) {
            best = Some((bin, mtime));
        }
    }
    best.map(|(p, _)| p)
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "hope-agent.exe"
    } else {
        "hope-agent"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: backup root resolves and the binary name matches the host.
    /// Full round-trip (store → most_recent → restore) requires writing
    /// into `~/.hope-agent/`, so we cover it from `tests/updater_e2e.rs`
    /// behind a `tempfile` + `HA_DATA_DIR` override.
    #[test]
    fn binary_name_matches_host_os() {
        let name = binary_name();
        if cfg!(windows) {
            assert_eq!(name, "hope-agent.exe");
        } else {
            assert_eq!(name, "hope-agent");
        }
    }
}
