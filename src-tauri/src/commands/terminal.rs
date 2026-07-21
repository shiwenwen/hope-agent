//! Thin Tauri adapter for the transport-agnostic PTY manager.

use crate::commands::CmdError;
use crate::AppState;
use ha_core::terminal::{CreateTerminalRequest, TerminalSnapshot, TerminalSummary};
use tauri::State;

async fn blocking<T, F>(operation: F) -> Result<T, CmdError>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| CmdError::msg(format!("Terminal task failed: {error}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn terminal_create(
    request: CreateTerminalRequest,
    state: State<'_, AppState>,
) -> Result<TerminalSnapshot, CmdError> {
    let manager = state.terminal_manager.clone();
    blocking(move || manager.create(request)).await
}

#[tauri::command]
pub async fn terminal_list(state: State<'_, AppState>) -> Result<Vec<TerminalSummary>, CmdError> {
    Ok(state.terminal_manager.list())
}

#[tauri::command]
pub async fn terminal_snapshot(
    terminal_id: String,
    state: State<'_, AppState>,
) -> Result<TerminalSnapshot, CmdError> {
    let manager = state.terminal_manager.clone();
    blocking(move || manager.snapshot(&terminal_id)).await
}

#[tauri::command]
pub async fn terminal_write(
    terminal_id: String,
    data: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let manager = state.terminal_manager.clone();
    blocking(move || manager.write_input(&terminal_id, &data)).await
}

#[tauri::command]
pub async fn terminal_resize(
    terminal_id: String,
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let manager = state.terminal_manager.clone();
    blocking(move || manager.resize(&terminal_id, cols, rows)).await
}

#[tauri::command]
pub async fn terminal_close(
    terminal_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let manager = state.terminal_manager.clone();
    blocking(move || manager.close(&terminal_id)).await
}
