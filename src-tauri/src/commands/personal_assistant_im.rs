use crate::commands::CmdError;
use ha_channel::personal_assistant_im as service;
use ha_core::personal_assistant_im::{ConnectionView, PairingCode, SessionGrant};

#[tauri::command]
pub async fn personal_assistant_im_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<service::AssistantImStatus, CmdError> {
    service::status(state.session_db.clone())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_configure(
    token: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<ConnectionView, CmdError> {
    service::configure(state.session_db.clone(), token)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_begin_pairing(
    state: tauri::State<'_, crate::AppState>,
) -> Result<PairingCode, CmdError> {
    service::begin_pairing(state.session_db.clone())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_confirm_pairing(
    generation: i64,
    user_id: i64,
    chat_id: i64,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    service::confirm_pairing(state.session_db.clone(), generation, user_id, chat_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_set_grant(
    input: SessionGrant,
    state: tauri::State<'_, crate::AppState>,
) -> Result<SessionGrant, CmdError> {
    service::set_grant(state.session_db.clone(), input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_revoke_grant(
    session_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    service::revoke_grant(state.session_db.clone(), session_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_revoke_pairing(
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    service::revoke_pairing(state.session_db.clone())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_remove_connection(
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    service::remove_connection(state.session_db.clone())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_im_set_notifications_paused(
    paused: bool,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    service::set_notifications_paused(state.session_db.clone(), paused)
        .await
        .map_err(Into::into)
}
