//! Tauri commands for the project file browser.
//!
//! Thin wrappers over [`ha_core::filesystem`] — all containment / validation
//! lives in `WorkspaceScope`. Every op runs on `spawn_blocking` (sync `std::fs`)
//! and is keyed by a `(scope, scopeId)` pair: `"session"` resolves the session's
//! effective working dir, `"project"` resolves the project workspace.
//!
//! Desktop (Tauri IPC) is always allowed to write; the HTTP transport gates
//! writes behind `filesystem.allow_remote_writes` (see the ha-server routes).

use crate::commands::CmdError;
use ha_core::filesystem::{
    self, ExtractedContent, FileSearchResponse, FileTextContent, GitInfo, RenameResult,
    UploadResult, WorkspaceListing, WorkspaceScope, WriteResult,
};

/// Run a blocking filesystem closure off the async runtime, mapping
/// `FilesystemError` to a flat `CmdError` string for the UI.
async fn blocking<T, F>(f: F) -> Result<T, CmdError>
where
    F: FnOnce() -> filesystem::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::msg(format!("fs task join error: {e}")))?
        .map_err(|e| CmdError::msg(e.message().to_string()))
}

/// Emit `project:fs_changed` so every open file-browser view refreshes the
/// affected directory. `dir` is the `/`-relative parent of the changed path.
fn emit_fs_changed(scope: &str, scope_id: &str, dir: &str) {
    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:fs_changed",
            serde_json::json!({ "scope": scope, "scopeId": scope_id, "dir": dir }),
        );
    }
}

/// `/`-relative parent of a relative path (`"a/b/c.txt"` → `"a/b"`, `"x"` → `""`).
fn parent_rel(rel: &str) -> String {
    match rel.trim_end_matches('/').rsplit_once('/') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    }
}

// ── Read ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn project_fs_list(
    scope: String,
    scope_id: String,
    path: Option<String>,
) -> Result<WorkspaceListing, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_list_dir(&s, path.as_deref().unwrap_or(""))
    })
    .await
}

#[tauri::command]
pub async fn project_fs_read_text(
    scope: String,
    scope_id: String,
    path: String,
) -> Result<FileTextContent, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_read_text(&s, &path)
    })
    .await
}

#[tauri::command]
pub async fn project_fs_extract(
    scope: String,
    scope_id: String,
    path: String,
) -> Result<ExtractedContent, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_fs_extract(&s, &path)
    })
    .await
}

#[tauri::command]
pub async fn project_fs_search(
    scope: String,
    scope_id: String,
    q: String,
    limit: Option<usize>,
) -> Result<FileSearchResponse, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::search_files(&s.root().to_string_lossy(), &q, limit)
    })
    .await
}

// ── Preview by absolute path (file-operations unification) ──────
//
// Read / extract an arbitrary local file for the in-app preview panel, by
// absolute path (not scope-relative). Desktop is the local machine, so reading
// any path the user can already reach is consistent with `open_directory`
// (which opens arbitrary paths). The HTTP transport has no equivalent unguarded
// command — it gates the same reads behind session authorization in ha-server's
// `/sessions/{id}/files/{read,extract}` endpoints.

#[tauri::command]
pub async fn preview_read_text(path: String) -> Result<FileTextContent, CmdError> {
    // Expand `~/` like `open_directory` does, so `~/`-prefixed Markdown links
    // preview instead of failing on a literal `~` path component.
    let path = crate::commands::misc::resolve_user_path(path);
    blocking(move || ha_core::filesystem::read_text_abs(std::path::Path::new(&path))).await
}

#[tauri::command]
pub async fn preview_extract(path: String) -> Result<ExtractedContent, CmdError> {
    let path = crate::commands::misc::resolve_user_path(path);
    blocking(move || ha_core::filesystem::extract_abs(std::path::Path::new(&path))).await
}

/// Resolve a workspace-relative path to its canonical absolute path. Desktop
/// only — used to feed `convertFileSrc` for image/PDF preview. (HTTP has no
/// equivalent: an absolute server path is meaningless to a remote browser.)
#[tauri::command]
pub async fn project_fs_resolve(
    scope: String,
    scope_id: String,
    path: String,
) -> Result<String, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        let abs = s.resolve_existing(&path)?;
        Ok(abs.to_string_lossy().to_string())
    })
    .await
}

/// Read-only git branch + worktree list for the scope's working dir. `None`
/// when the dir is not inside a git work tree.
#[tauri::command]
pub async fn project_git_info(
    scope: String,
    scope_id: String,
) -> Result<Option<GitInfo>, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        Ok(filesystem::git_info(s.root()))
    })
    .await
}

// ── Write ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn project_fs_write_text(
    scope: String,
    scope_id: String,
    path: String,
    content: String,
    create_only: Option<bool>,
) -> Result<WriteResult, CmdError> {
    let (s_scope, s_id) = (scope.clone(), scope_id.clone());
    let res = blocking(move || {
        let s = WorkspaceScope::resolve_writable(&scope, &scope_id)?;
        filesystem::project_write_text(&s, &path, &content, create_only.unwrap_or(false))
    })
    .await?;
    emit_fs_changed(&s_scope, &s_id, &parent_rel(&res.rel_path));
    Ok(res)
}

#[tauri::command]
pub async fn project_fs_delete(
    scope: String,
    scope_id: String,
    path: String,
    recursive: Option<bool>,
) -> Result<(), CmdError> {
    let (s_scope, s_id) = (scope.clone(), scope_id.clone());
    let dir = parent_rel(&path);
    blocking(move || {
        let s = WorkspaceScope::resolve_writable(&scope, &scope_id)?;
        filesystem::project_delete(&s, &path, recursive.unwrap_or(false))
    })
    .await?;
    emit_fs_changed(&s_scope, &s_id, &dir);
    Ok(())
}

#[tauri::command]
pub async fn project_fs_rename(
    scope: String,
    scope_id: String,
    from_path: String,
    to_path: String,
    overwrite: Option<bool>,
) -> Result<RenameResult, CmdError> {
    let (s_scope, s_id) = (scope.clone(), scope_id.clone());
    let from_dir = parent_rel(&from_path);
    let res = blocking(move || {
        let s = WorkspaceScope::resolve_writable(&scope, &scope_id)?;
        filesystem::project_rename(&s, &from_path, &to_path, overwrite.unwrap_or(false))
    })
    .await?;
    // Both source and destination directories may have changed.
    emit_fs_changed(&s_scope, &s_id, &from_dir);
    emit_fs_changed(&s_scope, &s_id, &parent_rel(&res.rel_path));
    Ok(res)
}

#[tauri::command]
pub async fn project_fs_mkdir(
    scope: String,
    scope_id: String,
    path: String,
) -> Result<WriteResult, CmdError> {
    let (s_scope, s_id) = (scope.clone(), scope_id.clone());
    let res = blocking(move || {
        let s = WorkspaceScope::resolve_writable(&scope, &scope_id)?;
        filesystem::project_mkdir(&s, &path)
    })
    .await?;
    emit_fs_changed(&s_scope, &s_id, &parent_rel(&res.rel_path));
    Ok(res)
}

#[tauri::command]
pub async fn project_fs_upload(
    scope: String,
    scope_id: String,
    dir_path: String,
    file_name: String,
    data: Vec<u8>,
    overwrite: Option<bool>,
) -> Result<UploadResult, CmdError> {
    let (s_scope, s_id) = (scope.clone(), scope_id.clone());
    let res = blocking(move || {
        let s = WorkspaceScope::resolve_writable(&scope, &scope_id)?;
        filesystem::project_upload(&s, &dir_path, &file_name, &data, overwrite.unwrap_or(false))
    })
    .await?;
    emit_fs_changed(&s_scope, &s_id, &parent_rel(&res.rel_path));
    Ok(res)
}
