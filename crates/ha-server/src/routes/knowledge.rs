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
    self, service, Backlink, BrokenLink, CompileProposal, CompileProposalStatus, CompileRun,
    CompileStartInput, CreateKnowledgeBaseInput, GraphNodePosition, KbAccess, KbAttachment,
    KbChatThread, KnowledgeAgentCompileProposeInput, KnowledgeAgentExpandInput,
    KnowledgeAgentExpandResult, KnowledgeAgentReadInput, KnowledgeAgentReadResult,
    KnowledgeAgentSearchInput, KnowledgeAgentSearchResult, KnowledgeAgentSourcesInput,
    KnowledgeAgentSourcesResult, KnowledgeBase, KnowledgeBaseMeta,
    KnowledgeBrowserSourceImportInput, KnowledgeCompileConfig, KnowledgeEvidenceClaim,
    KnowledgeEvidenceCoverage, KnowledgeEvidenceRebuildResult, KnowledgeGraph, KnowledgeSource,
    KnowledgeSourceAssetKind, KnowledgeSourceAssetLink, KnowledgeSourceDiff,
    KnowledgeSourceExternalRawSyncResult, KnowledgeSourceImportBatchInput,
    KnowledgeSourceImportInput, KnowledgeSourceImportRun, KnowledgeSourceImportRunDetail,
    KnowledgeSourceImportSessionAttachmentInput, KnowledgeSourceOcrPage, KnowledgeSourceReadResult,
    KnowledgeSourceRefreshInput, KnowledgeSourceRefreshResult,
    KnowledgeSourceSimilarityDismissInput, KnowledgeSourceSimilarityGroup,
    KnowledgeSourceSimilarityResolveInput, KnowledgeSourceSimilarityResolveResult,
    KnowledgeSourceVersionHistory, Note, NoteReadResult, NoteSearchHit, NoteSourceRef,
    QueryFileInput, ReferenceableNote, RenameOutcome, SchemaIssue, SchemaProfile,
    UpdateKnowledgeBaseInput,
};
use ha_core::session::SessionMeta;

use super::file_serve::{
    apply_inline_media_headers, resolve_mime_for_path, safe_content_disposition, HeaderOpts,
    MimeOpts,
};
use crate::error::AppError;
use crate::AppContext;
use ha_core::blocking::run_blocking;

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
    #[serde(default)]
    pub model_override: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbRewriteLogBody {
    pub kb_id: String,
    #[serde(default)]
    pub note_path: Option<String>,
    pub instruction: String,
    #[serde(default)]
    pub model: Option<String>,
    pub chars_before: i64,
    pub chars_after: i64,
    pub accepted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbFileQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub download: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct KbSourceImportBody {
    pub input: KnowledgeSourceImportInput,
}

#[derive(Debug, Deserialize)]
pub struct KbBrowserSourceImportBody {
    pub input: KnowledgeBrowserSourceImportInput,
}

#[derive(Debug, Deserialize)]
pub struct KbSourceImportSessionAttachmentBody {
    pub input: KnowledgeSourceImportSessionAttachmentInput,
}

#[derive(Debug, Deserialize)]
pub struct KbSourceImportBatchBody {
    pub input: KnowledgeSourceImportBatchInput,
}

#[derive(Debug, Deserialize)]
pub struct KbSourceRefreshBody {
    pub input: KnowledgeSourceRefreshInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbSourceDiffQuery {
    pub to_source_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbSourceImportRunsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct KbCompileStartBody {
    pub input: CompileStartInput,
}

#[derive(Debug, Deserialize)]
pub struct KbQueryFileBody {
    pub input: QueryFileInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbCompileProposalQuery {
    pub run_id: Option<String>,
    pub status: Option<CompileProposalStatus>,
}

#[derive(Debug, Deserialize)]
pub struct KbNoteSourceRefsQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AgentInputBody<T> {
    Wrapped { input: T },
    Direct(T),
}

impl<T> AgentInputBody<T> {
    fn into_inner(self) -> T {
        match self {
            AgentInputBody::Wrapped { input } => input,
            AgentInputBody::Direct(input) => input,
        }
    }
}

// ── Registry CRUD ───────────────────────────────────────────────

/// `GET /api/knowledge`
pub async fn list_kbs(
    Query(q): Query<ListKbsQuery>,
) -> Result<Json<Vec<KnowledgeBaseMeta>>, AppError> {
    Ok(Json(
        run_blocking(move || service::list_kb_meta(q.include_archived.unwrap_or(false))).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}`
pub async fn get_kb(Path(kb_id): Path<String>) -> Result<Json<KnowledgeBase>, AppError> {
    let reg = registry()?;
    let kb = {
        let kb_id = kb_id.clone();
        run_blocking(move || reg.get(&kb_id)).await?
    }
    .ok_or_else(|| AppError::not_found(format!("knowledge base not found: {kb_id}")))?;
    Ok(Json(kb))
}

/// `POST /api/knowledge`
pub async fn create_kb(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateKbBody>,
) -> Result<Json<KnowledgeBase>, AppError> {
    let kb = {
        let reg = registry()?;
        run_blocking(move || reg.create(body.input)).await?
    };
    // Index + watch the new KB. External vaults may already hold a large
    // number of files worth tracking with real progress — route them through
    // the job-tracked reembed/reindex path (the same one the settings-page
    // rebuild uses) so the bind shows live file-level progress instead of a
    // silent scan. Internal KBs are guaranteed empty at creation, so the
    // cheap fire-and-forget path is enough and skips job overhead.
    if kb.root_dir.is_some() {
        if let Err(e) =
            knowledge::start_knowledge_reembed_job(Some(vec![kb.id.clone()]), "kb-create")
        {
            ha_core::app_warn!(
                "knowledge",
                "create_kb",
                "job-tracked scan failed, falling back to untracked reindex: {}",
                e
            );
            knowledge::index::spawn_reindex_kb(kb.id.clone(), true);
        }
    } else {
        knowledge::index::spawn_reindex_kb(kb.id.clone(), true);
    }
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
    let kb = {
        let reg = registry()?;
        run_blocking(move || reg.update(&kb_id, body.patch)).await?
    };
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
    let removed = {
        let kb_id = kb_id.clone();
        run_blocking(move || knowledge::delete_kb_cascade(&kb_id)).await?
    };
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
    run_blocking(move || {
        knowledge::start_knowledge_reembed_job(Some(vec![kb_id]), "manual-reindex")
    })
    .await?;
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

// ── Raw source inbox (Knowledge Compiler Phase 1) ────────────────

/// `POST /api/knowledge/{kb_id}/sources`
pub async fn kb_source_import(
    Path(kb_id): Path<String>,
    Json(body): Json<KbSourceImportBody>,
) -> Result<Json<KnowledgeSource>, AppError> {
    Ok(Json(service::source_import(&kb_id, body.input).await?))
}

/// `POST /api/knowledge/{kb_id}/sources/browser`
pub async fn kb_source_import_browser(
    Path(kb_id): Path<String>,
    Json(body): Json<KbBrowserSourceImportBody>,
) -> Result<Json<KnowledgeSource>, AppError> {
    Ok(Json(
        service::source_import_browser(&kb_id, body.input).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/session-attachment`
pub async fn kb_source_import_session_attachment(
    Path(kb_id): Path<String>,
    Json(body): Json<KbSourceImportSessionAttachmentBody>,
) -> Result<Json<KnowledgeSource>, AppError> {
    Ok(Json(
        service::source_import_session_attachment(&kb_id, body.input).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/batch`
pub async fn kb_source_import_batch(
    Path(kb_id): Path<String>,
    Json(body): Json<KbSourceImportBatchBody>,
) -> Result<Json<KnowledgeSourceImportRunDetail>, AppError> {
    Ok(Json(
        service::source_import_batch(&kb_id, body.input).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/import-runs`
pub async fn kb_source_import_runs_list(
    Path(kb_id): Path<String>,
    Query(query): Query<KbSourceImportRunsQuery>,
) -> Result<Json<Vec<KnowledgeSourceImportRun>>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_import_runs_list(&kb_id, query.limit)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/import-runs/{run_id}`
pub async fn kb_source_import_run_detail(
    Path((kb_id, run_id)): Path<(String, String)>,
) -> Result<Json<KnowledgeSourceImportRunDetail>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_import_run_detail(&kb_id, &run_id)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/import-runs/{run_id}/retry-failed`
pub async fn kb_source_import_retry_failed(
    Path((kb_id, run_id)): Path<(String, String)>,
) -> Result<Json<KnowledgeSourceImportRunDetail>, AppError> {
    Ok(Json(
        service::source_import_retry_failed(&kb_id, &run_id).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/{source_id}/ocr-pages`
pub async fn kb_source_ocr_pages(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<Vec<KnowledgeSourceOcrPage>>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_ocr_pages(&kb_id, &source_id)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/{source_id}/ocr-retry`
pub async fn kb_source_ocr_retry(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<KnowledgeSource>, AppError> {
    Ok(Json(service::source_ocr_retry(&kb_id, &source_id).await?))
}

/// `GET /api/knowledge/{kb_id}/sources/similar`
pub async fn kb_source_similarity_groups(
    Path(kb_id): Path<String>,
) -> Result<Json<Vec<KnowledgeSourceSimilarityGroup>>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_similarity_groups(&kb_id)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/similar/dismiss`
pub async fn kb_source_similarity_dismiss(
    Path(kb_id): Path<String>,
    Json(input): Json<KnowledgeSourceSimilarityDismissInput>,
) -> Result<Json<Vec<KnowledgeSourceSimilarityGroup>>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_similarity_dismiss(&kb_id, input)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/similar/resolve`
pub async fn kb_source_similarity_resolve(
    Path(kb_id): Path<String>,
    Json(input): Json<KnowledgeSourceSimilarityResolveInput>,
) -> Result<Json<KnowledgeSourceSimilarityResolveResult>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_similarity_resolve(&kb_id, input)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources`
pub async fn kb_source_list(
    Path(kb_id): Path<String>,
) -> Result<Json<Vec<KnowledgeSource>>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_list(&kb_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/{source_id}`
pub async fn kb_source_read(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<KnowledgeSourceReadResult>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_read(&kb_id, &source_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/{source_id}/assets/{asset_kind}/link`
pub async fn kb_source_asset_link(
    Path((kb_id, source_id, asset_kind)): Path<(String, String, String)>,
) -> Result<Json<Option<KnowledgeSourceAssetLink>>, AppError> {
    let kind = parse_source_asset_kind(&asset_kind)?;
    Ok(Json(
        run_blocking(move || service::source_asset_link(&kb_id, &source_id, kind)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/{source_id}/assets/{asset_kind}?download=`
pub async fn kb_source_asset_file(
    Path((kb_id, source_id, asset_kind)): Path<(String, String, String)>,
    Query(q): Query<KbFileQuery>,
    request: Request,
) -> Result<Response, AppError> {
    let kind = parse_source_asset_kind(&asset_kind)?;
    let asset = {
        let kb_id = kb_id.clone();
        let source_id = source_id.clone();
        run_blocking(move || knowledge::source::source_asset_file(&kb_id, &source_id, kind)).await?
    };
    let Some((link, abs)) = asset else {
        return Err(AppError::not_found("source asset not found"));
    };
    if !abs.exists() {
        return Err(AppError::not_found("source asset file is missing"));
    }
    let mime = if link.mime_type.trim().is_empty() {
        resolve_mime_for_path(
            &abs,
            MimeOpts {
                html_charset: false,
                sniff_fallback: true,
            },
        )
        .await
    } else {
        link.mime_type
    };
    let disposition = safe_content_disposition(&abs, &mime, q.download.unwrap_or(0) == 1);
    let mut response = ServeFile::new(&abs)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve source asset: {e}")))?
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
    Ok(response)
}

fn parse_source_asset_kind(raw: &str) -> Result<KnowledgeSourceAssetKind, AppError> {
    match raw {
        "original" => Ok(KnowledgeSourceAssetKind::Original),
        "thumbnail" => Ok(KnowledgeSourceAssetKind::Thumbnail),
        _ => Err(AppError::bad_request(
            "source asset kind must be original or thumbnail",
        )),
    }
}

/// `POST /api/knowledge/{kb_id}/sources/{source_id}/refresh`
pub async fn kb_source_refresh(
    Path((kb_id, source_id)): Path<(String, String)>,
    Json(body): Json<KbSourceRefreshBody>,
) -> Result<Json<KnowledgeSourceRefreshResult>, AppError> {
    Ok(Json(
        service::source_refresh(&kb_id, &source_id, body.input).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/{source_id}/versions`
pub async fn kb_source_versions(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<KnowledgeSourceVersionHistory>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_versions(&kb_id, &source_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/sources/{source_id}/diff?toSourceId=...`
pub async fn kb_source_diff(
    Path((kb_id, source_id)): Path<(String, String)>,
    Query(query): Query<KbSourceDiffQuery>,
) -> Result<Json<KnowledgeSourceDiff>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_diff(&kb_id, &source_id, &query.to_source_id)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/{source_id}/reextract`
pub async fn kb_source_reextract(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<KnowledgeSource>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_reextract(&kb_id, &source_id)).await?,
    ))
}

/// `DELETE /api/knowledge/{kb_id}/sources/{source_id}`
pub async fn kb_source_delete(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<bool>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_delete(&kb_id, &source_id)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/sources/sync-external-raw`
pub async fn kb_source_sync_external_raw(
    Path(kb_id): Path<String>,
) -> Result<Json<KnowledgeSourceExternalRawSyncResult>, AppError> {
    Ok(Json(
        run_blocking(move || service::source_sync_external_raw(&kb_id)).await?,
    ))
}

// ── Knowledge Compiler (Phase 2) ─────────────────────────────────

/// `POST /api/knowledge/{kb_id}/compile-runs`
pub async fn kb_compile_start(
    Path(kb_id): Path<String>,
    Json(body): Json<KbCompileStartBody>,
) -> Result<Json<CompileRun>, AppError> {
    Ok(Json(service::compile_start(&kb_id, body.input).await?))
}

/// `GET /api/knowledge/{kb_id}/compile-runs`
pub async fn kb_compile_runs_list(
    Path(kb_id): Path<String>,
) -> Result<Json<Vec<CompileRun>>, AppError> {
    Ok(Json(
        run_blocking(move || service::compile_runs_list(&kb_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/compile-runs/{run_id}`
pub async fn kb_compile_status(
    Path((kb_id, run_id)): Path<(String, String)>,
) -> Result<Json<CompileRun>, AppError> {
    let run = run_blocking(move || service::compile_status(&run_id)).await?;
    if run.kb_id != kb_id {
        return Err(anyhow::anyhow!("compile run not found in knowledge base").into());
    }
    Ok(Json(run))
}

/// `POST /api/knowledge/{kb_id}/compile-runs/{run_id}/cancel`
pub async fn kb_compile_run_cancel(
    Path((kb_id, run_id)): Path<(String, String)>,
) -> Result<Json<CompileRun>, AppError> {
    let run = {
        let run_id = run_id.clone();
        run_blocking(move || service::compile_status(&run_id)).await?
    };
    if run.kb_id != kb_id {
        return Err(anyhow::anyhow!("compile run not found in knowledge base").into());
    }
    Ok(Json(
        run_blocking(move || service::compile_run_cancel(&run_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/compile-proposals`
pub async fn kb_compile_proposals_list(
    Path(kb_id): Path<String>,
    Query(q): Query<KbCompileProposalQuery>,
) -> Result<Json<Vec<CompileProposal>>, AppError> {
    Ok(Json(
        run_blocking(move || {
            service::compile_proposals_list(&kb_id, q.run_id.as_deref(), q.status)
        })
        .await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/compile-proposals/{id}/approve`
pub async fn kb_compile_proposal_approve(
    Path((kb_id, id)): Path<(String, i64)>,
) -> Result<Json<CompileProposal>, AppError> {
    run_blocking(move || ensure_compile_proposal_in_kb(&kb_id, id)).await?;
    Ok(Json(service::compile_proposal_approve(id).await?))
}

/// `POST /api/knowledge/{kb_id}/compile-proposals/{id}/reject`
pub async fn kb_compile_proposal_reject(
    Path((kb_id, id)): Path<(String, i64)>,
) -> Result<Json<bool>, AppError> {
    let removed = run_blocking(move || -> Result<_, AppError> {
        ensure_compile_proposal_in_kb(&kb_id, id)?;
        Ok(service::compile_proposal_reject(id)?)
    })
    .await?;
    Ok(Json(removed))
}

/// `POST /api/knowledge/{kb_id}/query-file`
pub async fn kb_query_file(
    Path(kb_id): Path<String>,
    Json(body): Json<KbQueryFileBody>,
) -> Result<Json<CompileProposal>, AppError> {
    Ok(Json(
        run_blocking(move || service::query_file(&kb_id, body.input)).await?,
    ))
}

fn ensure_compile_proposal_in_kb(kb_id: &str, id: i64) -> Result<(), AppError> {
    let found = service::compile_proposals_list(kb_id, None, None)?
        .into_iter()
        .any(|proposal| proposal.id == id);
    if !found {
        return Err(anyhow::anyhow!("compile proposal not found in knowledge base").into());
    }
    Ok(())
}

/// `GET /api/knowledge/{kb_id}/schema-profile`
pub async fn kb_schema_profile(Path(kb_id): Path<String>) -> Result<Json<SchemaProfile>, AppError> {
    Ok(Json(
        run_blocking(move || service::schema_profile(&kb_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/schema-issues`
pub async fn kb_schema_issues(
    Path(kb_id): Path<String>,
) -> Result<Json<Vec<SchemaIssue>>, AppError> {
    Ok(Json(
        run_blocking(move || service::schema_issues(&kb_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/note/source-refs?path=...`
pub async fn kb_note_source_refs(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNoteSourceRefsQuery>,
) -> Result<Json<Vec<NoteSourceRef>>, AppError> {
    Ok(Json(
        run_blocking(move || service::note_source_refs(&kb_id, &q.path)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/evidence/coverage`
pub async fn kb_evidence_coverage(
    Path(kb_id): Path<String>,
) -> Result<Json<KnowledgeEvidenceCoverage>, AppError> {
    Ok(Json(
        run_blocking(move || service::evidence_coverage(&kb_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/evidence/sources/{source_id}/claims`
pub async fn kb_evidence_source_claims(
    Path((kb_id, source_id)): Path<(String, String)>,
) -> Result<Json<Vec<KnowledgeEvidenceClaim>>, AppError> {
    Ok(Json(
        run_blocking(move || service::evidence_source_claims(&kb_id, &source_id)).await?,
    ))
}

/// `POST /api/knowledge/{kb_id}/evidence/rebuild`
pub async fn kb_evidence_rebuild(
    Path(kb_id): Path<String>,
) -> Result<Json<KnowledgeEvidenceRebuildResult>, AppError> {
    Ok(Json(
        run_blocking(move || service::evidence_rebuild(&kb_id)).await?,
    ))
}

// ── Phase 6 external-agent API ─────────────────────────────────

/// `POST /api/knowledge/agent/search` — stable `knowledge.search` surface.
pub async fn knowledge_agent_search(
    Json(body): Json<AgentInputBody<KnowledgeAgentSearchInput>>,
) -> Result<Json<KnowledgeAgentSearchResult>, AppError> {
    Ok(Json(
        run_blocking(move || knowledge::agent_api::search(body.into_inner())).await?,
    ))
}

/// `POST /api/knowledge/agent/read` — stable `knowledge.read` surface.
pub async fn knowledge_agent_read(
    Json(body): Json<AgentInputBody<KnowledgeAgentReadInput>>,
) -> Result<Json<KnowledgeAgentReadResult>, AppError> {
    Ok(Json(
        run_blocking(move || knowledge::agent_api::read(body.into_inner())).await?,
    ))
}

/// `POST /api/knowledge/agent/expand` — stable `knowledge.expand` surface.
pub async fn knowledge_agent_expand(
    Json(body): Json<AgentInputBody<KnowledgeAgentExpandInput>>,
) -> Result<Json<KnowledgeAgentExpandResult>, AppError> {
    Ok(Json(
        run_blocking(move || knowledge::agent_api::expand(body.into_inner())).await?,
    ))
}

/// `POST /api/knowledge/agent/sources` — stable `knowledge.sources` surface.
pub async fn knowledge_agent_sources(
    Json(body): Json<AgentInputBody<KnowledgeAgentSourcesInput>>,
) -> Result<Json<KnowledgeAgentSourcesResult>, AppError> {
    Ok(Json(
        run_blocking(move || knowledge::agent_api::sources(body.into_inner())).await?,
    ))
}

/// `POST /api/knowledge/agent/compile/propose` — stable
/// `knowledge.compile.propose` surface. Starts a normal compile run that only
/// creates Review Diff proposals.
pub async fn knowledge_agent_compile_propose(
    Json(body): Json<AgentInputBody<KnowledgeAgentCompileProposeInput>>,
) -> Result<Json<CompileRun>, AppError> {
    Ok(Json(
        knowledge::agent_api::compile_propose(body.into_inner()).await?,
    ))
}

// ── Access bindings ─────────────────────────────────────────────

/// `POST /api/knowledge/attach`
pub async fn attach_kb(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<AttachBody>,
) -> Result<Json<bool>, AppError> {
    let access = KbAccess::from_str_lenient(body.access.as_deref().unwrap_or("read"));
    let reg = registry()?;
    {
        let session_id = body.session_id.clone();
        let project_id = body.project_id.clone();
        let kb_id = body.kb_id.clone();
        run_blocking(move || -> Result<(), AppError> {
            if let Some(sid) = &session_id {
                reg.attach_session(sid, &kb_id, access)?;
            } else if let Some(pid) = &project_id {
                reg.attach_project(pid, &kb_id, access)?;
            } else {
                return Err(AppError::bad_request(
                    "attach requires sessionId or projectId",
                ));
            }
            Ok(())
        })
        .await?;
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
    {
        let session_id = body.session_id.clone();
        let project_id = body.project_id.clone();
        let kb_id = body.kb_id.clone();
        run_blocking(move || -> Result<(), AppError> {
            if let Some(sid) = &session_id {
                reg.detach_session(sid, &kb_id)?;
            } else if let Some(pid) = &project_id {
                reg.detach_project(pid, &kb_id)?;
            } else {
                return Err(AppError::bad_request(
                    "detach requires sessionId or projectId",
                ));
            }
            Ok(())
        })
        .await?;
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
    let out = run_blocking(move || -> Result<Vec<KbAttachment>, AppError> {
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
        Ok(out)
    })
    .await?;
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
    let out = run_blocking(move || -> Result<Vec<KbAttachment>, AppError> {
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
        Ok(out)
    })
    .await?;
    Ok(Json(out))
}

// ── Notes (owner plane) ─────────────────────────────────────────

/// `GET /api/knowledge/{kb_id}/notes`
pub async fn list_kb_notes(Path(kb_id): Path<String>) -> Result<Json<Vec<Note>>, AppError> {
    Ok(Json(
        run_blocking(move || service::list_notes(&kb_id)).await?,
    ))
}

/// `GET /api/knowledge/{kb_id}/note?path=`
pub async fn kb_note_read(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNotePathQuery>,
) -> Result<Json<NoteReadResult>, AppError> {
    Ok(Json(
        run_blocking(move || service::note_read(&kb_id, &q.path)).await?,
    ))
}

/// `PUT /api/knowledge/{kb_id}/note`
pub async fn kb_note_save(
    Path(kb_id): Path<String>,
    Json(body): Json<KbNoteSaveBody>,
) -> Result<Json<String>, AppError> {
    ensure_writes_allowed()?;
    let hash = run_blocking(move || {
        service::note_save(
            &kb_id,
            &body.path,
            &body.content,
            body.expected_file_hash.as_deref(),
            body.create_only.unwrap_or(false),
        )
    })
    .await?;
    Ok(Json(hash))
}

/// `DELETE /api/knowledge/{kb_id}/note?path=`
pub async fn kb_note_delete(
    Path(kb_id): Path<String>,
    Query(q): Query<KbNotePathQuery>,
) -> Result<Json<bool>, AppError> {
    ensure_writes_allowed()?;
    run_blocking(move || service::note_delete(&kb_id, &q.path)).await?;
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
    Ok(Json(
        run_blocking(move || service::list_tags(&kb_id)).await?,
    ))
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
    Ok(Json(
        run_blocking(move || {
            ha_core::knowledge::set_knowledge_embedding_default(&body.model_config_id, "http")
        })
        .await?,
    ))
}

/// `POST /api/knowledge/embedding/disable`
pub async fn knowledge_embedding_disable(
) -> Result<Json<ha_core::memory::EmbeddingSelectionState>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::knowledge::disable_knowledge_embedding("http")).await?,
    ))
}

/// `POST /api/knowledge/embedding/rebuild` — force a full rebuild of every KB
/// under the active model (no same-signature short-circuit).
pub async fn knowledge_embedding_rebuild() -> Result<Json<bool>, AppError> {
    run_blocking(move || ha_core::knowledge::start_knowledge_reembed_job(None, "manual-rebuild"))
        .await?;
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
    Ok(Json(
        run_blocking(move || {
            ha_core::knowledge::set_chunk_config(body.max_chars, body.overlap_chars, "http")
        })
        .await?,
    ))
}

/// `GET /api/knowledge/search-config` — current hybrid-search ranking parameters.
pub async fn knowledge_search_config_get(
) -> Result<Json<ha_core::knowledge::KnowledgeSearchConfig>, AppError> {
    Ok(Json(ha_core::knowledge::get_search_config()))
}

#[derive(Deserialize)]
pub struct KnowledgeSearchSetBody {
    pub config: ha_core::knowledge::KnowledgeSearchConfig,
}

/// `POST /api/knowledge/search-config` — save search ranking parameters (clamped,
/// no reindex). Send default values to restore defaults.
pub async fn knowledge_search_config_set(
    Json(body): Json<KnowledgeSearchSetBody>,
) -> Result<Json<ha_core::knowledge::KnowledgeSearchConfig>, AppError> {
    Ok(Json(ha_core::knowledge::set_search_config(
        body.config,
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

// ── Knowledge-space sidebar chat threads ────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbChatThreadQuery {
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbChatThreadsListQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// `GET /api/knowledge/{kb_id}/chat/thread?note=` — latest chat thread anchored
/// to a note (default-load target). `null` body when none exists.
pub async fn kb_chat_thread_get(
    Path(kb_id): Path<String>,
    Query(q): Query<KbChatThreadQuery>,
) -> Result<Json<Option<SessionMeta>>, AppError> {
    Ok(Json(service::kb_chat_thread_latest(
        &kb_id,
        q.note.as_deref(),
    )?))
}

/// `GET /api/knowledge/{kb_id}/chat/threads?query=` — history picker list.
pub async fn kb_chat_threads_list(
    Path(kb_id): Path<String>,
    Query(q): Query<KbChatThreadsListQuery>,
) -> Result<Json<Vec<KbChatThread>>, AppError> {
    Ok(Json(service::kb_chat_threads_list(
        &kb_id,
        q.query.as_deref(),
        q.limit,
        q.offset,
    )?))
}

/// `POST /api/knowledge/ai/rewrite` — AI rewrite of a text selection (WS9). Returns
/// rewritten Markdown; the client diffs it and the user saves through `note_save`.
pub async fn kb_ai_rewrite(Json(body): Json<KbAiRewriteBody>) -> Result<Json<String>, AppError> {
    Ok(Json(
        service::ai_rewrite(
            &body.text,
            &body.instruction,
            body.model_override.as_deref(),
        )
        .await?,
    ))
}

/// `POST /api/knowledge/rewrite/log` — record a quick-rewrite outcome for stats.
pub async fn kb_rewrite_log(Json(body): Json<KbRewriteLogBody>) -> Result<Json<bool>, AppError> {
    service::log_quick_rewrite(
        &body.kb_id,
        body.note_path.as_deref(),
        &body.instruction,
        body.model.as_deref(),
        body.chars_before,
        body.chars_after,
        body.accepted,
    );
    Ok(Json(true))
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

/// `GET /api/knowledge/media-retention/config`
pub async fn knowledge_media_retention_config_get(
) -> Result<Json<knowledge::KnowledgeMediaRetentionConfig>, AppError> {
    Ok(Json(service::get_media_retention_config()))
}

/// `POST /api/knowledge/media-retention/config` — body `{ config: KnowledgeMediaRetentionConfig }`.
pub async fn knowledge_media_retention_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<knowledge::KnowledgeMediaRetentionConfig>>,
) -> Result<Json<knowledge::KnowledgeMediaRetentionConfig>, AppError> {
    Ok(Json(service::set_media_retention_config(
        body.config,
        "http",
    )?))
}

/// `GET /api/knowledge/compile/config`
pub async fn knowledge_compile_config_get() -> Result<Json<KnowledgeCompileConfig>, AppError> {
    Ok(Json(service::get_compile_config()))
}

/// `POST /api/knowledge/compile/config` — body `{ config: KnowledgeCompileConfig }`.
pub async fn knowledge_compile_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<KnowledgeCompileConfig>>,
) -> Result<Json<KnowledgeCompileConfig>, AppError> {
    Ok(Json(service::set_compile_config(body.config, "http")?))
}

/// `GET /api/knowledge/vision/config`
pub async fn knowledge_vision_config_get(
) -> Result<Json<knowledge::KnowledgeVisionConfig>, AppError> {
    Ok(Json(service::get_vision_config()))
}

/// `POST /api/knowledge/vision/config` — body `{ config: KnowledgeVisionConfig }`.
pub async fn knowledge_vision_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<knowledge::KnowledgeVisionConfig>>,
) -> Result<Json<knowledge::KnowledgeVisionConfig>, AppError> {
    Ok(Json(service::set_vision_config(body.config, "http").await?))
}

/// `GET /api/knowledge/note-tools/config`
pub async fn note_tools_config_get() -> Result<Json<knowledge::NoteToolsConfig>, AppError> {
    Ok(Json(service::get_note_tools_config()))
}

/// `POST /api/knowledge/note-tools/config` — body `{ config: NoteToolsConfig }`.
pub async fn note_tools_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<knowledge::NoteToolsConfig>>,
) -> Result<Json<knowledge::NoteToolsConfig>, AppError> {
    Ok(Json(
        service::set_note_tools_config(body.config, "http").await?,
    ))
}

// ── Sprite / inspiration mode ───────────────────────────────────

#[derive(Deserialize)]
pub struct SpriteObserveBody {
    pub params: ha_core::sprite::SpriteObserveParams,
}

/// `POST /api/knowledge/sprite/observe` — body `{ params: SpriteObserveParams }`.
/// Fire-and-forget: the suggestion (if any) arrives via the `sprite:suggestion`
/// event stream.
pub async fn kb_sprite_observe(Json(body): Json<SpriteObserveBody>) -> Result<Json<()>, AppError> {
    tokio::spawn(async move {
        let _ = ha_core::sprite::observe_and_maybe_speak(body.params).await;
    });
    Ok(Json(()))
}

/// `GET /api/knowledge/sprite/config`
pub async fn sprite_config_get() -> Result<Json<ha_core::sprite::SpriteConfig>, AppError> {
    Ok(Json(ha_core::sprite::get_config()))
}

/// `POST /api/knowledge/sprite/config` — body `{ config: SpriteConfig }`.
pub async fn sprite_config_set(
    Json(body): Json<crate::routes::config::ConfigBody<ha_core::sprite::SpriteConfig>>,
) -> Result<Json<ha_core::sprite::SpriteConfig>, AppError> {
    Ok(Json(ha_core::sprite::set_config(body.config, "http")?))
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
