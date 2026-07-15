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
use ha_core::session::{ParentSessionFilter, ProjectFilter, SessionMeta};

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
pub struct ReorderProjectsBody {
    pub project_ids: Vec<String>,
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

#[derive(Debug, Deserialize)]
pub struct WriteProjectMemoryBody {
    pub input: ha_core::project::memory::ProjectMemoryWriteInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectMemoryQuery {
    #[serde(default)]
    pub expected_file_hash: Option<String>,
}

// ── Project CRUD ────────────────────────────────────────────────

/// `GET /api/projects`
pub async fn list_projects(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<ListProjectsQuery>,
) -> Result<Json<Vec<ProjectMeta>>, AppError> {
    let projects = {
        let project_db = ctx.project_db.clone();
        let include_archived = q.include_archived.unwrap_or(false);
        let active_session_id = q.active_session_id.clone();
        ha_core::blocking::run_blocking(move || -> anyhow::Result<_> {
            let mut projects = project_db.list(include_archived, active_session_id.as_deref())?;

            // Enrich with cross-DB memory counts (memory.db is separate).
            if let Some(backend) = ha_core::get_memory_backend() {
                for meta in &mut projects {
                    if let Ok(n) = backend.count_by_project(&meta.project.id) {
                        meta.memory_count = n as u32;
                    }
                }
            }
            Ok(projects)
        })
        .await?
    };

    Ok(Json(projects))
}

/// `GET /api/projects/:id`
pub async fn get_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Project>, AppError> {
    let project = {
        let project_db = ctx.project_db.clone();
        let id = id.clone();
        ha_core::blocking::run_blocking(move || project_db.get(&id)).await?
    }
    .ok_or_else(|| anyhow::anyhow!("project not found: {}", id))?;
    Ok(Json(project))
}

/// `POST /api/projects`
pub async fn create_project(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<Project>, AppError> {
    let project = {
        let project_db = ctx.project_db.clone();
        ha_core::blocking::run_blocking(move || project_db.create(body.input)).await?
    };

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
    let project = {
        let project_db = ctx.project_db.clone();
        ha_core::blocking::run_blocking(move || project_db.update(&id, body.patch)).await?
    };
    ctx.event_bus
        .emit("project:updated", json!({ "projectId": project.id }));
    Ok(Json(project))
}

/// `DELETE /api/projects/:id`
pub async fn delete_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let deleted = {
        let project_db = ctx.project_db.clone();
        let id = id.clone();
        ha_core::blocking::run_blocking(move || delete_project_cascade(&id, &project_db)).await?
    };
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
    let project = {
        let project_db = ctx.project_db.clone();
        ha_core::blocking::run_blocking(move || project_db.update(&id, patch)).await?
    };
    ctx.event_bus
        .emit("project:updated", json!({ "projectId": project.id }));
    Ok(Json(project))
}

/// `POST /api/projects/reorder`
pub async fn reorder_projects(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<ReorderProjectsBody>,
) -> Result<Json<Value>, AppError> {
    {
        let project_db = ctx.project_db.clone();
        ha_core::blocking::run_blocking(move || project_db.reorder(&body.project_ids)).await?;
    }
    ctx.event_bus
        .emit("project:updated", json!({ "kind": "reordered" }));
    Ok(Json(json!({ "ok": true })))
}

// ── Session ↔ Project binding ───────────────────────────────────

/// `GET /api/projects/:id/sessions`
pub async fn list_project_sessions(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<ListProjectSessionsQuery>,
) -> Result<Json<PaginatedSessions>, AppError> {
    let (mut sessions, total) = {
        let id = id.clone();
        let limit = q.limit;
        let offset = q.offset;
        let active_session_id = q.active_session_id.clone();
        ctx.session_db
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
            .await?
    };
    ha_core::session::enrich_pending_interactions(&mut sessions, &ctx.session_db).await?;
    Ok(Json(PaginatedSessions { sessions, total }))
}

/// `PATCH /api/sessions/:id/project`
pub async fn move_session_to_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<MoveSessionBody>,
) -> Result<Json<Value>, AppError> {
    let session: Option<SessionMeta> = ctx
        .session_db
        .run(move |db| -> anyhow::Result<_> {
            db.set_session_project(&id, body.project_id.as_deref())?;

            // Return updated session meta so the frontend can refresh cache.
            db.get_session(&id)
        })
        .await?;
    Ok(Json(json!({ "session": session })))
}

/// `POST /api/projects/:id/read`
pub async fn mark_project_sessions_read(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    {
        let id = id.clone();
        ctx.session_db
            .run(move |db| db.mark_project_sessions_read(&id))
            .await?;
    }
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
    let limit = q.limit.unwrap_or(200) as usize;
    let offset = q.offset.unwrap_or(0) as usize;
    let entries =
        ha_core::blocking::run_blocking(move || backend.list(Some(&scope), None, limit, offset))
            .await?;
    Ok(Json(entries))
}

// ── Project Auto Memory ─────────────────────────────────────────

/// `GET /api/projects/:id/memory-files`
pub async fn list_project_memory_files(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ha_core::project::memory::ProjectMemoryEntry>>, AppError> {
    let project_db = ctx.project_db.clone();
    let entries = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::list(&id)
    })
    .await?;
    Ok(Json(entries))
}

/// `GET /api/projects/:id/memory-files/:file_name`
pub async fn read_project_memory_file(
    State(ctx): State<Arc<AppContext>>,
    Path((id, file_name)): Path<(String, String)>,
) -> Result<Json<ha_core::project::memory::ProjectMemoryFile>, AppError> {
    let project_db = ctx.project_db.clone();
    let file = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::read(&id, &file_name)
    })
    .await?;
    Ok(Json(file))
}

/// `PUT /api/projects/:id/memory-files`
pub async fn write_project_memory_file(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<WriteProjectMemoryBody>,
) -> Result<Json<ha_core::project::memory::ProjectMemoryFile>, AppError> {
    let project_db = ctx.project_db.clone();
    let event_project_id = id.clone();
    let file = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::write(&id, body.input)
    })
    .await?;
    ctx.event_bus.emit(
        "project_memory:changed",
        json!({ "projectId": event_project_id, "action": "write" }),
    );
    Ok(Json(file))
}

/// `DELETE /api/projects/:id/memory-files/:file_name`
pub async fn delete_project_memory_file(
    State(ctx): State<Arc<AppContext>>,
    Path((id, file_name)): Path<(String, String)>,
    Query(q): Query<DeleteProjectMemoryQuery>,
) -> Result<Json<bool>, AppError> {
    let project_db = ctx.project_db.clone();
    let event_project_id = id.clone();
    let deleted = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::delete(&id, &file_name, q.expected_file_hash.as_deref())
    })
    .await?;
    if deleted {
        ctx.event_bus.emit(
            "project_memory:changed",
            json!({ "projectId": event_project_id, "action": "delete" }),
        );
    }
    Ok(Json(deleted))
}

/// `POST /api/projects/:id/memory-files/rebuild-index`
pub async fn rebuild_project_memory_index(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<String>, AppError> {
    let project_db = ctx.project_db.clone();
    let event_project_id = id.clone();
    let index = ha_core::blocking::run_blocking(move || {
        ensure_project_exists(&project_db, &id)?;
        ha_core::project::memory::rebuild_index(&id)
    })
    .await?;
    ctx.event_bus.emit(
        "project_memory:changed",
        json!({ "projectId": event_project_id, "action": "rebuild_index" }),
    );
    Ok(Json(index))
}

fn ensure_project_exists(project_db: &ha_core::project::ProjectDB, id: &str) -> anyhow::Result<()> {
    if project_db.get(id)?.is_none() {
        anyhow::bail!("project not found: {}", id);
    }
    Ok(())
}
