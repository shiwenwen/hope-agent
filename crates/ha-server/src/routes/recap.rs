use axum::extract::Path;
use axum::Json;
use serde::Deserialize;

use ha_core::recap::api;
use ha_core::recap::types::{GenerateMode, RecapReport, RecapReportSummary};

use crate::error::AppError;
use ha_core::blocking::run_blocking;

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
    Ok(Json(
        run_blocking(move || api::list_reports(body.limit.unwrap_or(50))).await?,
    ))
}

pub async fn get_report(Path(id): Path<String>) -> Result<Json<Option<RecapReport>>, AppError> {
    Ok(Json(run_blocking(move || api::get_report(&id)).await?))
}

pub async fn delete_report(Path(id): Path<String>) -> Result<Json<()>, AppError> {
    run_blocking(move || api::delete_report(&id)).await?;
    Ok(Json(()))
}

pub async fn export_html(
    Path(id): Path<String>,
    Json(body): Json<ExportBody>,
) -> Result<Json<String>, AppError> {
    Ok(Json(
        run_blocking(move || api::export_html(&id, body.output_path)).await?,
    ))
}
