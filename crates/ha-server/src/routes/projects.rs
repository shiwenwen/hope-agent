//! HTTP handlers for the Project feature. Thin axum wrappers over
//! `ha_core::project` — uses `AppContext.project_db` + `session_db`.

use axum::extract::{Path, Query, State};
use axum::Json;

use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::memory::{MemoryEntry, MemoryScope};
use ha_core::project::{
    create_project_with_instructions_file, delete_project_cascade,
    inspect_default_project_instructions, inspect_project_instructions, read_project_instructions,
    save_project_instructions, update_project_with_instructions_file, CreateProjectInput, Project,
    ProjectInstructionsDraft, ProjectInstructionsFile, ProjectMeta, ProjectOverviewSummary,
    StaleProjectInstructionsError, UpdateProjectInput,
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
    #[serde(default)]
    pub instructions: Option<ProjectInstructionsDraft>,
}

/// Body wrapper for `create_project_cmd`. Frontend ships `{ input: CreateProjectInput }`
/// so the same JSON object works on both the Tauri and HTTP transports.
#[derive(Debug, Deserialize)]
pub struct CreateProjectBody {
    pub input: CreateProjectInput,
    #[serde(default)]
    pub instructions: Option<ProjectInstructionsDraft>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectProjectInstructionsBody {
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProjectInstructionsBody {
    pub content: String,
    pub expected_file_hash: String,
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
            project_db.list(include_archived, active_session_id.as_deref())
        })
        .await?
    };

    Ok(Json(projects))
}

/// `GET /api/projects/:id/overview`
pub async fn get_project_overview(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectOverviewSummary>, AppError> {
    let project_db = ctx.project_db.clone();
    let session_db = ctx.session_db.clone();
    let overview_session_db = session_db.clone();
    let mut summary = ha_core::blocking::run_blocking(move || {
        ha_core::project::build_project_overview(&id, &project_db, &overview_session_db)
    })
    .await?;
    ha_core::session::enrich_pending_interactions(&mut summary.recent_sessions, &session_db)
        .await?;
    Ok(Json(summary))
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
    let instructions_changed = body.instructions.is_some();
    let result = {
        let project_db = ctx.project_db.clone();
        ha_core::blocking::run_blocking(move || {
            create_project_with_instructions_file(body.input, body.instructions, &project_db)
        })
        .await
    };
    let project = project_instructions_result(result)?;

    ctx.event_bus
        .emit("project:created", json!({ "projectId": project.id }));
    if instructions_changed {
        ctx.event_bus.emit(
            "project:fs_changed",
            json!({ "scope": "project", "scopeId": project.id, "dir": "" }),
        );
    }
    Ok(Json(project))
}

/// `PATCH /api/projects/:id`
pub async fn update_project(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProjectBody>,
) -> Result<Json<Project>, AppError> {
    let instructions_changed = body.instructions.is_some();
    let result = {
        let project_db = ctx.project_db.clone();
        ha_core::blocking::run_blocking(move || {
            update_project_with_instructions_file(&id, body.patch, body.instructions, &project_db)
        })
        .await
    };
    let project = project_instructions_result(result)?;
    ctx.event_bus
        .emit("project:updated", json!({ "projectId": project.id }));
    if instructions_changed {
        ctx.event_bus.emit(
            "project:fs_changed",
            json!({ "scope": "project", "scopeId": project.id, "dir": "" }),
        );
    }
    Ok(Json(project))
}

fn project_instructions_result(result: anyhow::Result<Project>) -> Result<Project, AppError> {
    match result {
        Ok(project) => Ok(project),
        Err(error)
            if error
                .downcast_ref::<StaleProjectInstructionsError>()
                .is_some() =>
        {
            Err(AppError::conflict_with_code(
                "project_instructions_stale",
                error.to_string(),
            ))
        }
        Err(error) => Err(error.into()),
    }
}

/// `POST /api/projects/instructions/inspect` — inspect AGENTS.md in a selected
/// working directory without creating a missing file.
pub async fn inspect_project_instructions_file(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<InspectProjectInstructionsBody>,
) -> Result<Json<ProjectInstructionsFile>, AppError> {
    let project_db = ctx.project_db.clone();
    let file = ha_core::blocking::run_blocking(move || {
        if let Some(path) = body.working_dir.filter(|path| !path.trim().is_empty()) {
            inspect_project_instructions(&path)
        } else if let Some(id) = body.project_id {
            inspect_default_project_instructions(&id, &project_db)
        } else {
            anyhow::bail!("workingDir or projectId is required")
        }
    })
    .await?;
    Ok(Json(file))
}

/// `GET /api/projects/:id/instructions` — read (and lazily create) root AGENTS.md.
pub async fn get_project_instructions(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectInstructionsFile>, AppError> {
    let project_db = ctx.project_db.clone();
    let file = ha_core::blocking::run_blocking(move || read_project_instructions(&id, &project_db))
        .await?;
    Ok(Json(file))
}

/// `PUT /api/projects/:id/instructions` — atomically replace root AGENTS.md.
/// This is an authenticated owner setting, so it is intentionally independent
/// of the generic `filesystem.allow_remote_writes` file-browser gate.
pub async fn save_project_instructions_file(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<SaveProjectInstructionsBody>,
) -> Result<Json<ProjectInstructionsFile>, AppError> {
    let event_id = id.clone();
    let project_db = ctx.project_db.clone();
    let result = ha_core::blocking::run_blocking(move || {
        save_project_instructions(&id, &body.content, &body.expected_file_hash, &project_db)
    })
    .await;
    let file = match result {
        Ok(file) => file,
        Err(error)
            if error
                .downcast_ref::<StaleProjectInstructionsError>()
                .is_some() =>
        {
            return Err(AppError::conflict_with_code(
                "project_instructions_stale",
                error.to_string(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    ctx.event_bus.emit(
        "project:fs_changed",
        json!({ "scope": "project", "scopeId": event_id, "dir": "" }),
    );
    Ok(Json(file))
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
