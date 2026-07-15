use std::path::PathBuf;

use axum::extract::{Path, Query, Request};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use ha_core::artifacts::{
    ArtifactKind, ArtifactService, CreateArtifactInput, ListArtifactsInput, UpdateArtifactInput,
};

use crate::error::AppError;
use crate::routes::file_serve::{apply_inline_media_headers, contained_canonical, HeaderOpts};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactListQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub kind: Option<String>,
    pub lifecycle_state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactImportBody {
    pub file_path: String,
    pub artifact_id: Option<String>,
    pub expected_version: Option<i64>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub privacy: Option<String>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub goal_id: Option<String>,
    pub version_message: Option<String>,
    pub producer: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBody {
    pub version: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBody {
    pub format: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReviewBody {
    pub audience: String,
    pub redaction_checked: bool,
}

pub async fn list_artifacts(
    Query(query): Query<ArtifactListQuery>,
) -> Result<Json<Value>, AppError> {
    let artifacts = ArtifactService::open()
        .and_then(|service| {
            service.list(ListArtifactsInput {
                limit: query.limit.unwrap_or(50),
                offset: query.offset.unwrap_or(0),
                kind: query.kind,
                lifecycle_state: query.lifecycle_state,
            })
        })
        .map_err(artifact_error)?;
    Ok(Json(json!({ "artifacts": artifacts })))
}

pub async fn get_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let artifact = ArtifactService::open()
        .and_then(|service| service.get(&id))
        .map_err(artifact_error)?
        .ok_or_else(|| AppError::not_found("artifact not found"))?;
    Ok(Json(json!({ "artifact": artifact })))
}

pub async fn list_versions(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let versions = ArtifactService::open()
        .and_then(|service| service.versions(&id))
        .map_err(artifact_error)?;
    Ok(Json(json!({ "artifactId": id, "versions": versions })))
}

pub async fn import_artifact(
    Json(body): Json<ArtifactImportBody>,
) -> Result<Json<Value>, AppError> {
    let source = resolve_import_path(&body.file_path);
    let roots = import_roots();
    let producer = body
        .producer
        .unwrap_or_else(|| json!({ "type": "owner_import", "surface": "http" }));
    let mut service = ArtifactService::open().map_err(artifact_error)?;

    let artifact = if let Some(artifact_id) = body.artifact_id {
        validate_id(&artifact_id)?;
        let current = service
            .get(&artifact_id)
            .map_err(artifact_error)?
            .ok_or_else(|| AppError::not_found("artifact not found"))?;
        let expected = body.expected_version.ok_or_else(|| {
            AppError::bad_request("expectedVersion is required when artifactId is provided")
        })?;
        if current.current_version != expected {
            return Err(AppError::conflict_with_code(
                "artifact_version_conflict",
                format!(
                    "expected version {}, current version {} ({})",
                    expected, current.current_version, current.current_hash
                ),
            ));
        }
        service
            .update_from_file(UpdateArtifactInput {
                artifact_id,
                file_path: source,
                expected_version: expected,
                title: body.title,
                message: body.version_message,
                producer,
                allowed_roots: Some(roots),
                incognito: false,
            })
            .map_err(artifact_error)?
    } else {
        service
            .create_from_file(CreateArtifactInput {
                file_path: source,
                title: body.title,
                kind: ArtifactKind::parse(body.kind.as_deref()),
                privacy: body.privacy.unwrap_or_else(|| "local_private".to_string()),
                session_id: body.session_id,
                project_id: body.project_id,
                agent_id: body.agent_id,
                goal_id: body.goal_id,
                producer,
                allowed_roots: Some(roots),
                incognito: false,
            })
            .map_err(artifact_error)?
    };
    Ok(Json(json!({ "artifact": artifact })))
}

pub async fn restore_artifact(
    Path(id): Path<String>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let artifact = ArtifactService::open()
        .and_then(|mut service| service.restore(&id, body.version))
        .map_err(artifact_error)?;
    Ok(Json(json!({ "artifact": artifact })))
}

pub async fn verify_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let verification = ArtifactService::open()
        .and_then(|service| service.verify(&id))
        .map_err(artifact_error)?;
    Ok(Json(json!({ "verification": verification })))
}

pub async fn create_export(
    Path(id): Path<String>,
    Json(body): Json<ExportBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let mut service = ArtifactService::open().map_err(artifact_error)?;
    let receipt = service
        .export_async(&id, &body.format)
        .await
        .map_err(artifact_error)?;
    Ok(Json(json!({ "receipt": receipt })))
}

pub async fn review_export(
    Path(id): Path<String>,
    Json(body): Json<ExportReviewBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let guard = ArtifactService::open()
        .and_then(|service| service.review_for_export(&id, &body.audience, body.redaction_checked))
        .map_err(artifact_error)?;
    Ok(Json(json!({ "guard": guard })))
}

pub async fn download_export(
    Path(export_id): Path<String>,
    request: Request,
) -> Result<Response, AppError> {
    validate_id(&export_id)?;
    let receipt = ArtifactService::open()
        .and_then(|service| service.get_export(&export_id))
        .map_err(artifact_error)?
        .ok_or_else(|| AppError::not_found("artifact export not found"))?;
    if receipt.status != "ready" {
        return Err(AppError::conflict_with_code(
            "artifact_export_not_ready",
            receipt
                .error
                .unwrap_or_else(|| "artifact export is not ready".to_string()),
        ));
    }
    let internal = receipt
        .internal_path
        .map(PathBuf::from)
        .ok_or_else(|| AppError::not_found("artifact export file not found"))?;
    let base = ha_core::paths::canvas_dir()
        .map_err(|error| AppError::internal(error.to_string()))?
        .join("exports");
    let canonical = contained_canonical(&base, &internal).await?;
    let mut response = ServeFile::new(&canonical)
        .oneshot(request)
        .await
        .map_err(|error| AppError::internal(format!("serve artifact export: {error}")))?
        .into_response();
    let disposition = format!(
        "attachment; filename=\"{}\"",
        receipt.filename.replace('"', "_")
    );
    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &receipt.mime_type,
            cache_secs: 0,
            disposition: &disposition,
            no_referrer: true,
        },
    );
    Ok(response)
}

pub async fn archive_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ArtifactService::open()
        .and_then(|service| service.archive(&id))
        .map_err(artifact_error)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ArtifactService::open()
        .and_then(|service| service.delete(&id))
        .map_err(artifact_error)?;
    Ok(Json(json!({ "ok": true })))
}

fn resolve_import_path(raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn import_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home);
    }
    roots
}

fn validate_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(AppError::bad_request("invalid artifact id"));
    }
    Ok(())
}

fn artifact_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("not found") {
        return AppError::not_found(message);
    }
    if message.contains("version conflict")
        || message.contains("Export Guard")
        || message.contains("verification failed")
    {
        return AppError::conflict_with_code("artifact_conflict", message);
    }
    if message.contains("required")
        || message.contains("unsupported")
        || message.contains("must ")
        || message.contains("may not")
        || message.contains("missing")
    {
        return AppError::bad_request(message);
    }
    AppError::internal(message)
}
