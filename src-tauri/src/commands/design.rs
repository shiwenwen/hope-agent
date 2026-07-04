//! Tauri commands for the Design Space feature.
//!
//! Thin wrappers around `ha_core::design` — all logic lives in ha-core. These
//! run on the **owner plane** (desktop = trusted local machine): the operator
//! sees all their design projects/artifacts, not gated by any agent access
//! check (that is for the agent `design` tool).

use crate::commands::CmdError;
use ha_core::design::service::{
    self, ArtifactView, CreateArtifactInput, CreateProjectInput, SaveSystemInput,
    UpdateProjectInput,
};
use ha_core::design::{
    DesignArtifact, DesignArtifactVersion, DesignConfig, DesignProject, DesignSystemFull,
    DesignSystemMeta,
};

// ── Projects ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_projects_cmd() -> Result<Vec<DesignProject>, CmdError> {
    service::list_projects().map_err(Into::into)
}

#[tauri::command]
pub async fn create_design_project_cmd(
    input: CreateProjectInput,
) -> Result<DesignProject, CmdError> {
    service::create_project(input).map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_project_cmd(id: String) -> Result<Option<DesignProject>, CmdError> {
    service::get_project(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn update_design_project_cmd(
    input: UpdateProjectInput,
) -> Result<DesignProject, CmdError> {
    service::update_project(input).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_project_cmd(id: String) -> Result<(), CmdError> {
    service::delete_project(&id).map_err(Into::into)
}

// ── Artifacts ───────────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_artifacts_cmd(
    project_id: String,
) -> Result<Vec<DesignArtifact>, CmdError> {
    service::list_artifacts(&project_id).map_err(Into::into)
}

#[tauri::command]
pub async fn create_design_artifact_cmd(
    input: CreateArtifactInput,
) -> Result<DesignArtifact, CmdError> {
    service::create_artifact(input).map_err(Into::into)
}

#[tauri::command]
pub async fn list_all_design_artifacts_cmd() -> Result<Vec<DesignArtifact>, CmdError> {
    service::list_all_artifacts().map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_artifact_cmd(id: String) -> Result<Option<ArtifactView>, CmdError> {
    service::get_artifact_view(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_artifact_cmd(id: String) -> Result<(), CmdError> {
    service::delete_artifact(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn list_design_artifact_versions_cmd(
    id: String,
) -> Result<Vec<DesignArtifactVersion>, CmdError> {
    service::list_versions(&id).map_err(Into::into)
}

// ── Design systems ──────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_systems_cmd() -> Result<Vec<DesignSystemMeta>, CmdError> {
    service::list_systems().map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_system_cmd(id: String) -> Result<DesignSystemFull, CmdError> {
    service::get_system_full(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn save_design_system_cmd(input: SaveSystemInput) -> Result<DesignSystemMeta, CmdError> {
    service::save_system(input).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_system_cmd(id: String) -> Result<(), CmdError> {
    service::delete_system(&id).map_err(Into::into)
}

// ── Config ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_design_config_cmd() -> Result<DesignConfig, CmdError> {
    Ok(ha_core::config::cached_config().design.clone())
}

#[tauri::command]
pub async fn save_design_config_cmd(config: DesignConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("design", "tauri"), |store| {
        store.design = config.clone();
        Ok(())
    })?;
    Ok(())
}
