use crate::session;
use crate::session::ProjectFilter;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn create_session_cmd(
    agent_id: Option<String>,
    project_id: Option<String>,
    incognito: Option<bool>,
    state: State<'_, AppState>,
) -> Result<session::SessionMeta, String> {
    let agent_id = agent_id.unwrap_or_else(|| "default".to_string());
    state
        .session_db
        .create_session_with_project(&agent_id, project_id.as_deref(), incognito)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_sessions_cmd(
    agent_id: Option<String>,
    project_id: Option<String>,
    unassigned: Option<bool>,
    limit: Option<u32>,
    offset: Option<u32>,
    active_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMeta>, u32), String> {
    // Precedence: explicit `unassigned=true` wins; then `project_id`; else All.
    let project_filter = if unassigned.unwrap_or(false) {
        ProjectFilter::Unassigned
    } else if let Some(ref pid) = project_id {
        ProjectFilter::InProject(pid.as_str())
    } else {
        ProjectFilter::All
    };

    let (mut sessions, total) = state
        .session_db
        .list_sessions_paged(
            agent_id.as_deref(),
            project_filter,
            limit,
            offset,
            active_session_id.as_deref(),
        )
        .map_err(|e| e.to_string())?;

    session::enrich_pending_interactions(&mut sessions, &state.session_db)
        .await
        .map_err(|e| e.to_string())?;

    Ok((sessions, total))
}

#[tauri::command]
pub async fn load_session_messages_latest_cmd(
    session_id: String,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, u32, bool), String> {
    state
        .session_db
        .load_session_messages_latest(&session_id, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_session_messages_before_cmd(
    session_id: String,
    before_id: i64,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, bool), String> {
    state
        .session_db
        .load_session_messages_before(&session_id, before_id, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<session::SessionMeta>, String> {
    state
        .session_db
        .get_session(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_session_incognito(
    session_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .session_db
        .update_session_incognito(&session_id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .session_db
        .delete_session(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn purge_session_if_incognito(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    state
        .session_db
        .purge_session_if_incognito(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_session_cmd(
    session_id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .session_db
        .update_session_title(&session_id, &title)
        .map_err(|e| e.to_string())
}

/// Mark all messages in a session as read.
#[tauri::command]
pub async fn mark_session_read_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .session_db
        .mark_session_read(&session_id)
        .map_err(|e| e.to_string())
}

/// Mark all messages in multiple sessions as read.
#[tauri::command]
pub async fn mark_session_read_batch_cmd(
    session_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .session_db
        .mark_session_read_batch(&session_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mark_all_sessions_read_cmd(state: State<'_, AppState>) -> Result<(), String> {
    state
        .session_db
        .mark_all_sessions_read()
        .map_err(|e| e.to_string())
}

/// Search message history (FTS5) across sessions.
///
/// `types` accepts any combination of `"regular"`, `"cron"`, `"subagent"`,
/// `"channel"`. Passing `None` or an empty vec returns results from all
/// session types.
#[tauri::command]
pub async fn search_sessions_cmd(
    query: String,
    agent_id: Option<String>,
    types: Option<Vec<String>>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionSearchResult>, String> {
    let limit = limit.unwrap_or(80) as usize;

    let parsed_types: Option<Vec<session::SessionTypeFilter>> = types.map(|list| {
        list.iter()
            .filter_map(|s| session::SessionTypeFilter::parse(s))
            .collect()
    });
    let type_slice = parsed_types.as_deref();

    state
        .session_db
        .search_messages(&query, agent_id.as_deref(), None, type_slice, limit)
        .map_err(|e| e.to_string())
}

/// Search message history within a single session (FTS5). Used by the
/// in-chat "find in page" search bar.
#[tauri::command]
pub async fn search_session_messages_cmd(
    session_id: String,
    query: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionSearchResult>, String> {
    let limit = limit.unwrap_or(200) as usize;
    state
        .session_db
        .search_messages(&query, None, Some(&session_id), None, limit)
        .map_err(|e| e.to_string())
}

/// Load a window of messages centred on a target message id (used by search
/// result "jump to message" flow).
#[tauri::command]
pub async fn load_session_messages_around_cmd(
    session_id: String,
    target_message_id: i64,
    before: u32,
    after: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, u32, bool, bool), String> {
    state
        .session_db
        .load_session_messages_around(&session_id, target_message_id, before, after)
        .map_err(|e| e.to_string())
}

/// Report whether a session currently has an active chat stream running in
/// the backend. Used by the frontend on session switch to decide whether to
/// attach the EventBus-backed stream listener for a reloaded window that
/// lost its per-call Channel.
#[tauri::command]
pub async fn get_session_stream_state(
    session_id: String,
) -> Result<ha_core::chat_engine::SessionStreamState, String> {
    Ok(ha_core::chat_engine::session_stream_state(&session_id))
}
