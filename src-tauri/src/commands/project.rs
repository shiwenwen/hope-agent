//! Tauri commands for the Project feature.
//!
//! Thin wrappers around [`oc_core::project`] — all business logic lives in
//! oc-core; this file only handles `Result<T, E>` → `Result<T, String>`
//! conversion and holds the `AppState` bridge.

use oc_core::project::{
    delete_project_cascade, delete_project_file as delete_file_pipeline, upload_project_file,
    CreateProjectInput, Project, ProjectFile, ProjectMeta, UpdateProjectInput, UploadInput,
};
use oc_core::session::SessionMeta;
use tauri::State;

use crate::AppState;

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ── Project CRUD ────────────────────────────────────────────────

#[tauri::command]
pub async fn list_projects_cmd(
    include_archived: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectMeta>, String> {
    let include_archived = include_archived.unwrap_or(false);
    let mut projects = state.project_db.list(include_archived).map_err(map_err)?;

    // Cross-DB enrichment: fetch project-scoped memory counts.
    if let Some(backend) = oc_core::get_memory_backend() {
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
) -> Result<Option<Project>, String> {
    state.project_db.get(&id).map_err(map_err)
}

#[tauri::command]
pub async fn create_project_cmd(
    input: CreateProjectInput,
    state: State<'_, AppState>,
) -> Result<Project, String> {
    let project = state.project_db.create(input).map_err(map_err)?;

    if let Some(bus) = oc_core::get_event_bus() {
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
) -> Result<Project, String> {
    let project = state.project_db.update(&id, patch).map_err(map_err)?;

    if let Some(bus) = oc_core::get_event_bus() {
        let _ = bus.emit(
            "project:updated",
            serde_json::json!({ "projectId": project.id }),
        );
    }
    Ok(project)
}

#[tauri::command]
pub async fn delete_project_cmd(id: String, state: State<'_, AppState>) -> Result<bool, String> {
    let deleted = delete_project_cascade(&id, &state.project_db).map_err(map_err)?;

    if deleted {
        if let Some(bus) = oc_core::get_event_bus() {
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
) -> Result<Project, String> {
    let patch = UpdateProjectInput {
        archived: Some(archived),
        ..Default::default()
    };
    let project = state.project_db.update(&id, patch).map_err(map_err)?;

    if let Some(bus) = oc_core::get_event_bus() {
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
    state: State<'_, AppState>,
) -> Result<(Vec<SessionMeta>, u32), String> {
    use oc_core::session::ProjectFilter;
    let (mut sessions, total) = state
        .session_db
        .list_sessions_paged(None, ProjectFilter::InProject(&id), limit, offset)
        .map_err(map_err)?;
    oc_core::session::enrich_pending_interactions(&mut sessions, &state.session_db)
        .await
        .map_err(map_err)?;
    Ok((sessions, total))
}

#[tauri::command]
pub async fn move_session_to_project_cmd(
    session_id: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .session_db
        .set_session_project(&session_id, project_id.as_deref())
        .map_err(map_err)
}

// ── Project Files ───────────────────────────────────────────────

#[tauri::command]
pub async fn list_project_files_cmd(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectFile>, String> {
    state.project_db.list_files(&id).map_err(map_err)
}

#[tauri::command]
pub async fn upload_project_file_cmd(
    project_id: String,
    file_name: String,
    mime_type: Option<String>,
    data: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<ProjectFile, String> {
    // Run the blocking upload pipeline (file IO + text extraction) off the
    // tokio runtime so the main thread stays responsive on large files.
    let project_db = state.project_db.clone();
    let file = tokio::task::spawn_blocking(move || -> anyhow::Result<ProjectFile> {
        upload_project_file(
            UploadInput {
                project_id: &project_id,
                original_filename: &file_name,
                mime_type: mime_type.as_deref(),
                data: &data,
            },
            &project_db,
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(map_err)?;

    if let Some(bus) = oc_core::get_event_bus() {
        let _ = bus.emit(
            "project:file_uploaded",
            serde_json::json!({
                "projectId": file.project_id,
                "fileId": file.id,
            }),
        );
    }
    Ok(file)
}

#[tauri::command]
pub async fn delete_project_file_cmd(
    project_id: String,
    file_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let project_db = state.project_db.clone();
    let file_id_for_pipe = file_id.clone();
    let deleted =
        tokio::task::spawn_blocking(move || delete_file_pipeline(&file_id_for_pipe, &project_db))
            .await
            .map_err(|e| e.to_string())?
            .map_err(map_err)?;

    if deleted {
        if let Some(bus) = oc_core::get_event_bus() {
            let _ = bus.emit(
                "project:file_deleted",
                serde_json::json!({
                    "projectId": project_id,
                    "fileId": file_id,
                }),
            );
        }
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn rename_project_file_cmd(
    _project_id: String,
    file_id: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .project_db
        .rename_file(&file_id, &name)
        .map_err(map_err)
}

#[tauri::command]
pub async fn read_project_file_content_cmd(
    project_id: String,
    file_id: String,
    offset: Option<u32>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let file = state
        .project_db
        .get_file(&project_id, &file_id)
        .map_err(map_err)?
        .ok_or_else(|| format!("file {} not found in project {}", file_id, project_id))?;

    let ext_rel = file
        .extracted_path
        .as_ref()
        .ok_or_else(|| "file has no extracted text (binary)".to_string())?;

    let base = oc_core::paths::projects_dir().map_err(map_err)?;
    let full = base.join(ext_rel);
    let content = tokio::fs::read_to_string(&full)
        .await
        .map_err(|e| format!("read {}: {}", full.display(), e))?;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let offset = offset.unwrap_or(0) as usize;
    let limit = limit.unwrap_or(1000).min(10_000) as usize;

    let end = (offset + limit).min(total);
    let start = offset.min(total);
    let window: Vec<&&str> = lines[start..end].iter().collect();
    let snippet = window
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(serde_json::json!({
        "content": snippet,
        "total": total,
        "offset": start,
        "limit": end - start,
    }))
}

#[tauri::command]
pub async fn list_project_memories_cmd(
    id: String,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<oc_core::memory::MemoryEntry>, String> {
    let backend = oc_core::get_memory_backend()
        .ok_or_else(|| "memory backend not initialized".to_string())?;
    let scope = oc_core::memory::MemoryScope::Project { id };
    backend
        .list(
            Some(&scope),
            None,
            limit.unwrap_or(200) as usize,
            offset.unwrap_or(0) as usize,
        )
        .map_err(map_err)
}
