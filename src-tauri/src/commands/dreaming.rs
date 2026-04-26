//! Tauri commands wiring the Dreaming pipeline (Phase B3) to the
//! frontend. All heavy work happens inside ha-core; these commands are
//! thin error-translating shells.

use crate::commands::CmdError;
use ha_core::memory::dreaming;

/// Run an offline consolidation cycle synchronously and return the report.
/// Maps to `POST /api/dreaming/run` on the HTTP side.
#[tauri::command]
pub async fn dreaming_run_now() -> Result<dreaming::DreamReport, CmdError> {
    Ok(dreaming::manual_run(dreaming::DreamTrigger::Manual).await)
}

/// List Dream Diary markdown files (newest first). `limit` caps the
/// returned set so the Dashboard stays responsive after months of daily
/// cycles; omitting it returns the full set.
#[tauri::command]
pub async fn dreaming_list_diaries(
    limit: Option<usize>,
) -> Result<Vec<dreaming::DiaryEntry>, CmdError> {
    dreaming::list_diaries(limit).map_err(Into::into)
}

/// Read the markdown for a single diary file.
#[tauri::command]
pub async fn dreaming_read_diary(filename: String) -> Result<String, CmdError> {
    dreaming::read_diary(&filename).map_err(Into::into)
}

/// Lightweight status probe so the Dashboard can grey out the "Run now"
/// button while a cycle is already in progress.
#[tauri::command]
pub async fn dreaming_is_running() -> Result<bool, CmdError> {
    Ok(dreaming::dreaming_running())
}
