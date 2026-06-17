//! Staging-directory garbage collection for the self-contained updater.
//!
//! Downloads land in `~/.hope-agent/updater/staging/<version>/` (see
//! [`crate::paths::updater_staging_dir`]). A download that crashes mid-flight,
//! a verify failure, or a swap failure all leave a half-written archive behind.
//! Nothing cleaned these up before, so a daemon that retried failed upgrades
//! would slowly accumulate dead staging dirs.
//!
//! Policy:
//! - On startup + before each fresh stage, drop staging dirs for versions other
//!   than the one we're about to (or just did) work on.
//! - As a backstop, drop any staging dir older than [`STALE_AFTER`] regardless.
//!
//! Best-effort throughout — any IO error is logged, never fatal. Backups are a
//! *separate* tree ([`super::backup`]) and are never touched here.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Staging dirs older than this are pruned even if they match no "keep" filter.
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

fn staging_root() -> Option<PathBuf> {
    let root = crate::paths::updater_dir().ok()?.join("staging");
    root.is_dir().then_some(root)
}

/// Remove every staging dir except the one for `keep_version` (when given) and
/// any that is still fresh enough to be a legitimate in-progress download.
pub fn prune(keep_version: Option<&str>) {
    let Some(root) = staging_root() else {
        return;
    };
    let keep = keep_version.map(|v| v.trim_start_matches('v').to_string());
    let now = SystemTime::now();

    let entries = match fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(e) => {
            app_warn!(
                "self_update",
                "staging_prune",
                "read staging root {} failed: {}",
                root.display(),
                e
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(keep) = keep.as_deref() {
            if name.trim_start_matches('v') == keep {
                continue;
            }
        }
        // Always keep recently-touched dirs, even when a keep target is set: a
        // sibling dir for *another* version may be an install that a concurrent
        // task (e.g. a manual `app_update install` racing the auto-loop) is
        // actively downloading into — yanking it mid-flight would surface a
        // confusing IO error. Only sweep dirs that have gone stale; fresh
        // siblings get cleaned on a later pass once they age out.
        if let Ok(age) = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| now.duration_since(t).unwrap_or_default())
        {
            if age < STALE_AFTER {
                continue;
            }
        }
        match fs::remove_dir_all(&path) {
            Ok(_) => app_info!(
                "self_update",
                "staging_prune",
                "pruned staging dir {}",
                path.display()
            ),
            Err(e) => app_warn!(
                "self_update",
                "staging_prune",
                "prune staging dir {} failed: {}",
                path.display(),
                e
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_no_root_is_noop() {
        // No staging dir present → must not panic / error.
        prune(Some("0.0.0"));
        prune(None);
    }
}
