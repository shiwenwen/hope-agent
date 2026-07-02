use crate::commands::CmdError;
use ha_core::session::{SessionIdeContext, SessionIdeContextSnapshot};

#[tauri::command]
pub async fn get_session_ide_context(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<SessionIdeContextSnapshot>, CmdError> {
    app_state
        .session_db
        .get_session_ide_context(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_session_ide_context(
    session_id: String,
    context: SessionIdeContext,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<SessionIdeContextSnapshot, CmdError> {
    app_state
        .session_db
        .save_session_ide_context(&session_id, context)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_session_ide_context(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    app_state
        .session_db
        .clear_session_ide_context(&session_id)
        .map_err(Into::into)
}
