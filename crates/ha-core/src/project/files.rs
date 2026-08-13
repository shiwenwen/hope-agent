//! Project working-directory resolution + project teardown.
//!
//! A project's files live directly in its working directory — either the
//! user's explicitly-selected `working_dir`, or the lazily-created default
//! workspace under `~/.hope-agent/projects/{id}/workspace/`. Uploading,
//! creating, and deleting individual files is handled by the filesystem API
//! ([`crate::filesystem`]); this module only resolves the project root and
//! tears the whole project down when it is deleted.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::db::ProjectDB;
use super::types::{CreateProjectInput, Project, UpdateProjectInput};

/// Project instruction filename. This is intentionally fixed: project settings
/// and system-prompt discovery must always operate on the same root file.
pub const PROJECT_INSTRUCTIONS_FILE: &str = "AGENTS.md";

/// Keep the settings editor bounded. The system prompt has a much smaller
/// per-file injection cap, but allowing a few megabytes here remains convenient
/// for real-world handbooks while preventing an accidental giant file from
/// freezing the webview.
fn max_project_instructions_bytes() -> u64 {
    crate::config::cached_config()
        .filesystem
        .max_text_edit_bytes()
}

/// Contents and absolute location of a project's root `AGENTS.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionsFile {
    pub path: String,
    pub content: String,
    /// Raw BLAKE3 of the file bytes. Clients must echo this when saving so an
    /// external edit cannot be overwritten by a stale draft.
    pub content_hash: String,
    /// Whether the file existed when it was inspected or read.
    pub exists: bool,
    /// True when this call created a previously-missing file.
    pub created: bool,
}

/// AGENTS.md source submitted together with project metadata. This remains a
/// filesystem draft only; it is never persisted in the project database row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionsDraft {
    pub content: String,
    pub expected_file_hash: String,
    /// Expected existence state observed alongside `expected_file_hash`.
    /// `None` keeps older clients fail-closed by assuming the file existed.
    #[serde(default)]
    pub expected_exists: Option<bool>,
}

/// Marker error for optimistic-concurrency failures while saving AGENTS.md.
/// Shell adapters use this to return a conflict instead of a generic failure.
#[derive(Debug)]
pub struct StaleProjectInstructionsError;

impl std::fmt::Display for StaleProjectInstructionsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "stale write: AGENTS.md changed or was removed on disk; reload before saving",
        )
    }
}

impl std::error::Error for StaleProjectInstructionsError {}

/// `WorkspaceScope::for_project` 的根解析器（经 `app_init` 注册进
/// `filesystem::workspace` 的钩子）。
pub(crate) fn workspace_root(
    project_id: &str,
) -> std::result::Result<crate::filesystem::ResolvedRoot, crate::filesystem::FilesystemError> {
    use crate::filesystem::{FilesystemError, ResolvedRoot};
    let db = crate::get_project_db()
        .ok_or_else(|| FilesystemError::internal("project db not initialized"))?;
    let project = db
        .get(project_id)
        .map_err(|e| FilesystemError::internal(e.to_string()))?
        .ok_or_else(|| FilesystemError::bad_input("project not found"))?;
    let dir = resolve_project_dir(project_id, &db)
        .map_err(|e| FilesystemError::bad_input(e.to_string()))?;
    Ok(ResolvedRoot {
        dir,
        read_only: project.archived,
    })
}

/// `WorkspaceScope::for_project_folder` resolver. The encoded id is
/// `"{base_scope}:{base_id}:{linked_index}:{expected_path}"`; `splitn(4)` keeps
/// Windows drive-letter colons and any later colons inside the expected path.
/// The index plus exact path forms a stale-selection guard: a project edit can
/// never make an already-open browser silently target a different folder.
pub(crate) fn workspace_linked_root(
    encoded_id: &str,
) -> std::result::Result<crate::filesystem::ResolvedRoot, crate::filesystem::FilesystemError> {
    use crate::filesystem::{FilesystemError, ResolvedRoot};

    let (base_scope, base_id, linked_index, expected_path) =
        parse_project_folder_scope(encoded_id)?;

    let project_id = match base_scope {
        "project" => base_id.to_string(),
        "session" => {
            let session_db = crate::require_session_db()
                .map_err(|e| FilesystemError::internal(e.to_string()))?;
            session_db
                .get_session(base_id)
                .map_err(|e| FilesystemError::internal(e.to_string()))?
                .and_then(|session| session.project_id)
                .ok_or_else(|| FilesystemError::bad_input("session is not attached to a project"))?
        }
        _ => {
            return Err(FilesystemError::bad_input(
                "invalid base scope for project folder",
            ))
        }
    };

    let project_db = crate::get_project_db()
        .ok_or_else(|| FilesystemError::internal("project db not initialized"))?;
    let project = project_db
        .get(&project_id)
        .map_err(|e| FilesystemError::internal(e.to_string()))?
        .ok_or_else(|| FilesystemError::bad_input("project not found"))?;
    let linked_path = project
        .linked_dirs
        .get(linked_index)
        .filter(|path| path.as_str() == expected_path)
        .ok_or_else(|| FilesystemError::bad_input("project folder is no longer attached"))?;

    Ok(ResolvedRoot {
        dir: PathBuf::from(linked_path),
        read_only: project.archived,
    })
}

fn parse_project_folder_scope(
    encoded_id: &str,
) -> std::result::Result<(&str, &str, usize, &str), crate::filesystem::FilesystemError> {
    use crate::filesystem::FilesystemError;

    let mut parts = encoded_id.splitn(4, ':');
    let base_scope = parts.next().unwrap_or("");
    let base_id = parts.next().unwrap_or("");
    let linked_index = parts
        .next()
        .and_then(|raw| raw.parse::<usize>().ok())
        .ok_or_else(|| FilesystemError::bad_input("invalid project folder scope"))?;
    let expected_path = parts.next().unwrap_or("");
    if base_id.is_empty() || expected_path.is_empty() {
        return Err(FilesystemError::bad_input("invalid project folder scope"));
    }
    Ok((base_scope, base_id, linked_index, expected_path))
}

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

/// Create a project, optionally ensure its root `AGENTS.md` exists, and replace
/// it with a concurrency-checked draft when supplied. The DB row is removed
/// again when the filesystem operation fails, so callers never receive a
/// partially-created project.
pub fn create_project_with_instructions_file(
    input: CreateProjectInput,
    instructions: Option<ProjectInstructionsDraft>,
    create_instructions_if_missing: bool,
    db: &ProjectDB,
) -> Result<Project> {
    let project = db.create(input)?;
    let prepare_result = (|| -> Result<()> {
        if let Some(draft) = instructions {
            if !create_instructions_if_missing && !draft.expected_exists.unwrap_or(true) {
                anyhow::bail!(
                    "project instructions draft requires creating AGENTS.md, but creation is disabled"
                );
            }
            save_project_instructions_draft(&project.id, draft, db)?;
        } else if create_instructions_if_missing {
            ensure_project_instructions(&project.id, db)?;
        }
        Ok(())
    })();
    if let Err(error) = prepare_result {
        if let Err(rollback_error) = db.delete(&project.id) {
            anyhow::bail!(
                "failed to create project AGENTS.md: {error}; project rollback also failed: {rollback_error}"
            );
        }
        purge_project_dir(&project.id);
        return Err(error);
    }
    Ok(project)
}

/// Update a project, establish `AGENTS.md` in a newly-selected working
/// directory, and optionally save a concurrency-checked draft. If any
/// filesystem step fails, restore the previous project record.
pub fn update_project_with_instructions_file(
    project_id: &str,
    patch: UpdateProjectInput,
    instructions: Option<ProjectInstructionsDraft>,
    db: &ProjectDB,
) -> Result<Project> {
    let previous = db
        .get(project_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {project_id}"))?;
    let updated = db.update(project_id, patch)?;
    let working_dir_changed = updated.working_dir.as_deref() != previous.working_dir.as_deref();
    let prepare_result = (|| -> Result<()> {
        if let Some(draft) = instructions {
            save_project_instructions_draft(project_id, draft, db)?;
        } else if working_dir_changed {
            ensure_project_instructions(project_id, db)?;
        }
        Ok(())
    })();
    if let Err(error) = prepare_result {
        let rollback = UpdateProjectInput {
            name: Some(previous.name),
            description: Some(previous.description.unwrap_or_default()),
            logo: Some(previous.logo.unwrap_or_default()),
            color: Some(previous.color.unwrap_or_default()),
            default_agent_id: Some(previous.default_agent_id.unwrap_or_default()),
            default_model_id: Some(previous.default_model_id.unwrap_or_default()),
            working_dir: Some(previous.working_dir.unwrap_or_default()),
            linked_dirs: Some(previous.linked_dirs),
            archived: Some(previous.archived),
        };
        if let Err(rollback_error) = db.update(project_id, rollback) {
            anyhow::bail!(
                "failed to prepare project AGENTS.md: {error}; project rollback also failed: {rollback_error}"
            );
        }
        return Err(error);
    }
    Ok(updated)
}

fn save_project_instructions_draft(
    project_id: &str,
    draft: ProjectInstructionsDraft,
    db: &ProjectDB,
) -> Result<ProjectInstructionsFile> {
    let expected_exists = draft.expected_exists.unwrap_or(true);
    let expected_hash = if draft.expected_file_hash.is_empty() {
        if expected_exists {
            return Err(StaleProjectInstructionsError.into());
        }
        blake3::hash(b"").to_hex().to_string()
    } else {
        draft.expected_file_hash
    };
    save_project_instructions(
        project_id,
        &draft.content,
        &expected_hash,
        expected_exists,
        db,
    )
}

/// Ensure `<project-root>/AGENTS.md` exists without reading or rewriting an
/// existing file. Returns `true` only when a new empty file was created.
pub fn ensure_project_instructions(project_id: &str, db: &ProjectDB) -> Result<bool> {
    let root = resolve_project_dir(project_id, db)?;
    ensure_instructions_at_root(&root)
}

/// Read the project root's `AGENTS.md` without creating a missing file. The
/// file must be regular UTF-8 text; symlinks are rejected so this owner endpoint
/// cannot be used to read an arbitrary path outside the project.
pub fn read_project_instructions(
    project_id: &str,
    db: &ProjectDB,
) -> Result<ProjectInstructionsFile> {
    let root = resolve_project_dir(project_id, db)?;
    inspect_project_instructions_at_root(&root)
}

/// Inspect an existing working directory without creating AGENTS.md. Missing
/// files are represented as an empty draft whose hash matches the empty file
/// that project creation/update will establish before saving.
pub fn inspect_project_instructions(working_dir: &str) -> Result<ProjectInstructionsFile> {
    let canonical = crate::util::canonicalize_working_dir(Some(working_dir))?
        .ok_or_else(|| anyhow::anyhow!("project working directory is required"))?;
    let root = PathBuf::from(canonical);
    inspect_project_instructions_at_root(&root)
}

/// Inspect the managed default workspace for an existing project. The
/// workspace directory may be prepared, but a missing AGENTS.md is not created
/// until the project update is committed.
pub fn inspect_default_project_instructions(
    project_id: &str,
    db: &ProjectDB,
) -> Result<ProjectInstructionsFile> {
    if db.get(project_id)?.is_none() {
        anyhow::bail!("project not found: {project_id}");
    }
    let workspace = crate::paths::project_workspace_dir(project_id)?;
    let canonical = crate::util::ensure_dir_canonical(&workspace)?;
    inspect_project_instructions_at_root(Path::new(&canonical))
}

fn inspect_project_instructions_at_root(root: &Path) -> Result<ProjectInstructionsFile> {
    let path = root.join(PROJECT_INSTRUCTIONS_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => read_project_instructions_at_root(root, false),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(ProjectInstructionsFile {
            path: path.to_string_lossy().to_string(),
            content: String::new(),
            content_hash: blake3::hash(b"").to_hex().to_string(),
            exists: false,
            created: false,
        }),
        Err(error) => Err(error).with_context(|| format!("stat {}", path.display())),
    }
}

fn read_project_instructions_at_root(
    root: &Path,
    created: bool,
) -> Result<ProjectInstructionsFile> {
    let path = root.join(PROJECT_INSTRUCTIONS_FILE);
    let link_metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    if link_metadata.file_type().is_symlink() {
        anyhow::bail!("project AGENTS.md must not be a symbolic link");
    }
    if !link_metadata.is_file() {
        anyhow::bail!("project AGENTS.md is not a regular file");
    }
    let metadata = std::fs::metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    let max_bytes = max_project_instructions_bytes();
    if metadata.len() > max_bytes {
        anyhow::bail!(
            "AGENTS.md is too large to edit: {} bytes (max {} bytes)",
            metadata.len(),
            max_bytes
        );
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read {} as UTF-8", path.display()))?;
    Ok(ProjectInstructionsFile {
        path: path.to_string_lossy().to_string(),
        content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
        content,
        exists: true,
        created,
    })
}

/// Atomically write the project root's `AGENTS.md` with the exact Markdown
/// source supplied by the user. Missing files are published directly with
/// create-new semantics, so a failed write cannot leave an empty placeholder
/// behind. Whitespace is preserved byte-for-byte.
pub fn save_project_instructions(
    project_id: &str,
    content: &str,
    expected_file_hash: &str,
    expected_exists: bool,
    db: &ProjectDB,
) -> Result<ProjectInstructionsFile> {
    let max_bytes = max_project_instructions_bytes();
    if content.len() as u64 > max_bytes {
        anyhow::bail!(
            "AGENTS.md is too large to save: {} bytes (max {} bytes)",
            content.len(),
            max_bytes
        );
    }
    let root = resolve_project_dir(project_id, db)?;
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("resolve project root {}", root.display()))?;
    let path = canonical_root.join(PROJECT_INSTRUCTIONS_FILE);
    let created = match std::fs::symlink_metadata(&path) {
        Ok(_) if !expected_exists => return Err(StaleProjectInstructionsError.into()),
        Ok(_) => false,
        Err(error) if error.kind() == ErrorKind::NotFound && expected_exists => {
            return Err(StaleProjectInstructionsError.into());
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let empty_hash = blake3::hash(b"").to_hex().to_string();
            if expected_file_hash != empty_hash {
                return Err(StaleProjectInstructionsError.into());
            }
            match crate::platform::write_atomic_create_new(&path, content.as_bytes()) {
                Ok(()) => {
                    return Ok(ProjectInstructionsFile {
                        path: path.to_string_lossy().to_string(),
                        content: content.to_string(),
                        content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
                        exists: true,
                        created: true,
                    });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return Err(StaleProjectInstructionsError.into());
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("write {}", path.display()));
                }
            }
        }
        Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
    };
    let metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("project AGENTS.md must not be a symbolic link");
    }
    if !metadata.is_file() {
        anyhow::bail!("project AGENTS.md is not a regular file");
    }
    let max_bytes = max_project_instructions_bytes();
    if metadata.len() > max_bytes {
        anyhow::bail!(
            "AGENTS.md is too large to save: {} bytes (max {} bytes)",
            metadata.len(),
            max_bytes
        );
    }
    let current = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let current_hash = blake3::hash(&current).to_hex().to_string();
    if current_hash != expected_file_hash {
        return Err(StaleProjectInstructionsError.into());
    }

    crate::platform::write_atomic(&path, content.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    Ok(ProjectInstructionsFile {
        path: path.to_string_lossy().to_string(),
        content: content.to_string(),
        content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
        exists: true,
        created,
    })
}

fn ensure_instructions_at_root(root: &std::path::Path) -> Result<bool> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("resolve project root {}", root.display()))?;
    if !canonical_root.is_dir() {
        anyhow::bail!(
            "project root is not a directory: {}",
            canonical_root.display()
        );
    }

    let path = canonical_root.join(PROJECT_INSTRUCTIONS_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!("project AGENTS.md must not be a symbolic link");
            }
            if !metadata.is_file() {
                anyhow::bail!("project AGENTS.md is not a regular file");
            }
            return Ok(false);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
    }

    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => {
            file.sync_all()
                .with_context(|| format!("sync {}", path.display()))?;
            Ok(true)
        }
        // Another request won the create race; validate what appeared.
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("stat {}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                anyhow::bail!("project AGENTS.md is not a regular file");
            }
            Ok(false)
        }
        Err(error) => Err(error).with_context(|| format!("create {}", path.display())),
    }
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

    // Step 0 (E7 / DELETE-6): cancel in-flight async jobs for every session in
    // this project BEFORE unassigning them. `db.delete` only clears `project_id`
    // — the sessions survive, so no `session:deleted` fires and the cleanup
    // watcher never sees them. Once `project_id` is NULL there's no link left to
    // find these jobs, so they'd run on against an orphaned workspace. Cancel
    // here while the link still exists. Best-effort: a lookup failure must not
    // block the deletion.
    if let Some(session_db) = crate::get_session_db() {
        match session_db.session_ids_in_project(project_id) {
            Ok(ids) => {
                let mut cancelled = 0;
                for sid in &ids {
                    cancelled += crate::async_jobs::JobManager::cancel_for_session(sid);
                }
                if cancelled > 0 {
                    crate::app_info!(
                        "project",
                        "delete_cascade",
                        "cancelled {} async job(s) across {} session(s) before deleting project {}",
                        cancelled,
                        ids.len(),
                        project_id
                    );
                }
            }
            Err(e) => crate::app_warn!(
                "project",
                "delete_cascade",
                "session_ids_in_project failed for {} ({}); proceeding with deletion",
                project_id,
                e
            ),
        }
    }

    // Step 1 + 2: DB side — session unassign + project row removal.
    db.delete(project_id)?;

    // Step 3: physical dir cleanup (best-effort).
    purge_project_dir(project_id);

    // Step 4: wipe project-scoped memories from memory.db. This is a separate
    // database and cannot ride the same transaction, so we do it last: if we
    // crash between step 2 and here, the only leftover is orphan memory rows
    // that are already unreachable via `project_id`.
    let scope = crate::memory::MemoryScope::Project {
        id: project_id.to_string(),
    };
    if let Some(backend) = crate::get_memory_backend() {
        if let Ok(project_mems) = backend.list(Some(&scope), None, 10_000, 0) {
            let ids: Vec<i64> = project_mems.into_iter().map(|m| m.id).collect();
            if !ids.is_empty() {
                let _ = backend.delete_batch(&ids);
            }
        }
    }

    // Step 5: tear down the structured claim graph for this project scope
    // (claims + evidence + links + vectors + profile snapshots). The claim layer
    // lives in the same memory.db but is not covered by the legacy `delete_batch`
    // above, so a deleted project would otherwise leave orphan claims that still
    // surface in the claim list / Lucid Review. Best-effort: a failure here must
    // not block the rest of the cascade.
    if let Err(e) = crate::memory::claims::delete_claims_for_scope(&scope) {
        crate::app_warn!(
            "project",
            "delete_cascade",
            "failed to purge claims for deleted project {} ({})",
            project_id,
            e
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionDB;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn project_db(db_dir: &std::path::Path) -> ProjectDB {
        let session_db =
            Arc::new(SessionDB::open_ephemeral_for_test(&db_dir.join("sessions.db")).unwrap());
        let project_db = ProjectDB::new(session_db);
        project_db.migrate().unwrap();
        project_db
    }

    fn input(name: &str, root: &std::path::Path) -> CreateProjectInput {
        CreateProjectInput {
            name: name.to_string(),
            description: None,
            logo: None,
            color: None,
            default_agent_id: None,
            default_model_id: None,
            working_dir: Some(root.to_string_lossy().to_string()),
            linked_dirs: Vec::new(),
        }
    }

    #[test]
    fn project_folder_scope_parser_preserves_colons_in_windows_paths() {
        let parsed = parse_project_folder_scope(
            "session:550e8400-e29b-41d4-a716-446655440000:2:C:\\work\\api",
        )
        .unwrap();
        assert_eq!(parsed.0, "session");
        assert_eq!(parsed.1, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(parsed.2, 2);
        assert_eq!(parsed.3, "C:\\work\\api");
    }

    #[test]
    fn project_folder_scope_parser_rejects_missing_identity_or_path() {
        assert!(parse_project_folder_scope("project::0:/repo").is_err());
        assert!(parse_project_folder_scope("project:id:0:").is_err());
        assert!(parse_project_folder_scope("project:id:not-a-number:/repo").is_err());
    }

    #[test]
    fn project_create_establishes_empty_agents_md() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());

        let project =
            create_project_with_instructions_file(input("Docs", root.path()), None, true, &db)
                .expect("create project");
        let agents_md = root.path().join(PROJECT_INSTRUCTIONS_FILE);
        assert!(agents_md.is_file());
        assert_eq!(std::fs::read_to_string(agents_md).unwrap(), "");
        let canonical_root = std::fs::canonicalize(root.path()).unwrap();
        assert_eq!(project.working_dir.as_deref(), canonical_root.to_str());
    }

    #[test]
    fn project_create_saves_inspected_agents_md_draft() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());

        let inspected = inspect_project_instructions(root.path().to_str().unwrap()).unwrap();
        assert!(!root.path().join(PROJECT_INSTRUCTIONS_FILE).exists());

        let markdown = "# Project rules\n\n- Keep this on disk.\n";
        create_project_with_instructions_file(
            input("Docs", root.path()),
            Some(ProjectInstructionsDraft {
                content: markdown.to_string(),
                expected_file_hash: inspected.content_hash,
                expected_exists: Some(inspected.exists),
            }),
            true,
            &db,
        )
        .expect("create project with instructions");

        assert_eq!(
            std::fs::read_to_string(root.path().join(PROJECT_INSTRUCTIONS_FILE)).unwrap(),
            markdown
        );
    }

    #[test]
    fn project_create_can_leave_missing_agents_md_untouched() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());

        let project =
            create_project_with_instructions_file(input("Docs", root.path()), None, false, &db)
                .expect("create project without instructions file");

        update_project_with_instructions_file(
            &project.id,
            UpdateProjectInput {
                name: Some("Renamed".to_string()),
                working_dir: Some(root.path().to_string_lossy().to_string()),
                ..UpdateProjectInput::default()
            },
            None,
            &db,
        )
        .expect("metadata-only update preserves opt-out");
        db.migrate().expect("restart migration keeps opt-out");
        let inspected = inspect_project_instructions(root.path().to_str().unwrap()).unwrap();
        assert!(!inspected.exists);
        assert!(!root.path().join(PROJECT_INSTRUCTIONS_FILE).exists());
    }

    #[test]
    fn read_keeps_missing_file_absent_and_save_round_trips_markdown_exactly() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());
        let project = db.create(input("Docs", root.path())).unwrap();

        let initial = read_project_instructions(&project.id, &db).unwrap();
        assert!(!initial.exists);
        assert!(!initial.created);
        assert_eq!(initial.content, "");
        assert!(!root.path().join(PROJECT_INSTRUCTIONS_FILE).exists());

        let markdown = "# Rules\n\n- Keep trailing whitespace  \n";
        let saved = save_project_instructions(
            &project.id,
            markdown,
            &initial.content_hash,
            initial.exists,
            &db,
        )
        .unwrap();
        assert!(saved.exists);
        assert!(saved.created);
        assert_eq!(saved.content, markdown);
        assert_eq!(
            saved.content_hash,
            blake3::hash(markdown.as_bytes()).to_hex().to_string()
        );
        assert_eq!(std::fs::read_to_string(saved.path).unwrap(), markdown);
    }

    #[test]
    fn stale_save_does_not_overwrite_an_external_edit() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());
        let project =
            create_project_with_instructions_file(input("Docs", root.path()), None, true, &db)
                .expect("create project");
        let loaded = read_project_instructions(&project.id, &db).unwrap();
        let path = root.path().join(PROJECT_INSTRUCTIONS_FILE);
        std::fs::write(&path, "external edit").unwrap();

        let error = save_project_instructions(
            &project.id,
            "stale editor draft",
            &loaded.content_hash,
            loaded.exists,
            &db,
        )
        .unwrap_err();
        assert!(error
            .downcast_ref::<StaleProjectInstructionsError>()
            .is_some());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "external edit");
    }

    #[test]
    fn stale_save_does_not_recreate_an_externally_deleted_empty_file() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());
        let project =
            create_project_with_instructions_file(input("Docs", root.path()), None, true, &db)
                .expect("create project");
        let loaded = read_project_instructions(&project.id, &db).unwrap();
        let path = root.path().join(PROJECT_INSTRUCTIONS_FILE);
        std::fs::remove_file(&path).unwrap();

        let error = save_project_instructions(
            &project.id,
            "replacement",
            &loaded.content_hash,
            loaded.exists,
            &db,
        )
        .unwrap_err();

        assert!(error
            .downcast_ref::<StaleProjectInstructionsError>()
            .is_some());
        assert!(!path.exists());
    }

    #[test]
    fn stale_missing_save_does_not_overwrite_an_external_creation() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());
        let project = db.create(input("Docs", root.path())).unwrap();
        let loaded = read_project_instructions(&project.id, &db).unwrap();
        let path = root.path().join(PROJECT_INSTRUCTIONS_FILE);
        std::fs::write(&path, "external creation").unwrap();

        let error = save_project_instructions(
            &project.id,
            "editor draft",
            &loaded.content_hash,
            loaded.exists,
            &db,
        )
        .unwrap_err();

        assert!(error
            .downcast_ref::<StaleProjectInstructionsError>()
            .is_some());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "external creation");
    }

    #[test]
    fn stale_draft_rolls_back_project_metadata_update() {
        let db_dir = tempdir().unwrap();
        let root = tempdir().unwrap();
        let db = project_db(db_dir.path());
        let project =
            create_project_with_instructions_file(input("Docs", root.path()), None, true, &db)
                .expect("create project");
        let loaded = read_project_instructions(&project.id, &db).unwrap();
        std::fs::write(root.path().join(PROJECT_INSTRUCTIONS_FILE), "external edit").unwrap();

        let error = update_project_with_instructions_file(
            &project.id,
            UpdateProjectInput {
                name: Some("Renamed".to_string()),
                ..UpdateProjectInput::default()
            },
            Some(ProjectInstructionsDraft {
                content: "stale editor draft".to_string(),
                expected_file_hash: loaded.content_hash,
                expected_exists: Some(loaded.exists),
            }),
            &db,
        )
        .unwrap_err();

        assert!(error
            .downcast_ref::<StaleProjectInstructionsError>()
            .is_some());
        assert_eq!(db.get(&project.id).unwrap().unwrap().name, "Docs");
    }
}
