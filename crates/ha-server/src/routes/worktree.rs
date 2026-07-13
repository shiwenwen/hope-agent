use axum::extract::Path;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateManagedWorktreeBody {
    pub source_working_dir: Option<String>,
    pub label: Option<String>,
    pub purpose: Option<String>,
    pub workflow_run_id: Option<String>,
    pub child_session_id: Option<String>,
    pub base_ref: Option<String>,
}

pub async fn list_managed_worktrees(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ha_core::worktree::ManagedWorktree>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.list_managed_worktrees_for_session(&session_id))
            .await?,
    ))
}

pub async fn create_managed_worktree(
    Path(session_id): Path<String>,
    Json(body): Json<CreateManagedWorktreeBody>,
) -> Result<Json<ha_core::worktree::ManagedWorktree>, AppError> {
    let db = session_db()?;
    let worktree = db
        .create_managed_worktree(ha_core::worktree::CreateManagedWorktreeInput {
            session_id,
            source_working_dir: body.source_working_dir,
            label: body.label,
            purpose: body
                .purpose
                .as_deref()
                .map(ha_core::worktree::ManagedWorktreePurpose::from_str)
                .unwrap_or(ha_core::worktree::ManagedWorktreePurpose::Manual),
            workflow_run_id: body.workflow_run_id,
            child_session_id: body.child_session_id,
            base_ref: body.base_ref,
            include_local_changes: false,
            bootstrap_request_id: None,
            bind_session_working_dir: false,
        })
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(worktree))
}

pub async fn get_managed_worktree(
    Path(id): Path<String>,
) -> Result<Json<Option<ha_core::worktree::ManagedWorktree>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.get_managed_worktree(&id))
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?,
    ))
}

pub async fn get_project_bootstrap_run(
    Path(request_id): Path<String>,
) -> Result<Json<Option<ha_core::project_bootstrap::ProjectBootstrapRun>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.get_project_bootstrap_run(&request_id))
            .await?,
    ))
}

pub async fn cancel_project_bootstrap(
    Path(request_id): Path<String>,
) -> Result<Json<bool>, AppError> {
    Ok(Json(ha_core::project_bootstrap::cancel_project_bootstrap(
        &request_id,
    )))
}

pub async fn archive_managed_worktree(
    Path(id): Path<String>,
) -> Result<Json<ha_core::worktree::ManagedWorktree>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.archive_managed_worktree(&id))
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?,
    ))
}

pub async fn restore_managed_worktree(
    Path(id): Path<String>,
) -> Result<Json<ha_core::worktree::ManagedWorktree>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.restore_managed_worktree(&id))
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?,
    ))
}

pub async fn handoff_managed_worktree(
    Path(id): Path<String>,
) -> Result<Json<ha_core::worktree::ManagedWorktree>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.handoff_managed_worktree(&id))
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?,
    ))
}
