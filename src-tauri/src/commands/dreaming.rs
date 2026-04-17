//! Tauri commands wiring the Dreaming pipeline (Phase B3) to the
//! frontend. All heavy work happens inside oc-core; these commands are
//! thin error-translating shells.

use oc_core::memory::dreaming;

/// Run an offline consolidation cycle synchronously and return the report.
/// Maps to `POST /api/dreaming/run` on the HTTP side.
#[tauri::command]
pub async fn dreaming_run_now() -> Result<dreaming::DreamReport, String> {
    Ok(dreaming::manual_run(dreaming::DreamTrigger::Manual).await)
}

/// List all Dream Diary markdown files (newest first).
#[tauri::command]
pub async fn dreaming_list_diaries() -> Result<Vec<dreaming::DiaryEntry>, String> {
    dreaming::list_diaries().map_err(|e| e.to_string())
}

/// Read the markdown for a single diary file.
#[tauri::command]
pub async fn dreaming_read_diary(filename: String) -> Result<String, String> {
    dreaming::read_diary(&filename).map_err(|e| e.to_string())
}

/// Lightweight status probe so the Dashboard can grey out the "Run now"
/// button while a cycle is already in progress.
#[tauri::command]
pub async fn dreaming_is_running() -> Result<bool, String> {
    Ok(dreaming::dreaming_running())
}
