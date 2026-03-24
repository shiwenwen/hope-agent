use tauri::State;
use crate::AppState;
use crate::session;

#[tauri::command]
pub async fn create_session_cmd(
    agent_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<session::SessionMeta, String> {
    let agent_id = agent_id.unwrap_or_else(|| "default".to_string());
    state.session_db.create_session(&agent_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_sessions_cmd(
    agent_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionMeta>, String> {
    state.session_db.list_sessions(agent_id.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_session_messages_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionMessage>, String> {
    state.session_db.load_session_messages(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_session_messages_latest_cmd(
    session_id: String,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, u32), String> {
    state.session_db.load_session_messages_latest(&session_id, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_session_messages_before_cmd(
    session_id: String,
    before_id: i64,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionMessage>, String> {
    state.session_db.load_session_messages_before(&session_id, before_id, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<session::SessionMeta>, String> {
    state.session_db.get_session(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.delete_session(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_session_cmd(
    session_id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.update_session_title(&session_id, &title).map_err(|e| e.to_string())
}

/// Mark all messages in a session as read.
#[tauri::command]
pub async fn mark_session_read_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.mark_session_read(&session_id).map_err(|e| e.to_string())
}

/// Mark all messages in multiple sessions as read.
#[tauri::command]
pub async fn mark_session_read_batch_cmd(
    session_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.mark_session_read_batch(&session_ids).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mark_all_sessions_read_cmd(
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.session_db.mark_all_sessions_read().map_err(|e| e.to_string())
}
