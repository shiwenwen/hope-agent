use std::sync::Arc;

use ha_core::git_control::{
    self, GitCommitInput, GitCreateBranchInput, GitCreatePullRequestInput, GitDiffScope,
    GitEnablePullRequestAutoMergeInput, GitHandoffInput, GitIndexMutationInput, GitPushInput,
    GitSwitchBranchInput,
};
use tauri::State;

use super::CmdError;
use crate::AppState;

#[tauri::command]
pub async fn load_session_git_control_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<git_control::SessionGitControlSnapshot, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || git_control::load_control_snapshot(&db, &session_id))
        .await
        .map_err(|error| CmdError::msg(format!("Git snapshot task failed: {error}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn load_session_git_diff_snapshot_cmd(
    session_id: String,
    scope: GitDiffScope,
    state: State<'_, AppState>,
) -> Result<git_control::SessionGitDiffSnapshot, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || {
        git_control::load_session_git_diff_snapshot(&db, &session_id, scope)
    })
    .await
    .map_err(|error| CmdError::msg(format!("Git diff task failed: {error}")))?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn mutate_session_git_index_cmd(
    session_id: String,
    input: GitIndexMutationInput,
    state: State<'_, AppState>,
) -> Result<git_control::SessionGitDiffSnapshot, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || git_control::mutate_index(&db, &session_id, &input))
        .await
        .map_err(|error| CmdError::msg(format!("Git index task failed: {error}")))?
        .map_err(Into::into)
}

macro_rules! blocking_git_command {
    ($name:ident, $input:ty, $call:path) => {
        #[tauri::command]
        pub async fn $name(
            session_id: String,
            input: $input,
            state: State<'_, AppState>,
        ) -> Result<git_control::GitMutationResult, CmdError> {
            let db = state.session_db.clone();
            tokio::task::spawn_blocking(move || $call(&db, &session_id, &input))
                .await
                .map_err(|error| CmdError::msg(format!("Git task failed: {error}")))?
                .map_err(Into::into)
        }
    };
}

blocking_git_command!(
    switch_session_git_branch_cmd,
    GitSwitchBranchInput,
    git_control::switch_branch
);
blocking_git_command!(
    create_session_git_branch_cmd,
    GitCreateBranchInput,
    git_control::create_branch
);
blocking_git_command!(commit_session_git_cmd, GitCommitInput, git_control::commit);
blocking_git_command!(push_session_git_cmd, GitPushInput, git_control::push);
blocking_git_command!(
    create_session_git_pr_cmd,
    GitCreatePullRequestInput,
    git_control::create_pull_request
);
blocking_git_command!(
    enable_session_git_pr_auto_merge_cmd,
    GitEnablePullRequestAutoMergeInput,
    git_control::enable_pull_request_auto_merge
);

#[tauri::command]
pub async fn session_git_pr_preflight_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<git_control::GitPullRequestPreflight, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || git_control::pull_request_preflight(&db, &session_id))
        .await
        .map_err(|error| CmdError::msg(format!("GitHub preflight task failed: {error}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn load_session_git_pr_feedback_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<git_control::GitPullRequestFeedback, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || git_control::pull_request_feedback(&db, &session_id))
        .await
        .map_err(|error| CmdError::msg(format!("GitHub feedback task failed: {error}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn handoff_session_git_cmd(
    session_id: String,
    input: GitHandoffInput,
    state: State<'_, AppState>,
) -> Result<git_control::GitMutationResult, CmdError> {
    git_control::handoff(Arc::clone(&state.session_db), session_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_git_operation_run_cmd(
    request_id: String,
    state: State<'_, AppState>,
) -> Result<Option<git_control::GitOperationRun>, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || db.get_git_operation_run(&request_id))
        .await
        .map_err(|error| CmdError::msg(format!("Git operation query failed: {error}")))?
        .map_err(Into::into)
}
