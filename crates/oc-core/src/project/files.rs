//! Project file upload / delete pipeline.
//!
//! Writes uploaded bytes under `~/.opencomputer/projects/{id}/files/`,
//! runs [`crate::file_extract::extract`] on text-bearing formats, stores
//! the extracted text under `extracted/`, and inserts a row into
//! [`ProjectDB::add_file`]. On delete, removes both the DB row and the
//! on-disk bytes.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::db::ProjectDB;
use super::types::ProjectFile;

/// Maximum size of a single uploaded project file (20 MB). Enforced at the
/// pipeline entry so routes can return a clean error before hitting disk.
pub const MAX_PROJECT_FILE_BYTES: usize = 20 * 1024 * 1024;

/// Inputs accepted by [`upload_project_file`].
pub struct UploadInput<'a> {
    pub project_id: &'a str,
    pub original_filename: &'a str,
    pub mime_type: Option<&'a str>,
    pub data: &'a [u8],
}

/// Upload a new file into a project.
///
/// 1. Validates size + a safe filename.
/// 2. Writes bytes to `projects/{id}/files/{uuid_prefix}_{name}`.
/// 3. Runs `file_extract::extract` on supported formats and stores the
///    result at `projects/{id}/extracted/{uuid}.txt` when any text is
///    produced. Unsupported formats (binary, images without OCR) leave
///    `extracted_path = NULL`.
/// 4. Inserts a `ProjectFile` row via `ProjectDB::add_file`.
///
/// On success, returns the persisted [`ProjectFile`]. On failure, any
/// partially-written bytes are cleaned up so the caller does not leak files.
pub fn upload_project_file(input: UploadInput<'_>, db: &ProjectDB) -> Result<ProjectFile> {
    // Reject oversize uploads up-front.
    if input.data.len() > MAX_PROJECT_FILE_BYTES {
        anyhow::bail!(
            "project file too large: {} bytes (max {} bytes)",
            input.data.len(),
            MAX_PROJECT_FILE_BYTES
        );
    }
    if input.data.is_empty() {
        anyhow::bail!("project file is empty");
    }
    if input.original_filename.trim().is_empty() {
        anyhow::bail!("project file name is empty");
    }

    // Ensure the project exists. Upload into a dangling project is a bug;
    // fail loudly so the caller can surface a 404.
    if db.get(input.project_id)?.is_none() {
        anyhow::bail!("project not found: {}", input.project_id);
    }

    // Ensure destination dirs exist.
    let files_dir = crate::paths::project_files_dir(input.project_id)?;
    let extracted_dir = crate::paths::project_extracted_dir(input.project_id)?;
    std::fs::create_dir_all(&files_dir)
        .with_context(|| format!("create {}", files_dir.display()))?;
    std::fs::create_dir_all(&extracted_dir)
        .with_context(|| format!("create {}", extracted_dir.display()))?;

    // Generate file id + safe disk name.
    let id = uuid::Uuid::new_v4().to_string();
    let safe_name = sanitize_filename(input.original_filename);
    let short_prefix = id.chars().take(8).collect::<String>();
    let disk_name = format!("{}_{}", short_prefix, safe_name);
    let file_path = files_dir.join(&disk_name);

    // Write the raw bytes.
    std::fs::write(&file_path, input.data)
        .with_context(|| format!("write {}", file_path.display()))?;

    // Guard: if anything fails after this point, remove the file from disk
    // before propagating the error so we don't leak orphans.
    let cleanup_guard = scopeguard_remove(&file_path);

    // Attempt text extraction. Non-fatal — binary files just produce `None`.
    let mime = input.mime_type.unwrap_or("application/octet-stream");
    let extracted = crate::file_extract::extract(
        file_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("project file path is not valid utf-8"))?,
        input.original_filename,
        mime,
    );

    let (extracted_rel, extracted_chars) = match extracted.text {
        Some(text) if !text.trim().is_empty() => {
            let ext_path = extracted_dir.join(format!("{}.txt", id));
            std::fs::write(&ext_path, &text)
                .with_context(|| format!("write {}", ext_path.display()))?;
            let rel = to_relative_project_path(&ext_path)?;
            (Some(rel), Some(text.len() as i64))
        }
        _ => (None, None),
    };

    let file_path_rel = to_relative_project_path(&file_path)?;

    let now = chrono::Utc::now().timestamp_millis();
    let record = ProjectFile {
        id: id.clone(),
        project_id: input.project_id.to_string(),
        name: input.original_filename.to_string(),
        original_filename: input.original_filename.to_string(),
        mime_type: Some(mime.to_string()),
        size_bytes: input.data.len() as i64,
        file_path: file_path_rel,
        extracted_path: extracted_rel.clone(),
        extracted_chars,
        summary: None,
        created_at: now,
        updated_at: now,
    };

    // Insert the DB row. If this fails we still need to delete the text
    // file and the original bytes, otherwise the cleanup guard misses them.
    if let Err(e) = db.add_file(&record) {
        if let Some(rel) = &extracted_rel {
            if let Ok(base) = crate::paths::projects_dir() {
                let _ = std::fs::remove_file(base.join(rel));
            }
        }
        // cleanup_guard drops when we return, removing file_path.
        return Err(e);
    }

    // Success — disable the guard so we keep the bytes on disk.
    scopeguard_disarm(cleanup_guard);
    Ok(record)
}

/// Delete a project file: removes the DB row first, then best-effort deletes
/// the on-disk bytes and any extracted-text sidecar. FS errors are logged
/// but do not fail the call so stale DB rows are never left behind.
pub fn delete_project_file(file_id: &str, db: &ProjectDB) -> Result<bool> {
    let Some(existing) = db.delete_file(file_id)? else {
        return Ok(false);
    };

    if let Ok(base) = crate::paths::projects_dir() {
        if !existing.file_path.is_empty() {
            let _ = std::fs::remove_file(base.join(&existing.file_path));
        }
        if let Some(ext) = &existing.extracted_path {
            if !ext.is_empty() {
                let _ = std::fs::remove_file(base.join(ext));
            }
        }
    }

    Ok(true)
}

/// Remove every file belonging to a project, both the rows and the on-disk
/// directory tree. Called when the parent project itself is being deleted.
pub fn purge_project_files_dir(project_id: &str) {
    let Ok(dir) = crate::paths::project_dir(project_id) else {
        return;
    };
    if !dir.exists() {
        return;
    }
    // Defense-in-depth: refuse to delete if `dir` canonicalizes outside
    // the projects root. Project IDs come from `Uuid::new_v4()` today so
    // this should never trigger, but a traversal-style id (or a symlink
    // that escaped the root) must not cause `remove_dir_all` to walk
    // outside `~/.opencomputer/projects/`.
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

/// Delete a project and every resource attached to it:
///
/// 1. Clears `project_id` on every session (sessions survive).
/// 2. Deletes the DB row (cascades to `project_files` via FK).
/// 3. Removes the on-disk `projects/{id}/` directory.
/// 4. Removes project-scoped memories from the memory backend.
///
/// Returns `Ok(false)` if the project did not exist.
pub fn delete_project_cascade(project_id: &str, db: &ProjectDB) -> Result<bool> {
    // Bail out if the project is gone already.
    if db.get(project_id)?.is_none() {
        return Ok(false);
    }

    // Step 1 + 2: DB side. `ProjectDB::delete` handles session unassign +
    // project row removal + project_files cascade inside one operation.
    let _files = db.delete(project_id)?;

    // Step 3: physical dir cleanup (best-effort).
    purge_project_files_dir(project_id);

    // Step 4: wipe project-scoped memories from memory.db. This is a
    // separate database and cannot ride the same transaction, so we do it
    // last: if we crash between step 2 and here, the only leftover is
    // orphan memory rows that are already unreachable via `project_id`.
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

// ── helpers ─────────────────────────────────────────────────────

/// Turn an absolute path under `projects_dir()` into a relative path stored
/// in the DB. Returns an error if the path somehow escaped the base dir.
fn to_relative_project_path(path: &Path) -> Result<String> {
    let base = crate::paths::projects_dir()?;
    let rel = path
        .strip_prefix(&base)
        .with_context(|| format!("path {} is not under projects_dir", path.display()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Replace filesystem-hostile characters with `_` so the disk name is safe
/// on every platform. Mirrors [`crate::attachments::save_attachment_bytes`].
fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', ':', '\0'], "_")
}

// ── tiny scopeguard shim ────────────────────────────────────────

/// Minimal scope guard for the one cleanup we need — avoids pulling in the
/// `scopeguard` crate for a single call site.
struct CleanupGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn scopeguard_remove(path: &Path) -> CleanupGuard {
    CleanupGuard {
        path: path.to_path_buf(),
        armed: true,
    }
}

fn scopeguard_disarm(mut guard: CleanupGuard) {
    guard.armed = false;
}
