use oc_core::recap::api;
use oc_core::recap::types::{GenerateMode, RecapReport, RecapReportSummary};

#[tauri::command]
pub async fn recap_generate(mode: GenerateMode) -> Result<RecapReport, String> {
    api::generate(mode).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recap_list_reports(limit: Option<u32>) -> Result<Vec<RecapReportSummary>, String> {
    api::list_reports(limit.unwrap_or(50)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recap_get_report(id: String) -> Result<Option<RecapReport>, String> {
    api::get_report(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recap_delete_report(id: String) -> Result<(), String> {
    api::delete_report(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recap_export_html(
    id: String,
    output_path: Option<String>,
) -> Result<String, String> {
    api::export_html(&id, output_path).map_err(|e| e.to_string())
}
