//! Cross-database orphan reconciler for project-scoped memory rows.
//!
//! `delete_project_cascade` (see `project/files.rs`) modifies two databases
//! in sequence: `session.db` (project row + sessions + project_files) and
//! `memory.db` (project-scoped memories). Because they cannot share a
//! transaction, a crash between the two leaves orphan memory rows that are
//! no longer reachable from any live project. Symlink-style path escapes are
//! already blocked by [`super::files::purge_project_files_dir`], so the
//! remaining failure mode is purely "memory.db has rows with `scope_type =
//! 'project'` and a `scope_project_id` that the projects table no longer
//! knows about". This module sweeps those at startup.
//!
//! Runs once per process (out of `app_init::start_background_tasks`). Project
//! deletion is low-frequency, so a periodic timer would be wasted work.
//! Failures only `app_warn!` — we never want a stale memory row to block
//! application startup.

use anyhow::Result;
use std::collections::HashSet;

use crate::memory::traits::MemoryBackend;
use crate::{app_info, app_warn};

/// Scan `memory.db` for project-scoped rows whose owning project no longer
/// exists in `session.db`, then `delete_batch` them. Returns the number of
/// rows deleted (across all orphan projects).
///
/// Returns `Ok(0)` when either the project DB or the memory backend is not
/// initialised — the reconciler is opportunistic, not load-bearing.
pub fn reconcile_orphan_project_memories() -> Result<usize> {
    let project_db = match crate::get_project_db() {
        Some(db) => db,
        None => return Ok(0),
    };
    let backend = match crate::get_memory_backend() {
        Some(b) => b,
        None => return Ok(0),
    };

    let alive: HashSet<String> = project_db.list_all_ids()?.into_iter().collect();
    reconcile_against(backend.as_ref(), &alive)
}

/// Pure-function core of [`reconcile_orphan_project_memories`]. Takes the
/// alive project id set explicitly so unit tests can drive it without
/// reaching into the global registries.
pub fn reconcile_against(
    backend: &dyn MemoryBackend,
    alive: &HashSet<String>,
) -> Result<usize> {
    let referenced = backend.list_distinct_project_scope_ids()?;
    let orphans: Vec<String> = referenced
        .into_iter()
        .filter(|id| !alive.contains(id))
        .collect();

    if orphans.is_empty() {
        return Ok(0);
    }

    let mut total_deleted = 0usize;
    for project_id in &orphans {
        let scope = crate::memory::MemoryScope::Project {
            id: project_id.clone(),
        };
        // 10_000 mirrors the upper bound `delete_project_cascade` uses on
        // the hot path. Project memories are user-authored notes — anything
        // approaching that ceiling already indicates an outlier.
        let entries = match backend.list(Some(&scope), None, 10_000, 0) {
            Ok(v) => v,
            Err(e) => {
                app_warn!(
                    "project",
                    "reconcile",
                    "Failed to list orphan memories for project {}: {}",
                    project_id,
                    e
                );
                continue;
            }
        };
        let ids: Vec<i64> = entries.into_iter().map(|m| m.id).collect();
        match backend.delete_batch(&ids) {
            Ok(n) => total_deleted += n,
            Err(e) => app_warn!(
                "project",
                "reconcile",
                "delete_batch failed for orphan project {}: {}",
                project_id,
                e
            ),
        }
    }

    if total_deleted > 0 {
        app_info!(
            "project",
            "reconcile",
            "Reaped {} orphan project-scoped memory rows across {} dead projects",
            total_deleted,
            orphans.len()
        );
    }
    Ok(total_deleted)
}

/// Spawn the startup orphan-memory reconciler as a one-shot blocking task.
/// Mirrors [`crate::recap::spawn_facet_retention_loop`]'s pattern so the spawn
/// site stays in the module that owns the work, not in `app_init`.
pub fn spawn_startup_reconciler() {
    tokio::task::spawn_blocking(|| {
        if let Err(e) = reconcile_orphan_project_memories() {
            app_warn!(
                "project",
                "reconcile",
                "Startup orphan-memory reconciler failed: {}",
                e
            );
        }
    });
}

