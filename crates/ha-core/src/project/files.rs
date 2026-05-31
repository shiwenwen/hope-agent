//! Project working-directory resolution + project teardown.
//!
//! A project's files live directly in its working directory — either the
//! user's explicitly-selected `working_dir`, or the lazily-created default
//! workspace under `~/.hope-agent/projects/{id}/workspace/`. Uploading,
//! creating, and deleting individual files is handled by the filesystem API
//! ([`crate::filesystem`]); this module only resolves the project root and
//! tears the whole project down when it is deleted.

use anyhow::Result;
use std::path::PathBuf;

use super::db::ProjectDB;

/// Maximum size of a single uploaded project file (20 MB). Enforced by the
/// filesystem upload entry so routes can return a clean error before disk.
pub const MAX_PROJECT_FILE_BYTES: usize = 20 * 1024 * 1024;

/// Resolve the on-disk directory that backs a project: its explicitly-selected
/// `working_dir` when set, otherwise the default workspace (created on demand).
///
/// This returns the same directory the model and tools see as the working dir
/// (see [`crate::session::effective_working_dir_for_meta`]), so files created
/// through the filesystem API land exactly where `read`/`exec` resolve relative
/// paths.
pub fn resolve_project_dir(project_id: &str, db: &ProjectDB) -> Result<PathBuf> {
    let project = db
        .get(project_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", project_id))?;
    if let Some(wd) = project.working_dir.filter(|s| !s.trim().is_empty()) {
        return Ok(PathBuf::from(wd));
    }
    let ws = crate::paths::project_workspace_dir(project_id)?;
    Ok(PathBuf::from(crate::util::ensure_dir_canonical(&ws)?))
}

/// Delete a project and every resource attached to it:
///
/// 1. Clears `project_id` on every session (sessions survive).
/// 2. Deletes the project DB row.
/// 3. Removes the on-disk `projects/{id}/` directory (which holds the default
///    workspace). A user-selected *external* `working_dir` is never touched.
/// 4. Removes project-scoped memories from the memory backend.
///
/// Returns `Ok(false)` if the project did not exist.
pub fn delete_project_cascade(project_id: &str, db: &ProjectDB) -> Result<bool> {
    // Bail out if the project is gone already.
    if db.get(project_id)?.is_none() {
        return Ok(false);
    }

    // Step 1 + 2: DB side — session unassign + project row removal.
    db.delete(project_id)?;

    // Step 3: physical dir cleanup (best-effort).
    purge_project_dir(project_id);

    // Step 4: wipe project-scoped memories from memory.db. This is a separate
    // database and cannot ride the same transaction, so we do it last: if we
    // crash between step 2 and here, the only leftover is orphan memory rows
    // that are already unreachable via `project_id`.
    if let Some(backend) = crate::get_memory_backend() {
        let scope = crate::memory::MemoryScope::Project {
            id: project_id.to_string(),
        };
        if let Ok(project_mems) = backend.list(Some(&scope), None, 10_000, 0) {
            let ids: Vec<i64> = project_mems.into_iter().map(|m| m.id).collect();
            if !ids.is_empty() {
                let _ = backend.delete_batch(&ids);
            }
        }
    }

    Ok(true)
}

/// Remove a project's on-disk directory tree (`projects/{id}/`, which holds the
/// default workspace). Called when the parent project is deleted.
///
/// A user's externally-selected `working_dir` lives outside `projects/` and is
/// never purged here — the containment check rejects anything that resolves
/// outside the projects root.
pub fn purge_project_dir(project_id: &str) {
    let Ok(dir) = crate::paths::project_dir(project_id) else {
        return;
    };
    if !dir.exists() {
        return;
    }
    // Defense-in-depth: refuse to delete if `dir` canonicalizes outside the
    // projects root. Project IDs come from `Uuid::new_v4()` today so this
    // should never trigger, but a traversal-style id (or a symlink that escaped
    // the root) must not cause `remove_dir_all` to walk outside
    // `~/.hope-agent/projects/`.
    let Ok(projects_root) = crate::paths::projects_dir() else {
        return;
    };
    let canonical = match dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "project",
                "files",
                "Refusing to purge project {}: canonicalize failed: {}",
                project_id,
                e
            );
            return;
        }
    };
    let canonical_root = match projects_root.canonicalize() {
        Ok(p) => p,
        Err(_) => projects_root.clone(),
    };
    if !canonical.starts_with(&canonical_root) {
        app_error!(
            "project",
            "files",
            "Refusing to purge project {}: resolved path {:?} escapes projects root {:?}",
            project_id,
            canonical,
            canonical_root
        );
        return;
    }
    let _ = std::fs::remove_dir_all(canonical);
}
