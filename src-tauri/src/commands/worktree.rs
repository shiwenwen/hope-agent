use crate::commands::CmdError;
use ha_core::worktree::{CreateManagedWorktreeInput, ManagedWorktree, ManagedWorktreePurpose};

fn parse_purpose(purpose: Option<String>) -> ManagedWorktreePurpose {
    purpose
        .as_deref()
        .map(ManagedWorktreePurpose::from_str)
        .unwrap_or(ManagedWorktreePurpose::Manual)
}

#[tauri::command]
pub async fn list_managed_worktrees(
    session_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ManagedWorktree>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.list_managed_worktrees_for_session(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_managed_worktree(
    session_id: String,
    source_working_dir: Option<String>,
    label: Option<String>,
    purpose: Option<String>,
    workflow_run_id: Option<String>,
    child_session_id: Option<String>,
    base_ref: Option<String>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    app_state
        .session_db
        .create_managed_worktree(CreateManagedWorktreeInput {
            session_id,
            source_working_dir,
            label,
            purpose: parse_purpose(purpose),
            workflow_run_id,
            child_session_id,
            base_ref,
            include_local_changes: false,
            bootstrap_request_id: None,
            bind_session_working_dir: false,
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<ManagedWorktree>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_managed_worktree(&worktree_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_project_bootstrap_run(
    request_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<ha_core::project_bootstrap::ProjectBootstrapRun>, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.get_project_bootstrap_run(&request_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_project_bootstrap(request_id: String) -> Result<bool, CmdError> {
    Ok(ha_core::project_bootstrap::cancel_project_bootstrap(
        &request_id,
    ))
}

#[tauri::command]
pub async fn archive_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.archive_managed_worktree(&worktree_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn restore_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.restore_managed_worktree(&worktree_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn handoff_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    let db = app_state.session_db.clone();
    db.run(move |db| db.handoff_managed_worktree(&worktree_id))
        .await
        .map_err(Into::into)
}
