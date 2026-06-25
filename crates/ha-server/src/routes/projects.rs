//! HTTP handlers for the Project feature. Thin axum wrappers over
//! `ha_core::project` — uses `AppContext.project_db` + `session_db`.

use axum::extract::{Path, Query, State};
use axum::Json;

use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::memory::{MemoryEntry, MemoryScope};
use ha_core::project::{
    delete_project_cascade, CreateProjectInput, Project, ProjectMeta, UpdateProjectInput,
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
    /// Currently-open session id; excluded from each project's unread rollup
    /// so the badge matches the per-session "active reads as 0" rule.
    #[serde(default)]
    pub active_session_id: Option<String>,
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
    let mut projects = ctx.project_db.list(
        q.include_archived.unwrap_or(false),
        q.active_session_id.as_deref(),
    )?;

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
    let (mut sessions, total) = ctx.session_db.list_sessions_paged_for_sidebar(
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
