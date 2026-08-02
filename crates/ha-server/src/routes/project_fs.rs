//! HTTP handlers for the project file browser (`/api/fs/*`).
//!
//! Thin axum adapters over [`ha_core::filesystem`]; all containment lives in
//! `WorkspaceScope`. Read endpoints (`list` / `read` / `extract` / `raw`) are
//! always available; **write** endpoints are gated behind
//! `filesystem.allow_remote_writes` (default off) so a remote token-bearer
//! cannot modify the server host's files unless the operator opts in. The
//! desktop (Tauri IPC) bypasses this gate entirely.

use axum::body::{Body, Bytes};
use axum::extract::{Extension, Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use super::file_serve::{
    apply_inline_media_headers, resolve_mime_for_path, safe_content_disposition, HeaderOpts,
    MimeOpts,
};
use super::helpers::parse_file_upload_to_temp;
use crate::error::AppError;
use crate::middleware::{AuthState, BoundFile};
use crate::AppContext;
use ha_core::filesystem::{
    self, ExtractedContent, FileSearchResponse, FileTextContent, FileWriteOutcome, FilesystemError,
    GitInfo, RenameResult, UploadResult, WorkspaceAccess, WorkspaceListing, WorkspaceScope,
    WriteResult,
};

fn map_err(e: FilesystemError) -> AppError {
    if e.is_forbidden() {
        AppError::forbidden(e.message().to_string())
    } else if e.is_bad_input() {
        AppError::bad_request(e.message().to_string())
    } else {
        AppError::internal(e.message().to_string())
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

fn emit_changed(ctx: &AppContext, scope: &str, scope_id: &str, dir: String, path: &str) {
    ctx.event_bus.emit(
        "project:fs_changed",
        json!({ "scope": scope, "scopeId": scope_id, "dir": dir, "path": path }),
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeQuery {
    pub scope: String,
    pub scope_id: String,
}

/// `GET /api/fs/capabilities?scope=&scopeId=`
pub async fn fs_capabilities(
    Query(q): Query<ScopeQuery>,
) -> Result<Json<WorkspaceAccess>, AppError> {
    let ScopeQuery { scope, scope_id } = q;
    Ok(Json(
        run(move || WorkspaceScope::access(&scope, &scope_id)).await?,
    ))
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
pub struct SearchQuery {
    pub scope: String,
    pub scope_id: String,
    pub q: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/fs/search?scope=&scopeId=&q=&limit=` — fuzzy search files and
/// directories under a project/session/worktree scope.
pub async fn fs_search(Query(q): Query<SearchQuery>) -> Result<Json<FileSearchResponse>, AppError> {
    let SearchQuery {
        scope,
        scope_id,
        q,
        limit,
    } = q;
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        filesystem::search_files(&s.root().to_string_lossy(), &q, limit)
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTicketBody {
    pub scope: String,
    pub scope_id: String,
    pub path: String,
    #[serde(default)]
    pub download: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawTicketResponse {
    ticket: String,
    expires_in_secs: u64,
}

pub(super) const BOUND_FILE_TICKET_TTL_SECS: u64 = 15 * 60;
const BOUND_FILE_STREAM_CHUNK_BYTES: usize = 64 * 1024;

async fn resolve_raw_path(
    scope: String,
    scope_id: String,
    path: String,
) -> Result<std::path::PathBuf, AppError> {
    run(move || {
        let resolved = WorkspaceScope::resolve(&scope, &scope_id)?;
        resolved.resolve_existing(&path)
    })
    .await
}

async fn serve_raw_path(
    abs: std::path::PathBuf,
    download: bool,
    request: Request,
) -> Result<Response, AppError> {
    let mime = resolve_mime_for_path(
        &abs,
        MimeOpts {
            html_charset: false,
            sniff_fallback: true,
        },
    )
    .await;
    let disposition = safe_content_disposition(&abs, &mime, download);
    // Stream via ServeFile (Range-capable, memory-bounded) rather than buffering
    // the whole file into a Vec — a large file would otherwise spike RSS / OOM.
    let mut response = ServeFile::new(&abs)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve file: {e}")))?
        .into_response();
    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &mime,
            cache_secs: 0,
            disposition: &disposition,
            no_referrer: false,
        },
    );
    // Defense in depth: stop content-type sniffing from upgrading a mislabeled
    // file (e.g. a `.txt` whose bytes look like HTML) into active content in
    // the app origin.
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HttpByteRange {
    start: u64,
    len: u64,
}

fn parse_single_byte_range(value: &str, file_len: u64) -> Option<HttpByteRange> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') || file_len == 0 {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        let len = suffix.min(file_len);
        return Some(HttpByteRange {
            start: file_len - len,
            len,
        });
    }

    let start = start.parse::<u64>().ok()?;
    if start >= file_len {
        return None;
    }
    let end = if end.is_empty() {
        file_len - 1
    } else {
        end.parse::<u64>().ok()?.min(file_len - 1)
    };
    if end < start {
        return None;
    }
    Some(HttpByteRange {
        start,
        len: end - start + 1,
    })
}

#[cfg(unix)]
fn read_bound_file_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn read_bound_file_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

fn bound_file_body(file: Arc<std::fs::File>, range: HttpByteRange) -> Body {
    let chunks = stream::try_unfold(
        (file, range.start, range.len),
        |(file, offset, remaining)| async move {
            if remaining == 0 {
                return Ok::<_, io::Error>(None);
            }
            let wanted = remaining.min(BOUND_FILE_STREAM_CHUNK_BYTES as u64) as usize;
            let read_file = file.clone();
            let (mut bytes, read) = tokio::task::spawn_blocking(move || {
                let mut bytes = vec![0_u8; wanted];
                let read = read_bound_file_at(&read_file, &mut bytes, offset)?;
                Ok::<_, io::Error>((bytes, read))
            })
            .await
            .map_err(|error| io::Error::other(format!("bound file-read task failed: {error}")))??;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "bound file changed length while streaming",
                ));
            }
            bytes.truncate(read);
            Ok(Some((
                Bytes::from(bytes),
                (file, offset + read as u64, remaining - read as u64),
            )))
        },
    );
    Body::from_stream(chunks)
}

fn serve_bound_raw_file(bound: BoundFile, request: Request) -> Result<Response, AppError> {
    let requested_range = request.headers().get(header::RANGE);
    let range = match requested_range {
        Some(value) => {
            let parsed = value
                .to_str()
                .ok()
                .and_then(|value| parse_single_byte_range(value, bound.len));
            let Some(range) = parsed else {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header(header::CONTENT_RANGE, format!("bytes */{}", bound.len))
                    .header(header::CACHE_CONTROL, "private, max-age=0")
                    .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                    .body(Body::empty())
                    .map_err(|error| {
                        AppError::internal(format!("build invalid-range response: {error}"))
                    });
            };
            range
        }
        None => HttpByteRange {
            start: 0,
            len: bound.len,
        },
    };

    let status = if requested_range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let body = if request.method() == Method::HEAD || range.len == 0 {
        Body::empty()
    } else {
        bound_file_body(bound.file.clone(), range)
    };
    let mut response = Response::builder()
        .status(status)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, range.len.to_string())
        .body(body)
        .map_err(|error| AppError::internal(format!("build bound file response: {error}")))?;
    if status == StatusCode::PARTIAL_CONTENT {
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!(
                "bytes {}-{}/{}",
                range.start,
                range.start + range.len - 1,
                bound.len
            ))
            .map_err(|error| AppError::internal(error.to_string()))?,
        );
    }
    let disposition = safe_content_disposition(&bound.path, &bound.mime, bound.download);
    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &bound.mime,
            cache_secs: 0,
            disposition: &disposition,
            no_referrer: false,
        },
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// `POST /api/fs/raw-ticket` — resolve authorization once and bind a short-lived
/// iframe-safe capability to that exact canonical file.
pub async fn create_fs_raw_ticket(
    Extension(auth): Extension<AuthState>,
    Json(body): Json<RawTicketBody>,
) -> Result<Response, AppError> {
    let abs = resolve_raw_path(body.scope, body.scope_id, body.path).await?;
    let ticket = auth
        .create_bound_file_ticket(abs, body.download, BOUND_FILE_TICKET_TTL_SECS)
        .await
        .map_err(|error| {
            ha_core::app_error!(
                "security",
                "workspace_raw_ticket_mint_failed",
                "Failed to mint a bound workspace raw ticket: {}",
                error
            );
            AppError::internal("Workspace preview is unavailable")
        })?;
    Ok(super::auth::no_store_json(
        StatusCode::OK,
        &RawTicketResponse {
            ticket,
            expires_in_secs: BOUND_FILE_TICKET_TTL_SECS,
        },
    ))
}

/// Public capability endpoint for one canonical authorized file. Query strings
/// are deliberately ignored: the path and disposition are server-side state.
pub async fn fs_raw_with_ticket(
    Path(ticket): Path<String>,
    Extension(auth): Extension<AuthState>,
    request: Request,
) -> Response {
    let Some(bound) = auth.resolve_bound_file_ticket(&ticket) else {
        return super::auth::no_store_json(
            StatusCode::UNAUTHORIZED,
            &json!({ "error": "Invalid or expired file preview ticket" }),
        );
    };
    match serve_bound_raw_file(bound, request) {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

/// `GET /api/fs/raw?scope=&scopeId=&path=&download=` — serve raw bytes inline
/// (images / PDFs / any file) for the preview pane.
pub async fn fs_raw(Query(q): Query<RawQuery>, request: Request) -> Result<Response, AppError> {
    let RawQuery {
        scope,
        scope_id,
        path,
        download,
    } = q;
    let abs = resolve_raw_path(scope, scope_id, path).await?;
    serve_raw_path(abs, download.unwrap_or(0) == 1, request).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeOnlyQuery {
    pub scope: String,
    pub scope_id: String,
}

/// `GET /api/fs/git?scope=&scopeId=` — read-only git branch + worktree list for
/// the scope's working dir. `null` when it is not inside a git work tree.
pub async fn fs_git_info(
    Query(q): Query<ScopeOnlyQuery>,
) -> Result<Json<Option<GitInfo>>, AppError> {
    let ScopeOnlyQuery { scope, scope_id } = q;
    let res = run(move || {
        let s = WorkspaceScope::resolve(&scope, &scope_id)?;
        Ok(filesystem::git_info(s.root()))
    })
    .await?;
    Ok(Json(res))
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
    #[serde(default)]
    pub expected_file_hash: Option<String>,
}

/// `PUT /api/fs/file`
pub async fn fs_write(
    State(ctx): State<Arc<AppContext>>,
    Json(b): Json<WriteTextBody>,
) -> Result<Json<FileWriteOutcome>, AppError> {
    let WriteTextBody {
        scope,
        scope_id,
        path,
        content,
        create_only,
        expected_file_hash,
    } = b;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let res = run(move || {
        let s = WorkspaceScope::resolve_effective_writable(&scope, &scope_id)?;
        filesystem::project_write_text_checked(
            &s,
            &path,
            &content,
            create_only.unwrap_or(false),
            expected_file_hash.as_deref(),
        )
    })
    .await?;
    if let FileWriteOutcome::Saved { ref rel_path, .. } = res {
        emit_changed(&ctx, &es, &ei, parent_rel(rel_path), rel_path);
    }
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
    let DeleteQuery {
        scope,
        scope_id,
        path,
        recursive,
    } = q;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let dir = parent_rel(&path);
    let changed_path = path.clone();
    run(move || {
        let s = WorkspaceScope::resolve_effective_writable(&scope, &scope_id)?;
        filesystem::project_delete(&s, &path, recursive.unwrap_or(false))
    })
    .await?;
    emit_changed(&ctx, &es, &ei, dir, &changed_path);
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
    let RenameBody {
        scope,
        scope_id,
        from_path,
        to_path,
        overwrite,
    } = b;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let from_dir = parent_rel(&from_path);
    let changed_from_path = from_path.clone();
    let res = run(move || {
        let s = WorkspaceScope::resolve_effective_writable(&scope, &scope_id)?;
        filesystem::project_rename(&s, &from_path, &to_path, overwrite.unwrap_or(false))
    })
    .await?;
    emit_changed(&ctx, &es, &ei, from_dir, &changed_from_path);
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path), &res.rel_path);
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
    let MkdirBody {
        scope,
        scope_id,
        path,
    } = b;
    let (es, ei) = (scope.clone(), scope_id.clone());
    let res = run(move || {
        let s = WorkspaceScope::resolve_effective_writable(&scope, &scope_id)?;
        filesystem::project_mkdir(&s, &path)
    })
    .await?;
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path), &res.rel_path);
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
    let UploadQuery {
        scope,
        scope_id,
        dir_path,
        overwrite,
    } = q;
    let (es, ei) = (scope.clone(), scope_id.clone());
    // Reject disabled/readonly remote writes before consuming the upload body.
    let resolved_scope =
        run(move || WorkspaceScope::resolve_effective_writable(&scope, &scope_id)).await?;
    let max_upload_bytes = ha_core::config::cached_config()
        .filesystem
        .max_workspace_upload_bytes()
        .min(ha_core::filesystem::LEGACY_MAX_WORKSPACE_UPLOAD_BYTES)
        as usize;
    let parsed = parse_file_upload_to_temp(multipart, max_upload_bytes).await?;
    let dir = dir_path.unwrap_or_default();
    let file_name = parsed.file_name;
    let file_path = parsed.file_path;
    let res = run(move || {
        filesystem::project_upload_file(
            &resolved_scope,
            &dir,
            &file_name,
            file_path.as_ref(),
            overwrite.unwrap_or(false),
        )
    })
    .await?;
    emit_changed(&ctx, &es, &ei, parent_rel(&res.rel_path), &res.rel_path);
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimUploadBody {
    pub scope: String,
    pub scope_id: String,
    #[serde(default)]
    pub dir_path: String,
    pub upload_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

/// `POST /api/fs/upload-claim` — claim an opaque `workspace_upload` lease.
pub async fn fs_claim_upload(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<ClaimUploadBody>,
) -> Result<Json<UploadResult>, AppError> {
    let ClaimUploadBody {
        scope,
        scope_id,
        dir_path,
        upload_id,
        file_name,
        overwrite,
    } = body;
    let (event_scope, event_id) = (scope.clone(), scope_id.clone());
    let result = run(move || {
        let resolved = WorkspaceScope::resolve_effective_writable(&scope, &scope_id)?;
        filesystem::project_claim_upload(
            &resolved,
            &dir_path,
            &upload_id,
            file_name.as_deref(),
            overwrite,
        )
    })
    .await?;
    emit_changed(
        &ctx,
        &event_scope,
        &event_id,
        parent_rel(&result.rel_path),
        &result.rel_path,
    );
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;

    #[tokio::test]
    async fn workspace_raw_capability_serves_only_its_server_bound_file() {
        let dir = tempfile::tempdir().unwrap();
        let visible = dir.path().join("visible.html");
        let secret = dir.path().join("secret.js");
        std::fs::write(&visible, "visible content").unwrap();
        std::fs::write(&secret, "secret content").unwrap();

        let auth = AuthState::new(Some("owner-token".into()), None, false);
        let visible_canonical = std::fs::canonicalize(&visible).unwrap();
        let bound_ticket = auth
            .create_bound_file_ticket(visible_canonical, true, 900)
            .await
            .unwrap();
        let generic_ticket = auth.create_access_ticket("resources", 900).unwrap();

        // The workspace can remain agent-writable after capability minting.
        // Replacing the authorized path must not make this ticket follow the
        // new symlink; it stays pinned to the already-open original file.
        #[cfg(unix)]
        {
            std::fs::remove_file(&visible).unwrap();
            std::os::unix::fs::symlink(&secret, &visible).unwrap();
        }
        let app = Router::new()
            .route("/api/resource/{ticket}/fs/raw", get(fs_raw_with_ticket))
            .route(
                "/api/resource/{ticket}/{*path}",
                get(|| async { StatusCode::IM_A_TEAPOT }),
            )
            .layer(Extension(auth));

        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!(
                        "/api/resource/{bound_ticket}/fs/raw?path={}&download=0",
                        secret.display()
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get(axum::http::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("attachment;")));
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "visible content"
        );

        let ranged = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/api/resource/{bound_ticket}/fs/raw"))
                    .header(axum::http::header::RANGE, "bytes=8-14")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            ranged
                .headers()
                .get(axum::http::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok()),
            Some("bytes 8-14/15")
        );
        assert_eq!(to_bytes(ranged.into_body(), 1024).await.unwrap(), "content");

        let rejected = app
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/api/resource/{generic_ticket}/fs/raw"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }
}
