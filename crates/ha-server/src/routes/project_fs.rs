//! HTTP handlers for the project file browser (`/api/fs/*`).
//!
//! Thin axum adapters over [`ha_core::filesystem`]; all containment lives in
//! `WorkspaceScope`. Read endpoints (`list` / `read` / `extract` / `raw`) are
//! always available; **write** endpoints are gated behind
//! `filesystem.allow_remote_writes` (default off) so a remote token-bearer
//! cannot modify the server host's files unless the operator opts in. The
//! desktop (Tauri IPC) bypasses this gate entirely.

use axum::body::Body;
use axum::extract::{Multipart, Query, State};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::file_serve::{apply_inline_media_headers, resolve_mime_for_path, HeaderOpts, MimeOpts};
use super::helpers::parse_file_upload;
use crate::error::AppError;
use crate::AppContext;
use ha_core::filesystem::{
    self, ExtractedContent, FileTextContent, FilesystemError, RenameResult, UploadResult,
    WorkspaceListing, WorkspaceScope, WriteResult,
};

fn map_err(e: FilesystemError) -> AppError {
    if e.is_bad_input() {
        AppError::bad_request(e.message().to_string())
    } else {
        AppError::internal(e.message().to_string())
    }
}

/// Reject writes over HTTP unless the operator enabled remote writes.
fn ensure_writes_allowed() -> Result<(), AppError> {
    if ha_core::config::cached_config()
        .filesystem
        .allow_remote_writes
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "remote file writes are disabled; enable filesystem.allowRemoteWrites to allow them",
        ))
    }
}

async fn run<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> filesystem::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("fs task failed: {e}")))?
        .map_err(map_err)
}

/// `/`-relative parent of a relative path.
fn parent_rel(rel: &str) -> String {
    match rel.trim_end_matches('/').rsplit_once('/') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    }
}

fn emit_changed(ctx: &AppContext, scope: &str, scope_id: &str, dir: String) {
    ctx.event_bus.emit(
        "project:fs_changed",
        json!({ "scope": scope, "scopeId": scope_id, "dir": dir }),
    );
}

// ── Read endpoints ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopePathQuery {
    pub scope: String,
    pub scope_id: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// `GET /api/fs/list?scope=&scopeId=&path=`
pub async fn fs_list(Query(q): Query<ScopePathQuery>) -> Result<Json<WorkspaceListing>, AppError> {
    let ScopePathQuery {
        scope,
        scope_id,
        path,
    } = q;
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_list_dir(&s, path.as_deref().unwrap_or(""))
    })
    .await?;
    Ok(Json(res))
}

/// `GET /api/fs/read?scope=&scopeId=&path=`
pub async fn fs_read(Query(q): Query<ScopePathQuery>) -> Result<Json<FileTextContent>, AppError> {
    let ScopePathQuery {
        scope,
        scope_id,
        path,
    } = q;
    let path = path.unwrap_or_default();
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_read_text(&s, &path)
    })
    .await?;
    Ok(Json(res))
}

/// `GET /api/fs/extract?scope=&scopeId=&path=`
pub async fn fs_extract(
    Query(q): Query<ScopePathQuery>,
) -> Result<Json<ExtractedContent>, AppError> {
    let ScopePathQuery {
        scope,
        scope_id,
        path,
    } = q;
    let path = path.unwrap_or_default();
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_fs_extract(&s, &path)
    })
    .await?;
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawQuery {
    pub scope: String,
    pub scope_id: String,
    pub path: String,
    #[serde(default)]
    pub download: Option<u8>,
}

/// `GET /api/fs/raw?scope=&scopeId=&path=&download=` — serve raw bytes inline
/// (images / PDFs / any file) for the preview pane.
pub async fn fs_raw(Query(q): Query<RawQuery>) -> Result<Response, AppError> {
    let RawQuery {
        scope,
        scope_id,
        path,
        download,
    } = q;
    let abs = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        s.resolve_existing(&path)
    })
    .await?;
    let bytes = tokio::fs::read(&abs)
        .await
        .map_err(|e| AppError::internal(format!("read file: {e}")))?;
    let mime = resolve_mime_for_path(
        &abs,
        MimeOpts {
            html_charset: false,
            sniff_fallback: true,
        },
    )
    .await;
    let disposition = if download.unwrap_or(0) == 1 {
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "download".to_string());
        format!("attachment; filename=\"{}\"", name.replace('"', ""))
    } else {
        "inline".to_string()
    };
    let mut response = Response::new(Body::from(bytes));
    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &mime,
            cache_secs: 0,
            disposition: &disposition,
            no_referrer: false,
        },
    );
    Ok(response)
}

// ── Write endpoints (gated) ─────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTextBody {
    pub scope: String,
    pub scope_id: String,
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub create_only: Option<bool>,
}

/// `PUT /api/fs/file`
pub async fn fs_write(
    State(ctx): State<Arc<AppContext>>,
    Json(b): Json<WriteTextBody>,
) -> Result<Json<WriteResult>, AppError> {
    ensure_writes_allowed()?;
    let WriteTextBody {
        scope,
        scope_id,
        path,
        content,
        create_only,
    } = b;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_write_text(&s, &path, &content, create_only.unwrap_or(false))
    })
    .await?;
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path));
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQuery {
    pub scope: String,
    pub scope_id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: Option<bool>,
}

/// `DELETE /api/fs/entry?scope=&scopeId=&path=&recursive=`
pub async fn fs_delete(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<DeleteQuery>,
) -> Result<Json<Value>, AppError> {
    ensure_writes_allowed()?;
    let DeleteQuery {
        scope,
        scope_id,
        path,
        recursive,
    } = q;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let dir = parent_rel(&path);
    run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_delete(&s, &path, recursive.unwrap_or(false))
    })
    .await?;
    emit_changed(&ctx, &es, &ei, dir);
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameBody {
    pub scope: String,
    pub scope_id: String,
    pub from_path: String,
    pub to_path: String,
    #[serde(default)]
    pub overwrite: Option<bool>,
}

/// `POST /api/fs/rename`
pub async fn fs_rename(
    State(ctx): State<Arc<AppContext>>,
    Json(b): Json<RenameBody>,
) -> Result<Json<RenameResult>, AppError> {
    ensure_writes_allowed()?;
    let RenameBody {
        scope,
        scope_id,
        from_path,
        to_path,
        overwrite,
    } = b;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let from_dir = parent_rel(&from_path);
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_rename(&s, &from_path, &to_path, overwrite.unwrap_or(false))
    })
    .await?;
    emit_changed(&ctx, &es, &ei, from_dir);
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path));
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MkdirBody {
    pub scope: String,
    pub scope_id: String,
    pub path: String,
}

/// `POST /api/fs/mkdir`
pub async fn fs_mkdir(
    State(ctx): State<Arc<AppContext>>,
    Json(b): Json<MkdirBody>,
) -> Result<Json<WriteResult>, AppError> {
    ensure_writes_allowed()?;
    let MkdirBody {
        scope,
        scope_id,
        path,
    } = b;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_mkdir(&s, &path)
    })
    .await?;
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path));
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadQuery {
    pub scope: String,
    pub scope_id: String,
    #[serde(default)]
    pub dir_path: Option<String>,
    #[serde(default)]
    pub overwrite: Option<bool>,
}

/// `POST /api/fs/upload?scope=&scopeId=&dirPath=&overwrite=` — multipart file.
pub async fn fs_upload(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<UploadQuery>,
    multipart: Multipart,
) -> Result<Json<UploadResult>, AppError> {
    ensure_writes_allowed()?;
    let parsed = parse_file_upload(multipart).await?;
    let UploadQuery {
        scope,
        scope_id,
        dir_path,
        overwrite,
    } = q;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let dir = dir_path.unwrap_or_default();
    let file_name = parsed.file_name;
    let data = parsed.file_data;
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::project_upload(&s, &dir, &file_name, &data, overwrite.unwrap_or(false))
    })
    .await?;
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path));
    Ok(Json(res))
}
