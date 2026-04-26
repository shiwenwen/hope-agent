use crate::commands::CmdError;
use ha_core::recap::api;
use ha_core::recap::types::{GenerateMode, RecapReport, RecapReportSummary};

#[tauri::command]
pub async fn recap_generate(mode: GenerateMode) -> Result<RecapReport, CmdError> {
    api::generate(mode).await.map_err(Into::into)
}

#[tauri::command]
pub async fn recap_list_reports(limit: Option<u32>) -> Result<Vec<RecapReportSummary>, CmdError> {
    api::list_reports(limit.unwrap_or(50)).map_err(Into::into)
}

#[tauri::command]
pub async fn recap_get_report(id: String) -> Result<Option<RecapReport>, CmdError> {
    api::get_report(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn recap_delete_report(id: String) -> Result<(), CmdError> {
    api::delete_report(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn recap_export_html(
    id: String,
    output_path: Option<String>,
) -> Result<String, CmdError> {
    api::export_html(&id, output_path).map_err(Into::into)
}
