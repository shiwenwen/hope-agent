use crate::commands::CmdError;
use ha_core::personal_assistant::{AssistantRequest, AssistantWatch};
use ha_core::session::SessionMeta;

#[tauri::command]
pub async fn ensure_personal_assistant_home(
    state: tauri::State<'_, crate::AppState>,
) -> Result<SessionMeta, CmdError> {
    state
        .session_db
        .run(|db| db.ensure_personal_assistant_home())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_personal_assistant_requests(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<AssistantRequest>, CmdError> {
    let mut requests = state
        .session_db
        .run(|db| db.list_personal_assistant_requests())
        .await?;
    ha_core::personal_assistant::enrich_personal_assistant_requests(
        &mut requests,
        &state.session_db,
    )
    .await?;
    Ok(requests)
}

#[tauri::command]
pub async fn cancel_personal_assistant_request(
    id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<AssistantRequest, CmdError> {
    ha_core::tools::cancel_personal_assistant_dispatch(state.session_db.clone(), id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn personal_assistant_inbox(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<AssistantWatch>, CmdError> {
    state
        .session_db
        .run(|db| db.personal_assistant_inbox())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn watch_personal_goal(
    session_id: String,
    goal_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<AssistantWatch, CmdError> {
    state
        .session_db
        .run(move |db| db.watch_personal_goal(&session_id, &goal_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unwatch_personal_goal(
    session_id: String,
    goal_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.unwatch_personal_goal(&session_id, &goal_id))
        .await
        .map_err(Into::into)
}
