use crate::commands::CmdError;
use ha_core::session::{SessionIdeContext, SessionIdeContextSnapshot};

#[tauri::command]
pub async fn get_session_ide_context(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<SessionIdeContextSnapshot>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_session_ide_context(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_session_ide_context(
    session_id: String,
    context: SessionIdeContext,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<SessionIdeContextSnapshot, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.save_session_ide_context(&session_id, context))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_session_ide_context(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.clear_session_ide_context(&session_id))
        .await
        .map_err(Into::into)
}
