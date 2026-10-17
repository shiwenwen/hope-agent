use crate::commands::CmdError;
use ha_dash::recap::api;
use ha_dash::recap::types::{GenerateMode, RecapReport, RecapReportSummary};

#[tauri::command]
pub async fn recap_generate(mode: GenerateMode) -> Result<RecapReport, CmdError> {
    api::generate(mode).await.map_err(Into::into)
}

#[tauri::command]
pub async fn recap_list_reports(limit: Option<u32>) -> Result<Vec<RecapReportSummary>, CmdError> {
    // Recap DB is synchronous rusqlite (mirrors the HTTP shell, which wraps
    // the same API in run_blocking).
    let limit = limit.unwrap_or(50);
    ha_core::blocking::run_blocking(move || api::list_reports(limit))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn recap_get_report(id: String) -> Result<Option<RecapReport>, CmdError> {
    ha_core::blocking::run_blocking(move || api::get_report(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn recap_delete_report(id: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || api::delete_report(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn recap_export_html(
    id: String,
    output_path: Option<String>,
) -> Result<String, CmdError> {
    // export_html also writes the rendered file to disk.
    ha_core::blocking::run_blocking(move || api::export_html(&id, output_path))
        .await
        .map_err(Into::into)
}
