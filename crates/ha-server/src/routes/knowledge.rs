//! HTTP handlers for the Knowledge Base ("Knowledge Space") feature. Thin axum
//! wrappers over `ha_core::knowledge`.
//!
//! This is the **pure owner / management plane** (design D10 / two auth planes):
//! the API-key holder is owner-equivalent and sees all their knowledge bases —
//! there is **no session parameter and no `effective_kb_access` fallback** here.
//! The agent/session plane (`note_*` tools) is enforced inside ha-core and never
//! routes through these endpoints. File reads add a `WorkspaceScope` containment
//! check; writes are gated by `filesystem.allow_remote_writes`.

use axum::extract::{Path, Query, Request, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use ha_core::filesystem::{
    self, ExtractedContent, FileTextContent, FilesystemError, WorkspaceScope,
};
use ha_core::knowledge::{
    self, service, Backlink, BrokenLink, CreateKnowledgeBaseInput, GraphNodePosition, KbAccess,
    KbAttachment, KnowledgeBase, KnowledgeBaseMeta, KnowledgeGraph, Note, NoteReadResult,
    NoteSearchHit, ReferenceableNote, RenameOutcome, UpdateKnowledgeBaseInput,
};

use super::file_serve::{
    apply_inline_media_headers, resolve_mime_for_path, safe_content_disposition, HeaderOpts,
    MimeOpts,
};
use crate::error::AppError;
use crate::AppContext;

fn registry() -> Result<&'static std::sync::Arc<knowledge::KnowledgeRegistry>, AppError> {
    ha_core::get_knowledge_db().ok_or_else(|| AppError::internal("knowledge db not initialized"))
}

fn map_fs(e: FilesystemError) -> AppError {
    if e.is_bad_input() {
        AppError::bad_request(e.message().to_string())
    } else {
        AppError::internal(e.message().to_string())
    }
}

async fn run<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> filesystem::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("fs task failed: {e}")))?
        .map_err(map_fs)
}

fn ensure_writes_allowed() -> Result<(), AppError> {
    if ha_core::config::cached_config()
        .filesystem
        .allow_remote_writes
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "remote file writes are disabled; enable filesystem.allowRemoteWrites to allow them",
        ))
    }
}

fn emit(ctx: &AppContext, name: &str, payload: serde_json::Value) {
    ctx.event_bus.emit(name, payload);
}

// ── Query / Body types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListKbsQuery {
    #[serde(default)]
    pub include_archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateKbBody {
    pub input: CreateKnowledgeBaseInput,
}

#[derive(Debug, Deserialize)]
pub struct UpdateKbBody {
    pub patch: UpdateKnowledgeBaseInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachBody {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub kb_id: String,
    #[serde(default)]
    pub access: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionKbsQuery {
    pub session_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbNotePathQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbNoteRefQuery {
    pub reference: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbSearchQuery {
    pub query: String,
    #[serde(default)]
    pub kb_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbNoteSaveBody {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub expected_file_hash: Option<String>,
    #[serde(default)]
    pub create_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbNoteRenameBody {
    pub path: String,
    pub new_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbDirBody {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbAiRewriteBody {
    pub text: String,
    pub instruction: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbFileQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub download: Option<u8>,
}

// ── Registry CRUD ───────────────────────────────────────────────

/// `GET /api/knowledge`
pub async fn list_kbs(
    Query(q): Query<ListKbsQuery>,
) -> Result<Json<Vec<KnowledgeBaseMeta>>, AppError> {
    Ok(Json(service::list_kb_meta(
        q.include_archived.unwrap_or(false),
    )?))
}

/// `GET /api/knowledge/{kb_id}`
pub async fn get_kb(Path(kb_id): Path<String>) -> Result<Json<KnowledgeBase>, AppError> {
    let kb = registry()?
        .get(&kb_id)?
        .ok_or_else(|| AppError::not_found(format!("knowledge base not found: {kb_id}")))?;
    Ok(Json(kb))
}

/// `POST /api/knowledge`
pub async fn create_kb(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateKbBody>,
) -> Result<Json<KnowledgeBase>, AppError> {
    let kb = registry()?.create(body.input)?;
    knowledge::index::spawn_reindex_kb(kb.id.clone(), true);
    let _ = knowledge::watcher::start_watcher(&kb.id);
    emit(&ctx, "knowledge:created", json!({ "kbId": kb.id }));
    Ok(Json(kb))
}

/// `PATCH /api/knowledge/{kb_id}`
pub async fn update_kb(
    State(ctx): State<Arc<AppContext>>,
    Path(kb_id): Path<String>,
    Json(body): Json<UpdateKbBody>,
) -> Result<Json<KnowledgeBase>, AppError> {
    let kb = registry()?.update(&kb_id, body.patch)?;
    emit(
        &ctx,
        "knowledge:changed",
        json!({ "kbId": kb.id, "op": "meta" }),
    );
    Ok(Json(kb))
}

/// `DELETE /api/knowledge/{kb_id}`
pub async fn delete_kb(
    State(ctx): State<Arc<AppContext>>,
    Path(kb_id): Path<String>,
) -> Result<Json<bool>, AppError> {
    knowledge::watcher::stop_watcher(&kb_id);
    let removed = knowledge::delete_kb_cascade(&kb_id)?;
    emit(
        &ctx,
        "knowledge:changed",
        json!({ "kbId": kb_id, "op": "delete" }),
    );
    Ok(Json(removed))
}

/// `POST /api/knowledge/{kb_id}/reindex`
pub async fn reindex_kb(Path(kb_id): Path<String>) -> Result<Json<bool>, AppError> {
    // Single-KB rebuild through the KnowledgeReembed job (progress-tracked, no
    // signature stamp). Rebuilds FTS + (if embedding enabled) vectors.
    knowledge::start_knowledge_reembed_job(Some(vec![kb_id]), "manual-reindex")?;
    Ok(Json(true))
}

/// `POST /api/knowledge/{kb_id}/note/reindex` — rebuild one note's index
/// (synchronous; `body.path` is the note's KB-relative path).
pub async fn reindex_note(
    Path(kb_id): Path<String>,
    Json(body): Json<KbDirBody>,
) -> Result<Json<bool>, AppError> {
    let path = body.path;
    tokio::task::spawn_blocking(move || knowledge::index::reindex_note_by_path(&kb_id, &path))
        .await
        .map_err(|e| AppError::internal(format!("reindex note task failed: {e}")))??;
    Ok(Json(true))
}

/// `POST /api/knowledge/{kb_id}/dir/reindex` — rebuild a folder's index
/// (synchronous; `body.path` is the folder's KB-relative path, `""` = root).
pub async fn reindex_dir(
    Path(kb_id): Path<String>,
    Json(body): Json<KbDirBody>,
) -> Result<Json<bool>, AppError> {
    let path = body.path;
    tokio::task::spawn_blocking(move || knowledge::index::reindex_dir(&kb_id, &path))
        .await
        .map_err(|e| AppError::internal(format!("reindex dir task failed: {e}")))??;
    Ok(Json(true))
}

// ── Access bindings ─────────────────────────────────────────────

/// `POST /api/knowledge/attach`
pub async fn attach_kb(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<AttachBody>,
) -> Result<Json<bool>, AppError> {
    let access = KbAccess::from_str_lenient(body.access.as_deref().unwrap_or("read"));
    let reg = registry()?;
    if let Some(sid) = &body.session_id {
        reg.attach_session(sid, &body.kb_id, access)?;
    } else if let Some(pid) = &body.project_id {
        reg.attach_project(pid, &body.kb_id, access)?;
    } else {
        return Err(AppError::bad_request(
            "attach requires sessionId or projectId",
        ));
    }
    emit(
        &ctx,
        "knowledge:changed",
        json!({ "kbId": body.kb_id, "op": "attach" }),
    );
    Ok(Json(true))
}

/// `POST /api/knowledge/detach`
pub async fn detach_kb(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<AttachBody>,
) -> Result<Json<bool>, AppError> {
    let reg = registry()?;
    if let Some(sid) = &body.session_id {
        reg.detach_session(sid, &body.kb_id)?;
    } else if let Some(pid) = &body.project_id {
        reg.detach_project(pid, &body.kb_id)?;
    } else {
        return Err(AppError::bad_request(
            "detach requires sessionId or projectId",
        ));
    }
    emit(
        &ctx,
        "knowledge:changed",
        json!({ "kbId": body.kb_id, "op": "detach" }),
    );
    Ok(Json(true))
}

/// `GET /api/knowledge/attachments?sessionId=&projectId=`
pub async fn list_session_kbs(
    Query(q): Query<ListSessionKbsQuery>,
) -> Result<Json<Vec<KbAttachment>>, AppError> {
    let reg = registry()?;
    let mut out: Vec<KbAttachment> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (kb_id, access) in reg.list_session_attachments(&q.session_id)? {
        if let Ok(Some(kb)) = reg.get(&kb_id) {
            seen.insert(kb_id);
            out.push(KbAttachment {
                kb,
                access,
                via: "session".into(),
            });
        }
    }
    if let Some(pid) = q.project_id {
        for (kb_id, access) in reg.list_project_attachments(&pid)? {
            if seen.contains(&kb_id) {
                continue;
            }
            if let Ok(Some(kb)) = reg.get(&kb_id) {
                out.push(KbAttachment {
                    kb,
                    access,
                    via: "project".into(),
                });
            }
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProjectKbsQuery {
    pub project_id: String,
}

/// `GET /api/knowledge/project-attachments?projectId=` — project-scoped KB
/// attachments for the project settings UI (owner plane).
pub async fn list_project_kbs(
    Query(q): Query<ListProjectKbsQuery>,
) -> Result<Json<Vec<KbAttachment>>, AppError> {
    let reg = registry()?;
    let mut out: Vec<KbAttachment> = Vec::new();
    for (kb_id, access) in reg.list_project_attachments(&q.project_id)? {
        if let Ok(Some(kb)) = reg.get(&kb_id) {
            out.push(KbAttachment {
                kb,
                access,
                via: "project".into(),
            });
        }
    }
    Ok(Json(out))
}

// ── Notes (owner plane) ─────────────────────────────────────────

/// `GET /api/knowledge/{kb_id}/notes`
pub async fn list_kb_notes(Path(kb_id): Path<String>) -> Result<Json<Vec<Note>>, AppError> {
    Ok(Json(service::list_notes(&kb_id)?))
}

/// `GET /api/knowledge/{kb_id}/note?path=`
pub async fn kb_note_read(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNotePathQuery>,
) -> Result<Json<NoteReadResult>, AppError> {
    Ok(Json(service::note_read(&kb_id, &q.path)?))
}

/// `PUT /api/knowledge/{kb_id}/note`
pub async fn kb_note_save(
    Path(kb_id): Path<String>,
    Json(body): Json<KbNoteSaveBody>,
) -> Result<Json<String>, AppError> {
    ensure_writes_allowed()?;
    let hash = service::note_save(
        &kb_id,
        &body.path,
        &body.content,
        body.expected_file_hash.as_deref(),
        body.create_only.unwrap_or(false),
    )?;
    Ok(Json(hash))
}

/// `DELETE /api/knowledge/{kb_id}/note?path=`
pub async fn kb_note_delete(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNotePathQuery>,
) -> Result<Json<bool>, AppError> {
    ensure_writes_allowed()?;
    service::note_delete(&kb_id, &q.path)?;
    Ok(Json(true))
}

/// `POST /api/knowledge/{kb_id}/note/rename`
pub async fn kb_note_rename(
    Path(kb_id): Path<String>,
    Json(body): Json<KbNoteRenameBody>,
) -> Result<Json<RenameOutcome>, AppError> {
    ensure_writes_allowed()?;
    Ok(Json(
        service::note_rename(&kb_id, &body.path, &body.new_path).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/dirs`
pub async fn kb_list_dirs(Path(kb_id): Path<String>) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(service::list_dirs(&kb_id).await?))
}

/// `GET /api/knowledge/{kb_id}/tags`
pub async fn kb_list_tags(Path(kb_id): Path<String>) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(service::list_tags(&kb_id)?))
}

// ── Knowledge embedding selection (D7, owner plane) ─────────────
// Independent of memory_embedding; draws from the shared `embedding_models`
// library. Set/disable carry a background reindex side effect.

/// `GET /api/knowledge/embedding` — current knowledge embedding selection state.
pub async fn knowledge_embedding_get(
) -> Result<Json<ha_core::memory::EmbeddingSelectionState>, AppError> {
    Ok(Json(ha_core::knowledge::get_knowledge_embedding_state()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEmbeddingSetDefaultBody {
    pub model_config_id: String,
}

/// `POST /api/knowledge/embedding/set-default`
pub async fn knowledge_embedding_set_default(
    Json(body): Json<KnowledgeEmbeddingSetDefaultBody>,
) -> Result<Json<ha_core::memory::EmbeddingSelectionState>, AppError> {
    Ok(Json(ha_core::knowledge::set_knowledge_embedding_default(
        &body.model_config_id,
        "http",
    )?))
}

/// `POST /api/knowledge/embedding/disable`
pub async fn knowledge_embedding_disable(
) -> Result<Json<ha_core::memory::EmbeddingSelectionState>, AppError> {
    Ok(Json(ha_core::knowledge::disable_knowledge_embedding(
        "http",
    )?))
}

/// `POST /api/knowledge/embedding/rebuild` — force a full rebuild of every KB
/// under the active model (no same-signature short-circuit).
pub async fn knowledge_embedding_rebuild() -> Result<Json<bool>, AppError> {
    ha_core::knowledge::start_knowledge_reembed_job(None, "manual-rebuild")?;
    Ok(Json(true))
}

/// `GET /api/knowledge/chunk` — current chunking parameters (advanced).
pub async fn knowledge_chunk_get() -> Result<Json<ha_core::knowledge::ChunkConfig>, AppError> {
    Ok(Json(ha_core::knowledge::get_chunk_config()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeChunkSetBody {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

/// `POST /api/knowledge/chunk` — save chunking parameters and reindex every KB.
pub async fn knowledge_chunk_set(
    Json(body): Json<KnowledgeChunkSetBody>,
) -> Result<Json<ha_core::knowledge::ChunkConfig>, AppError> {
    Ok(Json(ha_core::knowledge::set_chunk_config(
        body.max_chars,
        body.overlap_chars,
        "http",
    )?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceableNotesBody {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub draft_kb_ids: Vec<String>,
}

/// `POST /api/knowledge/referenceable-notes` — flat note list for the chat
/// composer's `[[ ]]` picker. POST (not GET) so the draft-kb-id array rides in a
/// JSON body. Read-only owner plane; not gated by `allow_remote_writes`.
pub async fn list_referenceable_notes(
    Json(body): Json<ReferenceableNotesBody>,
) -> Result<Json<Vec<ReferenceableNote>>, AppError> {
    Ok(Json(service::list_referenceable_notes(
        body.session_id.as_deref(),
        body.project_id.as_deref(),
        &body.draft_kb_ids,
    )?))
}

/// `POST /api/knowledge/{kb_id}/dir`
pub async fn kb_mkdir(
    Path(kb_id): Path<String>,
    Json(body): Json<KbDirBody>,
) -> Result<Json<String>, AppError> {
    ensure_writes_allowed()?;
    Ok(Json(service::mkdir(&kb_id, &body.path)?))
}

/// `POST /api/knowledge/{kb_id}/dir/rename`
pub async fn kb_rename_dir(
    Path(kb_id): Path<String>,
    Json(body): Json<KbNoteRenameBody>,
) -> Result<Json<RenameOutcome>, AppError> {
    ensure_writes_allowed()?;
    Ok(Json(
        service::rename_dir(&kb_id, &body.path, &body.new_path).await?,
    ))
}

/// `DELETE /api/knowledge/{kb_id}/dir?path=`
pub async fn kb_delete_dir(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNotePathQuery>,
) -> Result<Json<bool>, AppError> {
    ensure_writes_allowed()?;
    service::delete_dir(&kb_id, &q.path).await?;
    Ok(Json(true))
}

/// `GET /api/knowledge/{kb_id}/backlinks?path=`
pub async fn kb_backlinks(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNotePathQuery>,
) -> Result<Json<Vec<Backlink>>, AppError> {
    Ok(Json(service::backlinks(&kb_id, &q.path)?))
}

/// `GET /api/knowledge/{kb_id}/broken-links`
pub async fn kb_broken_links(Path(kb_id): Path<String>) -> Result<Json<Vec<BrokenLink>>, AppError> {
    Ok(Json(service::broken_links(&kb_id)?))
}

/// `GET /api/knowledge/{kb_id}/orphans`
pub async fn kb_orphans(Path(kb_id): Path<String>) -> Result<Json<Vec<Note>>, AppError> {
    Ok(Json(service::orphans(&kb_id)?))
}

/// `GET /api/knowledge/{kb_id}/graph`
pub async fn kb_graph(Path(kb_id): Path<String>) -> Result<Json<KnowledgeGraph>, AppError> {
    Ok(Json(service::graph(&kb_id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbGraphLayoutBody {
    pub positions: Vec<GraphNodePosition>,
}

/// `GET /api/knowledge/{kb_id}/graph/layout` — user-pinned node positions (Batch J).
pub async fn kb_graph_layout_get(
    Path(kb_id): Path<String>,
) -> Result<Json<Vec<GraphNodePosition>>, AppError> {
    Ok(Json(service::graph_layout(&kb_id)?))
}

/// `POST /api/knowledge/{kb_id}/graph/layout` — replace the layout (empty resets).
pub async fn kb_graph_layout_save(
    Path(kb_id): Path<String>,
    Json(body): Json<KbGraphLayoutBody>,
) -> Result<Json<bool>, AppError> {
    service::save_graph_layout(&kb_id, &body.positions)?;
    Ok(Json(true))
}

/// `POST /api/knowledge/ai/rewrite` — AI rewrite of a text selection (WS9). Returns
/// rewritten Markdown; the client diffs it and the user saves through `note_save`.
pub async fn kb_ai_rewrite(Json(body): Json<KbAiRewriteBody>) -> Result<Json<String>, AppError> {
    Ok(Json(
        service::ai_rewrite(&body.text, &body.instruction).await?,
    ))
}

// ── Layer-2 autonomous maintenance (WS6) ────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbMaintListQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// `POST /api/knowledge/maintenance/run` — run one maintenance cycle.
pub async fn kb_maintenance_run(
) -> Result<Json<knowledge::maintenance::MaintenanceReport>, AppError> {
    Ok(Json(
        knowledge::maintenance::manual_run(knowledge::maintenance::MaintenanceTrigger::Manual)
            .await,
    ))
}

/// `GET /api/knowledge/maintenance/status`
pub async fn kb_maintenance_status(
) -> Result<Json<knowledge::maintenance::MaintenanceStatus>, AppError> {
    Ok(Json(knowledge::maintenance::status()))
}

/// `GET /api/knowledge/{kb_id}/maintenance/proposals?status=`
pub async fn kb_maintenance_list(
    Path(kb_id): Path<String>,
    Query(q): Query<KbMaintListQuery>,
) -> Result<Json<Vec<knowledge::maintenance::MaintenanceProposal>>, AppError> {
    let st = q
        .status
        .as_deref()
        .and_then(knowledge::maintenance::ProposalStatus::from_str);
    Ok(Json(knowledge::maintenance::list_proposals(&kb_id, st)?))
}

/// `GET /api/knowledge/{kb_id}/maintenance/pending-count`
pub async fn kb_maintenance_pending_count(
    Path(kb_id): Path<String>,
) -> Result<Json<usize>, AppError> {
    Ok(Json(knowledge::maintenance::pending_count(&kb_id)?))
}

/// `POST /api/knowledge/maintenance/proposals/{id}/approve`
pub async fn kb_maintenance_approve(
    Path(id): Path<i64>,
) -> Result<Json<knowledge::maintenance::MaintenanceProposal>, AppError> {
    Ok(Json(knowledge::maintenance::approve_proposal(id).await?))
}

/// `POST /api/knowledge/maintenance/proposals/{id}/reject`
pub async fn kb_maintenance_reject(Path(id): Path<i64>) -> Result<Json<bool>, AppError> {
    knowledge::maintenance::reject_proposal(id)?;
    Ok(Json(true))
}

/// `POST /api/knowledge/{kb_id}/maintenance/reject-all`
pub async fn kb_maintenance_reject_all(Path(kb_id): Path<String>) -> Result<Json<usize>, AppError> {
    Ok(Json(knowledge::maintenance::reject_all(&kb_id)?))
}

/// `GET /api/knowledge/maintenance/config`
pub async fn kb_maintenance_config_get(
) -> Result<Json<knowledge::maintenance::MaintenanceConfig>, AppError> {
    Ok(Json(service::get_maintenance_config()))
}

/// `POST /api/knowledge/maintenance/config` — body is `{ config: MaintenanceConfig }`
/// (the `ConfigBody` wrapper every `save_*_config` route uses; the Tauri command +
/// HTTP transport both ship the config under a `config` key).
pub async fn kb_maintenance_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<knowledge::maintenance::MaintenanceConfig>>,
) -> Result<Json<knowledge::maintenance::MaintenanceConfig>, AppError> {
    Ok(Json(service::set_maintenance_config(body.config, "http")?))
}

/// `GET /api/knowledge/passive-recall/config` (read bridge ③)
pub async fn kb_passive_recall_config_get() -> Result<Json<knowledge::PassiveRecallConfig>, AppError>
{
    Ok(Json(service::get_passive_recall_config()))
}

/// `POST /api/knowledge/passive-recall/config` — body `{ config: PassiveRecallConfig }`.
pub async fn kb_passive_recall_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<knowledge::PassiveRecallConfig>>,
) -> Result<Json<knowledge::PassiveRecallConfig>, AppError> {
    Ok(Json(service::set_passive_recall_config(
        body.config,
        "http",
    )?))
}

/// `GET /api/knowledge/{kb_id}/note/resolve?reference=` — resolve a `[[ ]]` ref
/// to a note read result (for `![[ ]]` transclusion). Body is `null` (not a 404)
/// when the ref is broken, so the client treats it identically to the Tauri path.
pub async fn kb_note_read_ref(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNoteRefQuery>,
) -> Result<Json<Option<NoteReadResult>>, AppError> {
    Ok(Json(service::note_read_ref(&kb_id, &q.reference)?))
}

/// `GET /api/knowledge/search?query=&kbId=&limit=`
pub async fn kb_search(
    Query(q): Query<KbSearchQuery>,
) -> Result<Json<Vec<NoteSearchHit>>, AppError> {
    let limit = q.limit.unwrap_or(20).clamp(1, 100) as usize;
    Ok(Json(service::search(q.kb_id.as_deref(), &q.query, limit)?))
}

// ── File preview plane (pure owner) ─────────────────────────────

/// `GET /api/knowledge/{kb_id}/files/read?path=`
pub async fn kb_file_read(
    Path(kb_id): Path<String>,
    Query(q): Query<KbFileQuery>,
) -> Result<Json<FileTextContent>, AppError> {
    let path = q.path.unwrap_or_default();
    let res = run(move || {
        let s = WorkspaceScope::for_knowledge(&kb_id)?;
        filesystem::project_read_text(&s, &path)
    })
    .await?;
    Ok(Json(res))
}

/// `GET /api/knowledge/{kb_id}/files/extract?path=`
pub async fn kb_file_extract(
    Path(kb_id): Path<String>,
    Query(q): Query<KbFileQuery>,
) -> Result<Json<ExtractedContent>, AppError> {
    let path = q.path.unwrap_or_default();
    let res = run(move || {
        let s = WorkspaceScope::for_knowledge(&kb_id)?;
        filesystem::project_fs_extract(&s, &path)
    })
    .await?;
    Ok(Json(res))
}

/// `GET /api/knowledge/{kb_id}/files/raw?path=&download=`
pub async fn kb_file_raw(
    Path(kb_id): Path<String>,
    Query(q): Query<KbFileQuery>,
    request: Request,
) -> Result<Response, AppError> {
    let path = q.path.unwrap_or_default();
    let abs = run(move || {
        let s = WorkspaceScope::for_knowledge(&kb_id)?;
        s.resolve_existing(&path)
    })
    .await?;
    let mime = resolve_mime_for_path(
        &abs,
        MimeOpts {
            html_charset: false,
            sniff_fallback: true,
        },
    )
    .await;
    let disposition = safe_content_disposition(&abs, &mime, q.download.unwrap_or(0) == 1);
    let mut response = ServeFile::new(&abs)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve file: {e}")))?
        .into_response();
    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &mime,
            cache_secs: 0,
            disposition: &disposition,
            no_referrer: false,
        },
    );
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
