//! Session-scoped Git owner API. Read operations are always available; every
//! mutation honors `filesystem.allow_remote_writes` in server mode.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::AppContext;
use ha_core::git_control::{
    self, GitCommitInput, GitCreateBranchInput, GitCreatePullRequestInput, GitDiffScope,
    GitEnablePullRequestAutoMergeInput, GitHandoffInput, GitIndexMutationInput, GitPushInput,
    GitSwitchBranchInput,
};

pub(super) fn ensure_writes_allowed() -> Result<(), AppError> {
    if ha_core::config::cached_config()
        .filesystem
        .allow_remote_writes
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "remote Git writes are disabled; enable filesystem.allowRemoteWrites to allow them",
        ))
    }
}

async fn blocking<T, F>(task: F) -> Result<Json<T>, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let value = tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| AppError::internal(format!("Git task failed: {error}")))?
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(value))
}

pub async fn snapshot(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<git_control::SessionGitControlSnapshot>, AppError> {
    let db = ctx.session_db.clone();
    blocking(move || git_control::load_control_snapshot(&db, &id)).await
}

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    pub scope: Option<String>,
}

pub async fn diff(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<git_control::SessionGitDiffSnapshot>, AppError> {
    let scope = match query.scope.as_deref().unwrap_or("unstaged") {
        "unstaged" => GitDiffScope::Unstaged,
        "staged" => GitDiffScope::Staged,
        "all" => GitDiffScope::All,
        _ => return Err(AppError::bad_request("invalid Git diff scope")),
    };
    let db = ctx.session_db.clone();
    blocking(move || git_control::load_session_git_diff_snapshot(&db, &id, scope)).await
}

pub async fn mutate_index(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(input): Json<GitIndexMutationInput>,
) -> Result<Json<git_control::SessionGitDiffSnapshot>, AppError> {
    ensure_writes_allowed()?;
    let db = ctx.session_db.clone();
    blocking(move || git_control::mutate_index(&db, &id, &input)).await
}

macro_rules! blocking_mutation {
    ($name:ident, $input:ty, $call:path) => {
        pub async fn $name(
            State(ctx): State<Arc<AppContext>>,
            Path(id): Path<String>,
            Json(input): Json<$input>,
        ) -> Result<Json<git_control::GitMutationResult>, AppError> {
            ensure_writes_allowed()?;
            let db = ctx.session_db.clone();
            blocking(move || $call(&db, &id, &input)).await
        }
    };
}

blocking_mutation!(
    switch_branch,
    GitSwitchBranchInput,
    git_control::switch_branch
);
blocking_mutation!(
    create_branch,
    GitCreateBranchInput,
    git_control::create_branch
);
blocking_mutation!(commit, GitCommitInput, git_control::commit);
blocking_mutation!(push, GitPushInput, git_control::push);
blocking_mutation!(
    create_pull_request,
    GitCreatePullRequestInput,
    git_control::create_pull_request
);
blocking_mutation!(
    enable_pull_request_auto_merge,
    GitEnablePullRequestAutoMergeInput,
    git_control::enable_pull_request_auto_merge
);

pub async fn pull_request_preflight(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<git_control::GitPullRequestPreflight>, AppError> {
    let db = ctx.session_db.clone();
    blocking(move || git_control::pull_request_preflight(&db, &id)).await
}

pub async fn pull_request_feedback(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<git_control::GitPullRequestFeedback>, AppError> {
    let db = ctx.session_db.clone();
    blocking(move || git_control::pull_request_feedback(&db, &id)).await
}

pub async fn handoff(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(input): Json<GitHandoffInput>,
) -> Result<Json<git_control::GitMutationResult>, AppError> {
    ensure_writes_allowed()?;
    let result = git_control::handoff(ctx.session_db.clone(), id, input)
        .await
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(result))
}

pub async fn operation_run(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Option<git_control::GitOperationRun>>, AppError> {
    let db = ctx.session_db.clone();
    blocking(move || db.get_git_operation_run(&id)).await
}
