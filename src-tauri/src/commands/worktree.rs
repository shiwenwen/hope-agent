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
    app_state
        .session_db
        .list_managed_worktrees_for_session(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
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
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn archive_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    app_state
        .session_db
        .archive_managed_worktree(&worktree_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn restore_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    app_state
        .session_db
        .restore_managed_worktree(&worktree_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn handoff_managed_worktree(
    worktree_id: String,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<ManagedWorktree, CmdError> {
    app_state
        .session_db
        .handoff_managed_worktree(&worktree_id)
        .map_err(Into::into)
}
