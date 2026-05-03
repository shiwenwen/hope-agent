//! HTTP handlers for the Project feature. Thin axum wrappers over
//! `ha_core::project` — uses `AppContext.project_db` + `session_db`.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use super::helpers::parse_file_upload;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::memory::{MemoryEntry, MemoryScope};
use ha_core::project::{
    delete_project_cascade, delete_project_file as delete_file_pipeline, upload_project_file,
    CreateProjectInput, Project, ProjectFile, ProjectMeta, UpdateProjectInput, UploadInput,
    MAX_PROJECT_FILE_BYTES,
};
use ha_core::session::{ProjectFilter, SessionMeta};

use crate::error::AppError;
use crate::routes::sessions::PaginatedSessions;
use crate::AppContext;

// ── Query / Body Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProjectsQuery {
    #[serde(default)]
    pub include_archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveProjectBody {
    pub archived: bool,
}

/// Body wrapper for `update_project_cmd`. Frontend ships
/// `{ id, patch: <UpdateProjectInput> }` (id is in the path).
#[derive(Debug, Deserialize)]
pub struct UpdateProjectBody {
    pub patch: UpdateProjectInput,
}

/// Body wrapper for `create_project_cmd`. Frontend ships `{ input: CreateProjectInput }`
/// so the same JSON object works on both the Tauri and HTTP transports.
#[derive(Debug, Deserialize)]
pub struct CreateProjectBody {
    pub input: CreateProjectInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveSessionBody {
    /// `None` removes the session from its current project.
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameProjectFileBody {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProjectFileQuery {
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoriesQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProjectSessionsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    /// Currently-open session id; allowed in results even if incognito.
    #[serde(default)]
    pub active_session_id: Option<String>,
}

// ── Project CRUD ────────────────────────────────────────────────

/// `GET /api/projects`
pub async fn list_projects(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<ListProjectsQuery>,
) -> Result<Json<Vec<ProjectMeta>>, AppError> {
    let mut projects = ctx.project_db.list(q.include_archived.unwrap_or(false))?;

    // Enrich with cross-DB memory counts (memory.db is separate).
    if let Some(backend) = ha_core::get_memory_backend() {
        for meta in &mut projects {
            if let Ok(n) = backend.count_by_project(&meta.project.id) {
                meta.memory_count = n as u32;
            }
        }
    }

    Ok(Json(projects))
}

/// `GET /api/projects/:id`
pub async fn get_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Project>, AppError> {
    let project = ctx
        .project_db
        .get(&id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", id))?;
    Ok(Json(project))
}

/// `POST /api/projects`
pub async fn create_project(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<Project>, AppError> {
    let project = ctx.project_db.create(body.input)?;

    ctx.event_bus
        .emit("project:created", json!({ "projectId": project.id }));
    Ok(Json(project))
}

/// `PATCH /api/projects/:id`
pub async fn update_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProjectBody>,
) -> Result<Json<Project>, AppError> {
    let project = ctx.project_db.update(&id, body.patch)?;
    ctx.event_bus
        .emit("project:updated", json!({ "projectId": project.id }));
    Ok(Json(project))
}

/// `DELETE /api/projects/:id`
pub async fn delete_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let deleted = delete_project_cascade(&id, &ctx.project_db)?;
    if deleted {
        ctx.event_bus
            .emit("project:deleted", json!({ "projectId": id }));
    }
    Ok(Json(json!({ "deleted": deleted })))
}

/// `POST /api/projects/:id/archive`
pub async fn archive_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<ArchiveProjectBody>,
) -> Result<Json<Project>, AppError> {
    let patch = UpdateProjectInput {
        archived: Some(body.archived),
        ..Default::default()
    };
    let project = ctx.project_db.update(&id, patch)?;
    ctx.event_bus
        .emit("project:updated", json!({ "projectId": project.id }));
    Ok(Json(project))
}

// ── Session ↔ Project binding ───────────────────────────────────

/// `GET /api/projects/:id/sessions`
pub async fn list_project_sessions(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<ListProjectSessionsQuery>,
) -> Result<Json<PaginatedSessions>, AppError> {
    let (mut sessions, total) = ctx.session_db.list_sessions_paged(
        None,
        ProjectFilter::InProject(&id),
        q.limit,
        q.offset,
        q.active_session_id.as_deref(),
    )?;
    ha_core::session::enrich_pending_interactions(&mut sessions, &ctx.session_db).await?;
    Ok(Json(PaginatedSessions { sessions, total }))
}

/// `PATCH /api/sessions/:id/project`
pub async fn move_session_to_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<MoveSessionBody>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db
        .set_session_project(&id, body.project_id.as_deref())?;

    // Return updated session meta so the frontend can refresh cache.
    let session: Option<SessionMeta> = ctx.session_db.get_session(&id)?;
    Ok(Json(json!({ "session": session })))
}

/// `POST /api/projects/:id/read`
pub async fn mark_project_sessions_read(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.mark_project_sessions_read(&id)?;
    ctx.event_bus
        .emit("project:updated", json!({ "projectId": id }));
    Ok(Json(json!({ "ok": true })))
}

// ── Project Files ───────────────────────────────────────────────

/// `GET /api/projects/:id/files`
pub async fn list_project_files(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ProjectFile>>, AppError> {
    let files = ctx.project_db.list_files(&id)?;
    Ok(Json(files))
}

/// `POST /api/projects/:id/files` — upload a project file (multipart/form-data).
///
/// Multipart fields: `file` (required), `fileName` / `mimeType` (optional text).
pub async fn upload_project_file_route(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<ProjectFile>), AppError> {
    let upload = parse_file_upload(multipart).await?;

    if upload.file_data.len() > MAX_PROJECT_FILE_BYTES {
        return Err(AppError::from(anyhow::anyhow!(
            "project file too large: {} bytes (max {} bytes)",
            upload.file_data.len(),
            MAX_PROJECT_FILE_BYTES
        )));
    }

    let project_db = ctx.project_db.clone();
    let project_id = id.clone();
    let file_name = upload.file_name;
    let mime_type = upload.mime_type;
    let data = upload.file_data;

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
    .map_err(|e| anyhow::anyhow!("upload task join error: {}", e))??;

    ctx.event_bus.emit(
        "project:file_uploaded",
        json!({ "projectId": file.project_id, "fileId": file.id }),
    );
    Ok((StatusCode::CREATED, Json(file)))
}

/// `DELETE /api/projects/:id/files/:fid`
pub async fn delete_project_file_route(
    State(ctx): State<Arc<AppContext>>,
    Path((id, fid)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let project_db = ctx.project_db.clone();
    let file_id = fid.clone();
    let deleted = tokio::task::spawn_blocking(move || delete_file_pipeline(&file_id, &project_db))
        .await
        .map_err(|e| anyhow::anyhow!("delete task join error: {}", e))??;

    if deleted {
        ctx.event_bus.emit(
            "project:file_deleted",
            json!({ "projectId": id, "fileId": fid }),
        );
    }
    Ok(Json(json!({ "deleted": deleted })))
}

/// `PATCH /api/projects/:id/files/:fid`
pub async fn rename_project_file_route(
    State(ctx): State<Arc<AppContext>>,
    Path((_id, fid)): Path<(String, String)>,
    Json(body): Json<RenameProjectFileBody>,
) -> Result<Json<Value>, AppError> {
    ctx.project_db.rename_file(&fid, &body.name)?;
    Ok(Json(json!({ "renamed": true })))
}

/// `GET /api/projects/:id/files/:fid/content`
pub async fn read_project_file_content(
    State(ctx): State<Arc<AppContext>>,
    Path((id, fid)): Path<(String, String)>,
    Query(q): Query<ReadProjectFileQuery>,
) -> Result<Json<Value>, AppError> {
    let file = ctx
        .project_db
        .get_file(&id, &fid)?
        .ok_or_else(|| anyhow::anyhow!("file {} not found in project {}", fid, id))?;

    let ext_rel = file
        .extracted_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("file has no extracted text (binary)"))?;

    let base = ha_core::paths::projects_dir()?;
    let full = base.join(ext_rel);
    let content = tokio::fs::read_to_string(&full).await?;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let offset = q.offset.unwrap_or(0) as usize;
    let limit = q.limit.unwrap_or(1000).min(10_000) as usize;

    let start = offset.min(total);
    let end = (start + limit).min(total);
    let snippet = lines[start..end].join("\n");

    Ok(Json(json!({
        "content": snippet,
        "total": total,
        "offset": start,
        "limit": end - start,
    })))
}

// ── Project Memories ────────────────────────────────────────────

/// `GET /api/projects/:id/memories`
pub async fn list_project_memories(
    State(_ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<ListMemoriesQuery>,
) -> Result<Json<Vec<MemoryEntry>>, AppError> {
    let backend = ha_core::get_memory_backend()
        .ok_or_else(|| anyhow::anyhow!("memory backend not initialized"))?;
    let scope = MemoryScope::Project { id };
    let entries = backend.list(
        Some(&scope),
        None,
        q.limit.unwrap_or(200) as usize,
        q.offset.unwrap_or(0) as usize,
    )?;
    Ok(Json(entries))
}
