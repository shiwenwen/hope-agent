use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::AppContext;

fn bad_request(error: anyhow::Error) -> AppError {
    AppError::bad_request(error.to_string())
}

pub async fn get_config() -> Json<ha_pet::PetConfig> {
    Json(ha_core::config::cached_config().pet.clone())
}

#[derive(Debug, Deserialize)]
pub struct PetConfigBody {
    pub config: ha_pet::PetConfig,
}

pub async fn save_config(Json(body): Json<PetConfigBody>) -> Result<StatusCode, AppError> {
    if body.config.enabled != ha_core::config::cached_config().pet.enabled {
        return Err(overlay_unsupported().await);
    }
    ha_pet::update_config(None, Some(body.config.selected_pet_ref), "http")
        .await
        .map_err(bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetEnabledBody {
    pub enabled: bool,
}

pub async fn set_enabled(
    Json(_body): Json<PetEnabledBody>,
) -> Result<Json<ha_pet::PetConfig>, AppError> {
    Err(overlay_unsupported().await)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetActivateBody {
    pub pet_ref: ha_pet::PetRef,
}

pub async fn activate(
    Json(body): Json<PetActivateBody>,
) -> Result<Json<ha_pet::PetConfig>, AppError> {
    if !ha_core::is_desktop() {
        return Err(overlay_unsupported().await);
    }
    ha_pet::update_config(Some(true), Some(body.pet_ref), "http-pet-activate")
        .await
        .map(Json)
        .map_err(bad_request)
}

pub async fn list() -> Result<Json<ha_pet::PetLibrarySnapshot>, AppError> {
    ha_core::blocking::run_blocking(ha_pet::list_pets)
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetQuery {
    pub asset_id: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDescriptor {
    pub path: Option<String>,
    pub mime: String,
    pub etag: String,
    pub url: String,
}

pub async fn asset_descriptor(
    Query(query): Query<AssetQuery>,
) -> Result<Json<AssetDescriptor>, AppError> {
    let asset_id = query.asset_id.clone();
    let descriptor =
        ha_core::blocking::run_blocking(move || ha_pet::resolve_installed_sprite(&asset_id))
            .await
            .map_err(bad_request)?;
    Ok(Json(AssetDescriptor {
        path: None,
        mime: descriptor.mime.to_string(),
        etag: descriptor.etag,
        url: format!(
            "/api/pets/sprite?assetId={}",
            query.asset_id.replace('/', "%2F")
        ),
    }))
}

pub async fn sprite(
    Query(query): Query<AssetQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (bytes, descriptor) =
        ha_core::blocking::run_blocking(move || ha_pet::read_installed_sprite(&query.asset_id))
            .await
            .map_err(bad_request)?;
    let etag = format!("\"{}\"", descriptor.etag);
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag)
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, etag)
            .body(Body::empty())
            .map_err(|error| AppError::internal(error.to_string()));
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_str(descriptor.mime)
                .map_err(|error| AppError::internal(error.to_string()))?,
        )
        .header(ETAG, etag)
        .header(CACHE_CONTROL, "private, max-age=0, must-revalidate")
        .body(Body::from(bytes))
        .map_err(|error| AppError::internal(error.to_string()))
}

pub async fn codex_candidates() -> Result<Json<ha_pet::PetCandidatePage>, AppError> {
    ha_core::blocking::run_blocking(ha_pet::discover_codex_candidates)
        .await
        .map(Json)
        .map_err(bad_request)
}

pub async fn candidate_thumbnail(
    Path(candidate_id): Path<String>,
) -> Result<Json<Vec<u8>>, AppError> {
    ha_core::blocking::run_blocking(move || ha_pet::preview_thumbnail(&candidate_id))
        .await
        .map(Json)
        .map_err(bad_request)
}

pub async fn preview_thumbnail(
    Path(preview_token): Path<String>,
) -> Result<Json<Vec<u8>>, AppError> {
    ha_core::blocking::run_blocking(move || ha_pet::preview_token_thumbnail(&preview_token))
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
pub struct CreatePreviewBody {
    pub request: ha_pet::PetCreateRequest,
}

pub async fn create_preview(
    Json(body): Json<CreatePreviewBody>,
) -> Result<Json<ha_pet::PetImportPreview>, AppError> {
    ha_pet::create_preview(body.request)
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
pub struct UpgradeV2Body {
    pub request: ha_pet::PetUpgradeRequest,
}

pub async fn upgrade_v2(
    Json(body): Json<UpgradeV2Body>,
) -> Result<Json<ha_pet::PetUpgradeResult>, AppError> {
    ha_pet::upgrade_pet_to_v2(body.request)
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
pub struct PreviewBody {
    pub request: ha_pet::PetImportPreviewRequest,
}

pub async fn import_preview(
    Json(body): Json<PreviewBody>,
) -> Result<Json<ha_pet::PetImportPreview>, AppError> {
    if matches!(
        &body.request.source,
        ha_pet::PetImportSource::LocalPath { .. } | ha_pet::PetImportSource::LocalPaths { .. }
    ) {
        return Err(AppError::bad_request(
            "pet_local_paths_desktop_only".to_string(),
        ));
    }
    ha_pet::preview_import_async(body.request)
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelPreviewBody {
    pub preview_token: String,
}

pub async fn cancel_import_preview(
    Json(body): Json<CancelPreviewBody>,
) -> Result<Json<bool>, AppError> {
    ha_pet::cancel_import_preview(body.preview_token)
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
pub struct CommitBody {
    pub request: ha_pet::PetImportCommitRequest,
}

pub async fn import_commit(
    Json(body): Json<CommitBody>,
) -> Result<Json<ha_pet::PetImportCommitResult>, AppError> {
    if body.request.enable_after_import {
        return Err(overlay_unsupported().await);
    }
    ha_pet::commit_import(body.request)
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBody {
    pub pet_ref: ha_pet::PetRef,
    pub expected_package_hash: String,
}

pub async fn delete(
    Json(body): Json<DeleteBody>,
) -> Result<Json<ha_pet::PetDeleteResult>, AppError> {
    ha_core::blocking::run_blocking(move || {
        ha_pet::delete_pet(&body.pet_ref, &body.expected_package_hash)
    })
    .await
    .map(Json)
    .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    pub request: ha_pet::PetRestoreRequest,
}

pub async fn restore(Json(body): Json<RestoreBody>) -> Result<Json<ha_pet::PetSummary>, AppError> {
    ha_core::blocking::run_blocking(move || ha_pet::restore_pet(&body.request.restore_token))
        .await
        .map(Json)
        .map_err(bad_request)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBody {
    pub pet_ref: ha_pet::PetRef,
}

pub async fn export(
    Json(body): Json<ExportBody>,
) -> Result<Json<ha_pet::PetExportResult>, AppError> {
    ha_core::blocking::run_blocking(move || ha_pet::export_codex_package(&body.pet_ref))
        .await
        .map(Json)
        .map_err(bad_request)
}

pub async fn activity(
    State(ctx): State<Arc<AppContext>>,
) -> Result<Json<ha_pet::PetActivitySnapshot>, AppError> {
    ha_pet::activity_snapshot(ctx.session_db.clone())
        .await
        .map(Json)
        .map_err(AppError::from)
}

/// System protocol activation is owned by the desktop shell. HTTP keeps the
/// transport contract explicit without pretending a server process received
/// an OS deep link.
pub async fn pending_install_link() -> Json<Option<String>> {
    Json(None)
}

pub async fn overlay_unsupported() -> AppError {
    AppError::conflict_with_code(
        "PET_OVERLAY_DESKTOP_ONLY",
        "The pet overlay is only supported by the desktop app",
    )
}
