//! Retention / purge logic for the `async_jobs` subsystem.
//!
//! A long-running install accumulates terminal job rows and their spool files
//! forever unless we explicitly clean them up. This module runs a retention
//! sweep over the DB (deleting rows older than the configured cutoff) and a
//! separate orphan sweep over the spool directory (catching files whose DB row
//! is missing — e.g. a crash between `persist_result` and the DB update).

use std::time::{SystemTime, UNIX_EPOCH};

use super::get_async_jobs_db;
use crate::paths;

/// Upper bound on orphan files removed in a single sweep. Prevents a
/// pathological spool directory (100k+ files) from starving the blocking
/// pool for minutes; leftovers are picked up on the next daily tick.
const MAX_ORPHANS_PER_SWEEP: u64 = 10_000;

/// Run one retention + orphan sweep according to the current config. Safe to
/// call at any time; guarded internally if the DB is not yet initialized.
pub fn run_once() {
    let cfg = crate::config::cached_config().async_tools.clone();
    let retention_secs = cfg.retention_secs;
    let orphan_grace_secs = cfg.orphan_grace_secs;

    if retention_secs == 0 && orphan_grace_secs == 0 {
        return;
    }

    let db = match get_async_jobs_db() {
        Some(db) => db.clone(),
        None => return,
    };

    if retention_secs > 0 {
        let now = chrono::Utc::now().timestamp();
        let cutoff = now.saturating_sub(retention_secs as i64);
        match db.purge_terminal_older_than(cutoff) {
            Ok(stats) if stats.rows_deleted > 0 || stats.spool_files_deleted > 0 => {
                crate::app_info!(
                    "async_jobs",
                    "retention",
                    "Purged {} row(s), {} spool file(s), {} byte(s) freed (cutoff={}s ago)",
                    stats.rows_deleted,
                    stats.spool_files_deleted,
                    stats.spool_bytes_freed,
                    retention_secs
                );
            }
            Ok(_) => {}
            Err(e) => crate::app_warn!(
                "async_jobs",
                "retention",
                "Row retention sweep failed: {}",
                e
            ),
        }
    }

    if orphan_grace_secs > 0 {
        if let Err(e) = sweep_orphans(&db, orphan_grace_secs) {
            crate::app_warn!(
                "async_jobs",
                "retention",
                "Orphan spool sweep failed: {}",
                e
            );
        }
    }
}

fn sweep_orphans(db: &super::AsyncJobsDB, orphan_grace_secs: u64) -> anyhow::Result<()> {
    let spool_dir = paths::async_jobs_dir()?;
    if !spool_dir.exists() {
        return Ok(());
    }

    let referenced = db.list_all_spool_paths()?;
    let now = SystemTime::now();
    let grace = std::time::Duration::from_secs(orphan_grace_secs);

    let mut deleted = 0u64;
    let mut freed = 0u64;

    for entry in std::fs::read_dir(&spool_dir)? {
        if deleted >= MAX_ORPHANS_PER_SWEEP {
            crate::app_warn!(
                "async_jobs",
                "retention",
                "Orphan sweep hit cap {}; remainder will be cleaned up next cycle",
                MAX_ORPHANS_PER_SWEEP
            );
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("txt") {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if referenced.contains(&path_str) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        let age = now.duration_since(mtime).unwrap_or_default();
        if age < grace {
            continue;
        }
        let size = meta.len();
        match std::fs::remove_file(&path) {
            Ok(()) => {
                deleted += 1;
                freed += size;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => crate::app_warn!(
                "async_jobs",
                "retention",
                "Failed to delete orphan spool file {}: {}",
                path.display(),
                e
            ),
        }
    }

    if deleted > 0 {
        crate::app_info!(
            "async_jobs",
            "retention",
            "Orphan spool sweep: {} file(s), {} byte(s) freed",
            deleted,
            freed
        );
    }

    Ok(())
}

/// Spawn a background task that runs retention once at startup and then once
/// per day. Returns immediately. Skipped entirely when both retention and
/// orphan sweeps are disabled, so a fully-off config doesn't leave a permanent
/// 24h ticker running doing nothing.
pub fn spawn_background_loop() {
    let cfg = crate::config::cached_config().async_tools.clone();
    if cfg.retention_secs == 0 && cfg.orphan_grace_secs == 0 {
        return;
    }

    tokio::spawn(async move {
        // Initial sweep, detached — don't block the ticker on a slow first pass.
        tokio::task::spawn_blocking(run_once);

        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(crate::SECS_PER_DAY));
        ticker.tick().await; // interval fires immediately on first tick; consume it
        loop {
            ticker.tick().await;
            tokio::task::spawn_blocking(run_once);
        }
    });
}
