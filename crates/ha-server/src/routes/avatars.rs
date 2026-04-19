//! `GET /api/avatars/{filename}` + `POST /api/avatars` — serve and
//! upload user / agent avatar images under `~/.hope-agent/avatars/`.

use axum::extract::{Multipart, Path, Request};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use ha_core::paths;

use crate::error::AppError;
use crate::routes::file_serve::{
    apply_inline_media_headers, contained_canonical, resolve_mime_for_path, validate_safe_filename,
    HeaderOpts, MimeOpts,
};
use crate::routes::helpers::parse_file_upload;

/// `GET /api/avatars/{filename}` — binary image download.
pub async fn download(
    Path(filename): Path<String>,
    request: Request,
) -> Result<Response, AppError> {
    validate_safe_filename(&filename)?;

    let base_dir = paths::avatars_dir().map_err(|e| AppError::internal(e.to_string()))?;
    let candidate = base_dir.join(&filename);
    let file_canon = contained_canonical(&base_dir, &candidate).await?;

    let mime = resolve_mime_for_path(&file_canon, MimeOpts::default()).await;

    let mut response = ServeFile::new(&file_canon)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve avatar: {}", e)))?
        .into_response();

    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &mime,
            cache_secs: 300,
            disposition: "inline",
            no_referrer: false,
        },
    );

    Ok(response)
}

/// `POST /api/avatars` — multipart upload. Returns the absolute on-disk
/// path so the frontend can drop it into `AgentConfig.avatar` /
/// `UserConfig.avatar`, matching the Tauri `save_avatar` return contract.
pub async fn upload(multipart: Multipart) -> Result<Json<Value>, AppError> {
    let upload = parse_file_upload(multipart).await?;
    validate_safe_filename(&upload.file_name)?;

    let dir = paths::avatars_dir().map_err(|e| AppError::internal(e.to_string()))?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::internal(format!("create avatars dir: {}", e)))?;

    let path = dir.join(&upload.file_name);
    tokio::fs::write(&path, &upload.file_data)
        .await
        .map_err(|e| AppError::internal(format!("write avatar: {}", e)))?;

    Ok(Json(json!({ "path": path.to_string_lossy().to_string() })))
}
