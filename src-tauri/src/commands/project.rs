//! Tauri commands for the Project feature.
//!
//! Thin wrappers around [`ha_core::project`] — all business logic lives in
//! ha-core; this file only handles `Result<T, E>` → `Result<T, CmdError>`
//! conversion and holds the `AppState` bridge.

use crate::commands::CmdError;
use ha_core::project::{
    delete_project_cascade, CreateProjectInput, Project, ProjectMeta, UpdateProjectInput,
};
use ha_core::session::SessionMeta;
use tauri::State;

use crate::AppState;

// ── Project CRUD ────────────────────────────────────────────────

#[tauri::command]
pub async fn list_projects_cmd(
    include_archived: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectMeta>, CmdError> {
    let include_archived = include_archived.unwrap_or(false);
    let mut projects = state.project_db.list(include_archived)?;

    // Cross-DB enrichment: fetch project-scoped memory counts.
    if let Some(backend) = ha_core::get_memory_backend() {
        for meta in &mut projects {
            if let Ok(n) = backend.count_by_project(&meta.project.id) {
                meta.memory_count = n as u32;
            }
        }
    }

    Ok(projects)
}

#[tauri::command]
pub async fn get_project_cmd(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Project>, CmdError> {
    state.project_db.get(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn create_project_cmd(
    input: CreateProjectInput,
    state: State<'_, AppState>,
) -> Result<Project, CmdError> {
    let project = state.project_db.create(input)?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:created",
            serde_json::json!({ "projectId": project.id }),
        );
    }
    Ok(project)
}

#[tauri::command]
pub async fn update_project_cmd(
    id: String,
    patch: UpdateProjectInput,
    state: State<'_, AppState>,
) -> Result<Project, CmdError> {
    let project = state.project_db.update(&id, patch)?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "projectId": project.id }),
        );
    }
    Ok(project)
}

#[tauri::command]
pub async fn delete_project_cmd(id: String, state: State<'_, AppState>) -> Result<bool, CmdError> {
    let deleted = delete_project_cascade(&id, &state.project_db)?;

    if deleted {
        if let Some(bus) = ha_core::get_event_bus() {
            let _ = bus.emit("project:deleted", serde_json::json!({ "projectId": id }));
        }
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn archive_project_cmd(
    id: String,
    archived: bool,
    state: State<'_, AppState>,
) -> Result<Project, CmdError> {
    let patch = UpdateProjectInput {
        archived: Some(archived),
        ..Default::default()
    };
    let project = state.project_db.update(&id, patch)?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "projectId": project.id }),
        );
    }
    Ok(project)
}

// ── Session ↔ Project binding ───────────────────────────────────

#[tauri::command]
pub async fn list_project_sessions_cmd(
    id: String,
    limit: Option<u32>,
    offset: Option<u32>,
    active_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(Vec<SessionMeta>, u32), CmdError> {
    use ha_core::session::ProjectFilter;
    let (mut sessions, total) = state.session_db.list_sessions_paged_for_sidebar(
        None,
        ProjectFilter::InProject(&id),
        limit,
        offset,
        active_session_id.as_deref(),
    )?;
    ha_core::session::enrich_pending_interactions(&mut sessions, &state.session_db).await?;
    Ok((sessions, total))
}

#[tauri::command]
pub async fn move_session_to_project_cmd(
    session_id: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .set_session_project(&session_id, project_id.as_deref())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mark_project_sessions_read_cmd(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .mark_project_sessions_read(&project_id)
        .map_err(CmdError::from)?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "projectId": project_id }),
        );
    }

    Ok(())
}

#[tauri::command]
pub async fn list_project_memories_cmd(
    id: String,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<ha_core::memory::MemoryEntry>, CmdError> {
    let backend = ha_core::get_memory_backend()
        .ok_or_else(|| CmdError::msg("memory backend not initialized"))?;
    let scope = ha_core::memory::MemoryScope::Project { id };
    backend
        .list(
            Some(&scope),
            None,
            limit.unwrap_or(200) as usize,
            offset.unwrap_or(0) as usize,
        )
        .map_err(Into::into)
}
