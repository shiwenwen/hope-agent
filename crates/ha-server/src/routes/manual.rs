//! Built-in user manual (Help Center) — thin shells over `ha_core::manual`.

use axum::extract::Query;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use ha_core::blocking::run_blocking;
use ha_core::manual::{ManualBundle, ManualSearchHit};

#[derive(Deserialize)]
pub struct BundleQuery {
    pub lang: Option<String>,
}

/// `GET /api/manual/bundle?lang=`
pub async fn get_bundle(Query(q): Query<BundleQuery>) -> Result<Json<ManualBundle>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::manual::bundle_for_command(q.lang.as_deref())).await,
    ))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub lang: Option<String>,
    /// Named `query` (not `q`) so the HTTP transport can forward the Tauri
    /// command args verbatim as query params.
    pub query: String,
}

/// `GET /api/manual/search?lang=&query=`
pub async fn search(Query(q): Query<SearchQuery>) -> Result<Json<Vec<ManualSearchHit>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::manual::search_for_command(q.lang.as_deref(), &q.query))
            .await,
    ))
}
