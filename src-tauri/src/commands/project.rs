//! Tauri commands for the Project feature.
//!
//! Thin wrappers around [`ha_core::project`] — all business logic lives in
//! ha-core; this file only handles `Result<T, E>` → `Result<T, CmdError>`
//! conversion and holds the `AppState` bridge.

use crate::commands::CmdError;
use ha_core::project::{
    create_project_with_instructions_file, delete_project_cascade,
    inspect_default_project_instructions, inspect_project_instructions, read_project_instructions,
    save_project_instructions, update_project_with_instructions_file, CreateProjectInput, Project,
    ProjectInstructionsDraft, ProjectInstructionsFile, ProjectMeta, UpdateProjectInput,
};
use ha_core::session::SessionMeta;
use tauri::State;

use crate::AppState;

// ── Project CRUD ────────────────────────────────────────────────

#[tauri::command]
pub async fn list_projects_cmd(
    include_archived: Option<bool>,
    active_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectMeta>, CmdError> {
    let include_archived = include_archived.unwrap_or(false);
    let project_db = state.project_db.clone();
    let projects = ha_core::blocking::run_blocking(move || -> anyhow::Result<Vec<ProjectMeta>> {
        let mut projects = project_db.list(include_archived, active_session_id.as_deref())?;

        // Cross-DB enrichment: fetch project-scoped memory counts.
        if let Some(backend) = ha_core::get_memory_backend() {
            for meta in &mut projects {
                if let Ok(n) = backend.count_by_project(&meta.project.id) {
                    meta.memory_count = n as u32;
                }
            }
        }
        Ok(projects)
    })
    .await?;

    Ok(projects)
}

#[tauri::command]
pub async fn get_project_cmd(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Project>, CmdError> {
    let project_db = state.project_db.clone();
    ha_core::blocking::run_blocking(move || project_db.get(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_project_cmd(
    input: CreateProjectInput,
    instructions: Option<ProjectInstructionsDraft>,
    state: State<'_, AppState>,
) -> Result<Project, CmdError> {
    let instructions_changed = instructions.is_some();
    let project_db = state.project_db.clone();
    let project = ha_core::blocking::run_blocking(move || {
        create_project_with_instructions_file(input, instructions, &project_db)
    })
    .await?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:created",
            serde_json::json!({ "projectId": project.id }),
        );
        if instructions_changed {
            let _ = bus.emit(
                "project:fs_changed",
                serde_json::json!({ "scope": "project", "scopeId": project.id, "dir": "" }),
            );
        }
    }
    Ok(project)
}

#[tauri::command]
pub async fn update_project_cmd(
    id: String,
    patch: UpdateProjectInput,
    instructions: Option<ProjectInstructionsDraft>,
    state: State<'_, AppState>,
) -> Result<Project, CmdError> {
    let instructions_changed = instructions.is_some();
    let project_db = state.project_db.clone();
    let project = ha_core::blocking::run_blocking(move || {
        update_project_with_instructions_file(&id, patch, instructions, &project_db)
    })
    .await?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "projectId": project.id }),
        );
        if instructions_changed {
            let _ = bus.emit(
                "project:fs_changed",
                serde_json::json!({ "scope": "project", "scopeId": project.id, "dir": "" }),
            );
        }
    }
    Ok(project)
}

/// Inspect `<working-dir>/AGENTS.md` without creating a missing file.
#[tauri::command]
pub async fn inspect_project_instructions_cmd(
    working_dir: Option<String>,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectInstructionsFile, CmdError> {
    let project_db = state.project_db.clone();
    ha_core::blocking::run_blocking(move || {
        if let Some(path) = working_dir.filter(|path| !path.trim().is_empty()) {
            inspect_project_instructions(&path)
        } else if let Some(id) = project_id {
            inspect_default_project_instructions(&id, &project_db)
        } else {
            anyhow::bail!("workingDir or projectId is required")
        }
    })
    .await
    .map_err(Into::into)
}

/// Read `<project-root>/AGENTS.md`, creating an empty file when missing.
#[tauri::command]
pub async fn get_project_instructions_cmd(
    id: String,
    state: State<'_, AppState>,
) -> Result<ProjectInstructionsFile, CmdError> {
    let project_db = state.project_db.clone();
    ha_core::blocking::run_blocking(move || read_project_instructions(&id, &project_db))
        .await
        .map_err(Into::into)
}

/// Atomically replace `<project-root>/AGENTS.md` with Markdown source.
#[tauri::command]
pub async fn save_project_instructions_cmd(
    id: String,
    content: String,
    expected_file_hash: String,
    state: State<'_, AppState>,
) -> Result<ProjectInstructionsFile, CmdError> {
    let event_id = id.clone();
    let project_db = state.project_db.clone();
    let file = ha_core::blocking::run_blocking(move || {
        save_project_instructions(&id, &content, &expected_file_hash, &project_db)
    })
    .await?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:fs_changed",
            serde_json::json!({ "scope": "project", "scopeId": event_id, "dir": "" }),
        );
    }
    Ok(file)
}

#[tauri::command]
pub async fn delete_project_cmd(id: String, state: State<'_, AppState>) -> Result<bool, CmdError> {
    let deleted = {
        let id = id.clone();
        let project_db = state.project_db.clone();
        ha_core::blocking::run_blocking(move || delete_project_cascade(&id, &project_db)).await?
    };

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
    let project_db = state.project_db.clone();
    let project = ha_core::blocking::run_blocking(move || project_db.update(&id, patch)).await?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "projectId": project.id }),
        );
    }
    Ok(project)
}

#[tauri::command]
pub async fn reorder_projects_cmd(
    project_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let project_db = state.project_db.clone();
    ha_core::blocking::run_blocking(move || project_db.reorder(&project_ids)).await?;

    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "kind": "reordered" }),
        );
    }
    Ok(())
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
    use ha_core::session::{ParentSessionFilter, ProjectFilter};
    let (mut sessions, total) = state
        .session_db
        .run(move |db| {
            db.list_sessions_paged_for_sidebar(
                None,
                ProjectFilter::InProject(&id),
                ParentSessionFilter::All,
                limit,
                offset,
                active_session_id.as_deref(),
            )
        })
        .await?;
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
        .run(move |db| db.set_session_project(&session_id, project_id.as_deref()))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mark_project_sessions_read_cmd(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    {
        let project_id = project_id.clone();
        state
            .session_db
            .run(move |db| db.mark_project_sessions_read(&project_id))
            .await
            .map_err(CmdError::from)?;
    }

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
        .ok_or_else(|| CmdError::msg("memory backend not initialized"))?
        .clone();
    let scope = ha_core::memory::MemoryScope::Project { id };
    ha_core::blocking::run_blocking(move || {
        backend.list(
            Some(&scope),
            None,
            limit.unwrap_or(200) as usize,
            offset.unwrap_or(0) as usize,
        )
    })
    .await
    .map_err(Into::into)
}

// ── Project Auto Memory ─────────────────────────────────────────

#[tauri::command]
pub async fn list_project_memory_files_cmd(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ha_core::project::memory::ProjectMemoryEntry>, CmdError> {
    let project_db = state.project_db.clone();
    ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::list(&id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn read_project_memory_file_cmd(
    id: String,
    file_name: String,
    state: State<'_, AppState>,
) -> Result<ha_core::project::memory::ProjectMemoryFile, CmdError> {
    let project_db = state.project_db.clone();
    ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::read(&id, &file_name)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn write_project_memory_file_cmd(
    id: String,
    input: ha_core::project::memory::ProjectMemoryWriteInput,
    state: State<'_, AppState>,
) -> Result<ha_core::project::memory::ProjectMemoryFile, CmdError> {
    let project_db = state.project_db.clone();
    let event_project_id = id.clone();
    let file = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::write(&id, input)
    })
    .await?;
    emit_project_memory_changed(&event_project_id, "write");
    Ok(file)
}

#[tauri::command]
pub async fn delete_project_memory_file_cmd(
    id: String,
    file_name: String,
    expected_file_hash: Option<String>,
    state: State<'_, AppState>,
) -> Result<bool, CmdError> {
    let project_db = state.project_db.clone();
    let event_project_id = id.clone();
    let deleted = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::delete(&id, &file_name, expected_file_hash.as_deref())
    })
    .await?;
    if deleted {
        emit_project_memory_changed(&event_project_id, "delete");
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn rebuild_project_memory_index_cmd(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, CmdError> {
    let project_db = state.project_db.clone();
    let event_project_id = id.clone();
    let index = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::rebuild_index(&id)
    })
    .await?;
    emit_project_memory_changed(&event_project_id, "rebuild_index");
    Ok(index)
}

fn ensure_project_exists(project_db: &ha_core::project::ProjectDB, id: &str) -> anyhow::Result<()> {
    if project_db.get(id)?.is_none() {
        anyhow::bail!("project not found: {}", id);
    }
    Ok(())
}

fn emit_project_memory_changed(project_id: &str, action: &str) {
    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(
            "project_memory:changed",
            serde_json::json!({ "projectId": project_id, "action": action }),
        );
    }
}
