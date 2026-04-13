use axum::extract::Path;
use axum::Json;
use serde::Deserialize;

use oc_core::recap::api;
use oc_core::recap::types::{GenerateMode, RecapReport, RecapReportSummary};

use crate::error::AppError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateBody {
    pub mode: GenerateMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListBody {
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBody {
    pub output_path: Option<String>,
}

pub async fn generate(Json(body): Json<GenerateBody>) -> Result<Json<RecapReport>, AppError> {
    Ok(Json(api::generate(body.mode).await?))
}

pub async fn list_reports(
    Json(body): Json<ListBody>,
) -> Result<Json<Vec<RecapReportSummary>>, AppError> {
    Ok(Json(api::list_reports(body.limit.unwrap_or(50))?))
}

pub async fn get_report(
    Path(id): Path<String>,
) -> Result<Json<Option<RecapReport>>, AppError> {
    Ok(Json(api::get_report(&id)?))
}

pub async fn delete_report(Path(id): Path<String>) -> Result<Json<()>, AppError> {
    api::delete_report(&id)?;
    Ok(Json(()))
}

pub async fn export_html(
    Path(id): Path<String>,
    Json(body): Json<ExportBody>,
) -> Result<Json<String>, AppError> {
    Ok(Json(api::export_html(&id, body.output_path)?))
}
