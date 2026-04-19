use axum::Json;
use serde::Deserialize;

use ha_core::url_preview;

use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct SingleBody {
    pub url: String,
}

/// `POST /api/url-preview`
pub async fn fetch_url_preview(
    Json(body): Json<SingleBody>,
) -> Result<Json<url_preview::UrlPreviewMeta>, AppError> {
    Ok(Json(url_preview::fetch_preview(&body.url).await?))
}

#[derive(Debug, Deserialize)]
pub struct BatchBody {
    pub urls: Vec<String>,
}

/// `POST /api/url-preview/batch`
pub async fn fetch_url_previews(
    Json(body): Json<BatchBody>,
) -> Result<Json<Vec<url_preview::UrlPreviewMeta>>, AppError> {
    let handles: Vec<_> = body
        .urls
        .into_iter()
        .map(|url| tokio::spawn(async move { url_preview::fetch_preview(&url).await }))
        .collect();
    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Ok(meta)) = handle.await {
            results.push(meta);
        }
    }
    Ok(Json(results))
}
