//! `GET /api/generated-images/{filename}` — serve AI-generated images
//! stored under `~/.opencomputer/image_generate/` so legacy `mediaUrls`
//! absolute paths resolve in HTTP mode.

use axum::extract::{Path, Request};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use oc_core::paths;

use crate::error::AppError;
use crate::routes::file_serve::{
    apply_inline_media_headers, contained_canonical, resolve_mime_for_path, validate_safe_filename,
    HeaderOpts, MimeOpts,
};

/// `GET /api/generated-images/{filename}` — binary image download.
pub async fn download(
    Path(filename): Path<String>,
    request: Request,
) -> Result<Response, AppError> {
    validate_safe_filename(&filename)?;

    let base_dir = paths::generated_images_dir().map_err(|e| AppError::internal(e.to_string()))?;
    let candidate = base_dir.join(&filename);
    let file_canon = contained_canonical(&base_dir, &candidate).await?;

    let mime = resolve_mime_for_path(&file_canon, MimeOpts::default()).await;

    let mut response = ServeFile::new(&file_canon)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve image: {}", e)))?
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
