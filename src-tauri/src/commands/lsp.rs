use crate::commands::CmdError;

#[tauri::command]
pub async fn get_lsp_status(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ha_core::lsp::LspStatusSnapshot, CmdError> {
    ha_core::lsp::status_for_session(&app_state.session_db, &session_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_lsp_diagnostics(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ha_core::lsp::LspDiagnosticsSnapshot, CmdError> {
    ha_core::lsp::diagnostics_for_session(&app_state.session_db, &session_id)
        .await
        .map_err(Into::into)
}
