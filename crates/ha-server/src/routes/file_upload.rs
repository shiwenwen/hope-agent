use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use ha_core::file_upload::{FileUploadLease, FileUploadStartInput};

fn internal(error: anyhow::Error) -> AppError {
    AppError::bad_request(error.to_string())
}

pub async fn start(
    Json(input): Json<FileUploadStartInput>,
) -> Result<Json<FileUploadLease>, AppError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::start_upload(input))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map(Json)
        .map_err(internal)
}

pub async fn status(Path(upload_id): Path<String>) -> Result<Json<FileUploadLease>, AppError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::upload_status(&upload_id))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map(Json)
        .map_err(internal)
}

#[derive(Debug, Deserialize)]
pub struct ChunkQuery {
    pub offset: u64,
}

pub async fn chunk(
    Path(upload_id): Path<String>,
    Query(query): Query<ChunkQuery>,
    body: Bytes,
) -> Result<Json<FileUploadLease>, AppError> {
    if body.len() > ha_core::file_upload::FILE_UPLOAD_CHUNK_BYTES {
        return Err(AppError::bad_request("upload chunk exceeds 4 MiB"));
    }
    let data = body.to_vec();
    tokio::task::spawn_blocking(move || {
        ha_core::file_upload::upload_chunk(&upload_id, query.offset, &data)
    })
    .await
    .map_err(|error| AppError::internal(error.to_string()))?
    .map(Json)
    .map_err(internal)
}

pub async fn complete(Path(upload_id): Path<String>) -> Result<Json<FileUploadLease>, AppError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::complete_upload(&upload_id))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map(Json)
        .map_err(internal)
}

pub async fn discard(Path(upload_id): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::discard_upload(&upload_id))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
