//! Owner-only HTTP surface for updating the service process itself.

use axum::extract::Path;
use axum::{Extension, Json};
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::AuthState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareInstallBody {
    current_version: String,
    target_version: String,
    server_instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmInstallBody {
    plan_id: String,
}

pub async fn status() -> Result<Json<ha_updater::remote_update::RemoteUpdateStatus>, AppError> {
    Ok(Json(ha_updater::remote_update::status().await?))
}

pub async fn check() -> Result<Json<ha_updater::remote_update::RemoteUpdateStatus>, AppError> {
    Ok(Json(ha_updater::remote_update::check_now().await?))
}

pub async fn prepare(
    Extension(auth): Extension<AuthState>,
    Json(body): Json<PrepareInstallBody>,
) -> Result<Json<ha_updater::remote_update::RemoteInstallPlan>, AppError> {
    require_explicit_owner_auth(&auth)?;
    let plan = ha_updater::remote_update::prepare_install(
        &body.current_version,
        &body.target_version,
        &body.server_instance_id,
    )
    .await
    .map_err(|error| AppError::conflict_with_code("stale_update_plan", error.to_string()))?;
    Ok(Json(plan))
}

pub async fn confirm(
    Extension(auth): Extension<AuthState>,
    Json(body): Json<ConfirmInstallBody>,
) -> Result<Json<ha_updater::remote_update::RemoteUpdateJob>, AppError> {
    require_explicit_owner_auth(&auth)?;
    let job = ha_updater::remote_update::confirm_install(&body.plan_id)
        .await
        .map_err(|error| AppError::conflict_with_code("update_not_started", error.to_string()))?;
    Ok(Json(job))
}

pub async fn job(
    Path(job_id): Path<String>,
) -> Result<Json<ha_updater::remote_update::RemoteUpdateJob>, AppError> {
    let job = ha_updater::remote_update::job_status(&job_id)
        .await?
        .ok_or_else(|| AppError::not_found("update job not found"))?;
    Ok(Json(job))
}

fn require_explicit_owner_auth(auth: &AuthState) -> Result<(), AppError> {
    if auth.auth_required() {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "远程服务端更新要求启用 Server Owner Token",
        ))
    }
}
