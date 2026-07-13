use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::blocking::run_blocking;

use crate::error::AppError;

// ── Query / Body Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub scope: Option<String>,
    pub agent_id: Option<String>,
    pub types: Option<String>,
    pub sources: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMemoryBody {
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountQuery {
    pub scope: Option<String>,
    pub agent_id: Option<String>,
    pub sources: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsQuery {
    pub scope: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHistoryQueryParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub query: Option<String>,
    pub actions: Option<String>,
    pub memory_types: Option<String>,
    pub sources: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportPromptQuery {
    pub locale: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairMemoryBody {
    pub action: ha_core::memory::MemoryRepairAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbSnapshotRestorePreviewBody {
    pub snapshot_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddEpisodeBody {
    pub episode: ha_core::memory::NewMemoryEpisode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEpisodeBody {
    pub patch: ha_core::memory::MemoryEpisodePatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodePageBody {
    pub query: ha_core::memory::MemoryEpisodeQuery,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddProcedureBody {
    pub procedure: ha_core::memory::NewMemoryProcedure,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProcedureBody {
    pub patch: ha_core::memory::MemoryProcedurePatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteEpisodeBody {
    #[serde(default)]
    pub options: Option<ha_core::memory::PromoteEpisodeOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcedurePageBody {
    pub query: ha_core::memory::MemoryProcedureQuery,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceHistoryPageBody {
    pub query: ha_core::memory::MemoryExperienceHistoryQuery,
}

/// `GET /api/claims` query. Scope is primitive `scopeType` + `scopeId` (not a
/// JSON object) so the filter can never silently degrade over the query
/// transport; an invalid `scopeType` is a 400 (not a fail-open to "all").
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListClaimsQuery {
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub status: Option<String>,
    pub claim_type: Option<String>,
    pub confidence_source: Option<String>,
    pub evidence_class: Option<String>,
    pub evidence_source_type: Option<String>,
    pub query: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimConflictsQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimConflictSummariesBody {
    #[serde(default)]
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimEvidenceSummariesBody {
    #[serde(default)]
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimReviewSummariesBody {
    #[serde(default)]
    pub ids: Vec<String>,
}

/// `POST /api/claims/{id}/forget` body — both fields optional (`{}` = archive).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgetClaimBody {
    pub permanent: Option<bool>,
    pub note: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────

fn claim_list_filter_from_query(
    q: ListClaimsQuery,
) -> Result<ha_core::memory::claims::ClaimListFilter, AppError> {
    // Strict scope parse: invalid scopeType → 400 (no silent fail-open to all).
    let scope =
        ha_core::memory::claims::parse_claim_scope(q.scope_type.as_deref(), q.scope_id.as_deref())
            .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(ha_core::memory::claims::ClaimListFilter {
        scope,
        status: q.status,
        claim_type: q.claim_type,
        confidence_source: q.confidence_source,
        evidence_class: q.evidence_class,
        evidence_source_type: q.evidence_source_type,
        query: q.query,
        sort: q.sort,
        limit: q.limit,
        offset: q.offset,
    })
}

fn get_backend() -> Result<&'static std::sync::Arc<dyn ha_core::memory::MemoryBackend>, AppError> {
    ha_core::get_memory_backend()
        .ok_or_else(|| AppError::internal("Memory backend not initialized"))
}

/// Body wrapper for `memory_add` — frontend ships `{ entry: <NewMemory> }`.
#[derive(Debug, Deserialize)]
pub struct AddMemoryBody {
    pub entry: ha_core::memory::NewMemory,
}

/// Body wrapper for `memory_search` — frontend ships `{ query: <MemorySearchQuery> }`.
#[derive(Debug, Deserialize)]
pub struct SearchMemoryBody {
    pub query: ha_core::memory::MemorySearchQuery,
}

/// Parse scope from query params: explicit `scope` JSON or shorthand `agent_id`.
fn parse_scope(
    scope: &Option<String>,
    agent_id: &Option<String>,
) -> Option<ha_core::memory::MemoryScope> {
    if let Some(s) = scope {
        serde_json::from_str(s).ok()
    } else {
        agent_id
            .as_ref()
            .map(|id| ha_core::memory::MemoryScope::Agent { id: id.clone() })
    }
}

/// Parse memory types from comma-separated string.
fn parse_types(types: &Option<String>) -> Option<Vec<ha_core::memory::MemoryType>> {
    types.as_ref().map(|t| {
        t.split(',')
            .map(|s| ha_core::memory::MemoryType::from_str(s.trim()))
            .collect()
    })
}

/// Parse memory sources from a comma-separated string.
fn parse_sources(sources: &Option<String>) -> Option<Vec<String>> {
    sources.as_ref().map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    })
}

/// Parse memory history actions from a comma-separated string. Unlike the
/// legacy type parser, audit filters are strict so a typo cannot silently show
/// an unrelated audit stream.
fn parse_history_actions(
    actions: &Option<String>,
) -> Result<Option<Vec<ha_core::memory::MemoryHistoryAction>>, AppError> {
    let Some(raw) = actions else {
        return Ok(None);
    };
    let mut parsed = Vec::new();
    for action in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let action = match action {
            "add" => ha_core::memory::MemoryHistoryAction::Add,
            "update" => ha_core::memory::MemoryHistoryAction::Update,
            "delete" => ha_core::memory::MemoryHistoryAction::Delete,
            "pin" => ha_core::memory::MemoryHistoryAction::Pin,
            "unpin" => ha_core::memory::MemoryHistoryAction::Unpin,
            "import" => ha_core::memory::MemoryHistoryAction::Import,
            other => {
                return Err(AppError::bad_request(format!(
                    "invalid memory history action: {other}"
                )));
            }
        };
        parsed.push(action);
    }
    Ok(Some(parsed))
}

// ── Handlers ────────────────────────────────────────────────────

/// `GET /api/claims/schema` -- read-only structured claim schema metadata.
pub async fn claim_schema_metadata(
) -> Result<Json<ha_core::memory::claims::ClaimSchemaMetadata>, AppError> {
    Ok(Json(ha_core::memory::claims::claim_schema_metadata()))
}

/// `POST /api/memory` -- add a new memory entry.
pub async fn add_memory(Json(body): Json<AddMemoryBody>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let id = run_blocking(move || backend.add(body.entry)).await?;
    ha_core::memory::emit_memory_changed("add", Some(id), None);
    Ok(Json(json!({ "id": id })))
}

/// `PUT /api/memory/{id}` -- update an existing memory entry.
pub async fn update_memory(
    Path(id): Path<i64>,
    Json(body): Json<UpdateMemoryBody>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    run_blocking(move || backend.update(id, &body.content, &body.tags)).await?;
    ha_core::memory::emit_memory_changed("update", Some(id), None);
    Ok(Json(json!({ "updated": true })))
}

/// `DELETE /api/memory/{id}` -- delete a memory entry.
pub async fn delete_memory(Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    run_blocking(move || backend.delete(id)).await?;
    ha_core::memory::emit_memory_changed("delete", Some(id), None);
    Ok(Json(json!({ "deleted": true })))
}

/// `GET /api/memory/{id}` -- get a single memory entry.
pub async fn get_memory(Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let entry = run_blocking(move || backend.get(id))
        .await?
        .ok_or_else(|| AppError::not_found(format!("memory not found: {}", id)))?;
    Ok(Json(serde_json::to_value(entry)?))
}

/// `GET /api/memory` -- list memories with optional filtering.
pub async fn list_memories(
    Query(q): Query<ListMemoryQuery>,
) -> Result<Json<Vec<ha_core::memory::MemoryEntry>>, AppError> {
    let backend = get_backend()?;
    let scope = parse_scope(&q.scope, &q.agent_id);
    let types = parse_types(&q.types);
    let sources = parse_sources(&q.sources);
    let entries = run_blocking(move || {
        backend.list_filtered(
            scope.as_ref(),
            types.as_deref(),
            sources.as_deref(),
            q.limit.unwrap_or(50),
            q.offset.unwrap_or(0),
        )
    })
    .await?;
    Ok(Json(entries))
}

/// `GET /api/memory/history` -- durable owner audit stream for legacy memory.
pub async fn memory_history(
    Query(q): Query<MemoryHistoryQueryParams>,
) -> Result<Json<Vec<ha_core::memory::MemoryHistoryRecord>>, AppError> {
    let backend = get_backend()?;
    let actions = parse_history_actions(&q.actions)?;
    let memory_types = parse_types(&q.memory_types);
    let sources = parse_sources(&q.sources);
    let query = ha_core::memory::MemoryHistoryQuery {
        query: q.query,
        actions,
        memory_types,
        sources,
        limit: q.limit,
        offset: q.offset,
    };
    Ok(Json(
        run_blocking(move || backend.history_filtered(&query)).await?,
    ))
}

/// `GET /api/memory/history/page` -- durable owner audit page with total count.
pub async fn memory_history_page(
    Query(q): Query<MemoryHistoryQueryParams>,
) -> Result<Json<ha_core::memory::MemoryHistoryListResponse>, AppError> {
    let backend = get_backend()?;
    let actions = parse_history_actions(&q.actions)?;
    let memory_types = parse_types(&q.memory_types);
    let sources = parse_sources(&q.sources);
    let query = ha_core::memory::MemoryHistoryQuery {
        query: q.query,
        actions,
        memory_types,
        sources,
        limit: q.limit,
        offset: q.offset,
    };
    Ok(Json(
        run_blocking(move || backend.history_filtered_page(&query)).await?,
    ))
}

/// `GET /api/memory/audit/page` -- owner-only unified audit page across
/// legacy memory history, Experience/Workflow history, and claim decisions.
pub async fn memory_audit_page(
    Query(q): Query<ha_core::memory::MemoryAuditPageQuery>,
) -> Result<Json<ha_core::memory::MemoryAuditPageResponse>, AppError> {
    if let Some(action) = q.action.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !matches!(
            action,
            "all" | "add" | "update" | "delete" | "pin" | "unpin" | "import"
        ) {
            return Err(AppError::bad_request(format!(
                "invalid memory audit action: {action}"
            )));
        }
    }
    Ok(Json(
        run_blocking(move || ha_core::memory::memory_audit_page(q)).await?,
    ))
}

/// `GET /api/claims` -- list structured claims (next-gen Dreaming, read-only).
pub async fn list_claims(
    Query(q): Query<ListClaimsQuery>,
) -> Result<Json<Vec<ha_core::memory::claims::ClaimRecord>>, AppError> {
    let filter = claim_list_filter_from_query(q)?;
    let claims = run_blocking(move || ha_core::memory::claims::list_claims(filter)).await?;
    Ok(Json(claims))
}

/// `GET /api/claims/page` -- page structured claims with an exact total count.
pub async fn list_claims_page(
    Query(q): Query<ListClaimsQuery>,
) -> Result<Json<ha_core::memory::claims::ClaimListPage>, AppError> {
    let filter = claim_list_filter_from_query(q)?;
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::list_claims_page(filter)).await?,
    ))
}

/// `GET /api/claims/{id}` -- a single claim plus its evidence + legacy-memory
/// links. Returns `null` when the id is unknown (mirrors the Tauri command).
pub async fn get_claim(
    Path(id): Path<String>,
) -> Result<Json<Option<ha_core::memory::claims::ClaimDetail>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::get_claim(&id)).await?,
    ))
}

/// `GET /api/claims/{id}/graph` -- read-only entity context graph around one
/// claim. Returns an empty graph for an unknown id.
pub async fn claim_graph(
    Path(id): Path<String>,
    Query(q): Query<ClaimConflictsQuery>,
) -> Result<Json<ha_core::memory::claims::ClaimGraphProjection>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::claim_graph(&id, q.limit)).await?,
    ))
}

/// `GET /api/claims/{id}/conflicts` -- owner-plane conflict candidates for one
/// claim. Returns an empty list for an unknown id.
pub async fn claim_conflicts(
    Path(id): Path<String>,
    Query(q): Query<ClaimConflictsQuery>,
) -> Result<Json<Vec<ha_core::memory::claims::ClaimRecord>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::list_claim_conflicts(&id, q.limit)).await?,
    ))
}

/// `GET /api/claims/{id}/conflict-details` -- bounded conflict details for the
/// Review Inbox evidence matrix.
pub async fn claim_conflict_details(
    Path(id): Path<String>,
    Query(q): Query<ClaimConflictsQuery>,
) -> Result<Json<Vec<ha_core::memory::claims::ClaimDetail>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::list_claim_conflict_details(&id, q.limit))
            .await?,
    ))
}

/// `POST /api/claims/conflict-summaries` -- batch owner-plane conflict counts
/// for Review Inbox list grouping. Empty / unknown ids return no-op summaries.
pub async fn claim_conflict_summaries(
    Json(body): Json<ClaimConflictSummariesBody>,
) -> Result<Json<Vec<ha_core::memory::claims::ClaimConflictSummary>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::list_claim_conflict_summaries(&body.ids))
            .await?,
    ))
}

/// `POST /api/claims/evidence-summaries` -- batch owner-plane evidence trust
/// counts for claim list rows. Full evidence stays behind `GET /api/claims/{id}`.
pub async fn claim_evidence_summaries(
    Json(body): Json<ClaimEvidenceSummariesBody>,
) -> Result<Json<Vec<ha_core::memory::claims::ClaimEvidenceSummary>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::list_claim_evidence_summaries(&body.ids))
            .await?,
    ))
}

/// `POST /api/claims/review-summaries` -- batch owner-plane Review Inbox risk
/// summaries. Full evidence and conflicts stay behind the detail endpoints.
pub async fn claim_review_summaries(
    Json(body): Json<ClaimReviewSummariesBody>,
) -> Result<Json<Vec<ha_core::memory::claims::ClaimReviewSummary>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::list_claim_review_summaries(&body.ids))
            .await?,
    ))
}

/// `PATCH /api/claims/{id}` -- user correction (Lucid Review, design §5.2):
/// edit content/triple/tags, change status (approve / reject / mark-outdated),
/// move scope, or pin/unpin. The path id is authoritative (overrides any
/// `claimId` in the body). Owner plane.
pub async fn update_claim(
    Path(id): Path<String>,
    Json(mut body): Json<ha_core::memory::claims::ClaimUpdate>,
) -> Result<Json<ha_core::memory::claims::ClaimActionOutcome>, AppError> {
    body.claim_id = id;
    Ok(Json(
        run_blocking(move || ha_core::memory::claims::update_claim(body)).await?,
    ))
}

/// `POST /api/claims/{id}/forget` -- forget a claim (design §5.3). Body:
/// `{ permanent?: bool, note?: string }`. `permanent=false` archives (kept as
/// an audit trail); `true` hard-deletes the claim graph. Owner plane.
pub async fn forget_claim(
    Path(id): Path<String>,
    Json(body): Json<ForgetClaimBody>,
) -> Result<Json<ha_core::memory::claims::ClaimActionOutcome>, AppError> {
    Ok(Json(
        run_blocking(move || {
            ha_core::memory::claims::forget_claim(
                &id,
                body.permanent.unwrap_or(false),
                body.note.as_deref(),
            )
        })
        .await?,
    ))
}

/// `GET /api/memory/backfill/plan` -- dry-run existing-memory backfill plan
/// (owner plane). Writes nothing; the full-table scan runs on a blocking thread.
pub async fn memory_backfill_plan() -> Result<Json<ha_core::memory::claims::BackfillPlan>, AppError>
{
    let plan = tokio::task::spawn_blocking(ha_core::memory::claims::plan_backfill)
        .await
        .map_err(|e| AppError::internal(format!("backfill plan task failed: {e}")))??;
    Ok(Json(plan))
}

/// `POST /api/memory/backfill/apply` -- apply the existing-memory backfill
/// (owner plane): deterministic re-scan → claim + memory evidence + detached
/// link per not-yet-linked memory.
pub async fn memory_backfill_apply(
) -> Result<Json<ha_core::memory::claims::BackfillApplyResult>, AppError> {
    let result = tokio::task::spawn_blocking(ha_core::memory::claims::apply_backfill)
        .await
        .map_err(|e| AppError::internal(format!("backfill apply task failed: {e}")))??;
    Ok(Json(result))
}

/// `POST /api/memory/search` -- semantic search over memories.
pub async fn search_memories(
    Json(body): Json<SearchMemoryBody>,
) -> Result<Json<Vec<ha_core::memory::MemoryEntry>>, AppError> {
    let backend = get_backend()?;
    let results = run_blocking(move || backend.search(&body.query)).await?;
    Ok(Json(results))
}

/// `GET /api/memory/count` -- get total memory count.
pub async fn memory_count(Query(q): Query<CountQuery>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let scope = parse_scope(&q.scope, &q.agent_id);
    let sources = parse_sources(&q.sources);
    let count =
        run_blocking(move || backend.count_filtered(scope.as_ref(), sources.as_deref())).await?;
    Ok(Json(json!({ "count": count })))
}

/// `GET /api/memory/stats` -- get memory statistics.
pub async fn memory_stats(
    Query(q): Query<StatsQuery>,
) -> Result<Json<ha_core::memory::MemoryStats>, AppError> {
    let backend = get_backend()?;
    let scope = parse_scope(&q.scope, &q.agent_id);
    let stats = run_blocking(move || backend.stats(scope.as_ref())).await?;
    Ok(Json(stats))
}

/// `GET /api/memory/health` -- read-only diagnostics for memory storage,
/// indexes, embedding coverage, and claim graph consistency.
pub async fn memory_health() -> Result<Json<ha_core::memory::MemoryHealth>, AppError> {
    let backend = get_backend()?;
    Ok(Json(run_blocking(move || backend.health()).await?))
}

/// `POST /api/memory/repair` -- owner-only conservative repair for rebuildable
/// indexes and claim graph links.
pub async fn memory_repair(
    Json(body): Json<RepairMemoryBody>,
) -> Result<Json<ha_core::memory::MemoryRepairReport>, AppError> {
    let backend = get_backend()?;
    let report = run_blocking(move || backend.repair(body.action)).await?;
    ha_core::memory::emit_memory_changed("repair", None, None);
    Ok(Json(report))
}

/// `POST /api/memory/db-snapshot/restore-preview` -- owner-only, read-only
/// preflight for a raw SQLite safety snapshot. It verifies manifest metadata,
/// file existence, size, and sha256 without replacing the active database.
pub async fn memory_db_snapshot_restore_preview(
    Json(body): Json<DbSnapshotRestorePreviewBody>,
) -> Result<Json<ha_core::memory::MemoryDbSnapshotRestorePreview>, AppError> {
    let backend = get_backend()?;
    Ok(Json(
        run_blocking(move || backend.db_snapshot_restore_preview(&body.snapshot_path)).await?,
    ))
}

/// `POST /api/memory/db-snapshot/restore` -- owner-only explicit restore from
/// a preflight-verified raw SQLite safety snapshot. The backend creates a
/// rollback snapshot first and restores through SQLite's backup API.
pub async fn memory_db_snapshot_restore(
    Json(body): Json<DbSnapshotRestorePreviewBody>,
) -> Result<Json<ha_core::memory::MemoryDbSnapshotRestoreReport>, AppError> {
    let backend = get_backend()?;
    let report = run_blocking(move || backend.db_snapshot_restore(&body.snapshot_path)).await?;
    ha_core::memory::emit_memory_changed("db_snapshot_restore", None, None);
    Ok(Json(report))
}

/// `POST /api/memory/episodes` -- owner-only manual episode capture. The new
/// record is not injected into prompts; it is a durable, auditable source for
/// later Retrieval Planner / procedure mining work.
pub async fn add_episode(
    Json(body): Json<AddEpisodeBody>,
) -> Result<Json<ha_core::memory::MemoryEpisodeRecord>, AppError> {
    let record = run_blocking(move || ha_core::memory::add_episode(body.episode)).await?;
    ha_core::memory::emit_memory_changed("episode_add", None, Some(1));
    Ok(Json(record))
}

/// `POST /api/memory/episodes/page` -- list episodes with optional nested
/// scope/query filters.
pub async fn list_episodes_page(
    Json(body): Json<EpisodePageBody>,
) -> Result<Json<ha_core::memory::MemoryEpisodeListPage>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::list_episodes_page(body.query)).await?,
    ))
}

/// `GET /api/memory/episodes/{id}` -- fetch one episode.
pub async fn get_episode(
    Path(id): Path<String>,
) -> Result<Json<Option<ha_core::memory::MemoryEpisodeRecord>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::get_episode(&id)).await?,
    ))
}

/// `PATCH /api/memory/episodes/{id}` -- owner-only correction for a durable
/// episode. Missing patch fields keep their current values.
pub async fn update_episode(
    Path(id): Path<String>,
    Json(body): Json<UpdateEpisodeBody>,
) -> Result<Json<Option<ha_core::memory::MemoryEpisodeRecord>>, AppError> {
    let record = run_blocking(move || ha_core::memory::update_episode(&id, body.patch)).await?;
    if record.is_some() {
        ha_core::memory::emit_memory_changed("episode_update", None, Some(1));
    }
    Ok(Json(record))
}

/// `POST /api/memory/episodes/{id}/archive` -- hide an episode from active
/// owner views without deleting provenance.
pub async fn archive_episode(Path(id): Path<String>) -> Result<Json<bool>, AppError> {
    let changed = run_blocking(move || ha_core::memory::archive_episode(&id)).await?;
    if changed {
        ha_core::memory::emit_memory_changed("episode_archive", None, Some(1));
    }
    Ok(Json(changed))
}

/// `POST /api/memory/episodes/{id}/restore` -- restore an archived episode to
/// active owner views.
pub async fn restore_episode(Path(id): Path<String>) -> Result<Json<bool>, AppError> {
    let changed = run_blocking(move || ha_core::memory::restore_episode(&id)).await?;
    if changed {
        ha_core::memory::emit_memory_changed("episode_restore", None, Some(1));
    }
    Ok(Json(changed))
}

/// `POST /api/memory/procedures` -- owner-only manual procedure capture.
pub async fn add_procedure(
    Json(body): Json<AddProcedureBody>,
) -> Result<Json<ha_core::memory::MemoryProcedureRecord>, AppError> {
    let record = run_blocking(move || ha_core::memory::add_procedure(body.procedure)).await?;
    ha_core::memory::emit_memory_changed("procedure_add", None, Some(1));
    Ok(Json(record))
}

/// `POST /api/memory/episodes/{id}/promote-procedure` -- promote one episode
/// into a soft workflow memory.
pub async fn promote_episode_to_procedure(
    Path(id): Path<String>,
    Json(body): Json<PromoteEpisodeBody>,
) -> Result<Json<ha_core::memory::MemoryProcedureRecord>, AppError> {
    let record = run_blocking(move || {
        ha_core::memory::promote_episode_to_procedure(&id, body.options.unwrap_or_default())
    })
    .await?;
    ha_core::memory::emit_memory_changed("procedure_add", None, Some(1));
    Ok(Json(record))
}

/// `POST /api/memory/procedures/page` -- list soft workflow memories.
pub async fn list_procedures_page(
    Json(body): Json<ProcedurePageBody>,
) -> Result<Json<ha_core::memory::MemoryProcedureListPage>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::list_procedures_page(body.query)).await?,
    ))
}

/// `GET /api/memory/procedures/{id}` -- fetch one procedure.
pub async fn get_procedure(
    Path(id): Path<String>,
) -> Result<Json<Option<ha_core::memory::MemoryProcedureRecord>>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::get_procedure(&id)).await?,
    ))
}

/// `PATCH /api/memory/procedures/{id}` -- owner-only correction for a durable
/// procedure. Missing patch fields keep their current values.
pub async fn update_procedure(
    Path(id): Path<String>,
    Json(body): Json<UpdateProcedureBody>,
) -> Result<Json<Option<ha_core::memory::MemoryProcedureRecord>>, AppError> {
    let record = run_blocking(move || ha_core::memory::update_procedure(&id, body.patch)).await?;
    if record.is_some() {
        ha_core::memory::emit_memory_changed("procedure_update", None, Some(1));
    }
    Ok(Json(record))
}

/// `POST /api/memory/procedures/{id}/archive` -- hide a procedure from active
/// owner views without deleting it.
pub async fn archive_procedure(Path(id): Path<String>) -> Result<Json<bool>, AppError> {
    let changed = run_blocking(move || ha_core::memory::archive_procedure(&id)).await?;
    if changed {
        ha_core::memory::emit_memory_changed("procedure_archive", None, Some(1));
    }
    Ok(Json(changed))
}

/// `POST /api/memory/procedures/{id}/restore` -- restore an archived procedure
/// to active owner views.
pub async fn restore_procedure(Path(id): Path<String>) -> Result<Json<bool>, AppError> {
    let changed = run_blocking(move || ha_core::memory::restore_procedure(&id)).await?;
    if changed {
        ha_core::memory::emit_memory_changed("procedure_restore", None, Some(1));
    }
    Ok(Json(changed))
}

/// `POST /api/memory/experience/history/page` -- owner-only audit trail for
/// episode/procedure edits. History is read-only and never participates in
/// prompt injection or retrieval.
pub async fn experience_history_page(
    Json(body): Json<ExperienceHistoryPageBody>,
) -> Result<Json<ha_core::memory::MemoryExperienceHistoryListPage>, AppError> {
    Ok(Json(
        run_blocking(move || ha_core::memory::list_experience_history_page(body.query)).await?,
    ))
}

/// `GET /api/memory/import-from-ai-prompt` -- get the prompt template shown to the user
/// when importing memories from another AI assistant. Returns a JSON-encoded string
/// (the raw Markdown template), matching the Tauri command's `String` return type.
pub async fn import_from_ai_prompt(
    Query(q): Query<ImportPromptQuery>,
) -> Result<Json<String>, AppError> {
    let locale = q.locale.as_deref().unwrap_or("en");
    let prompt = ha_core::memory::import_prompt::import_from_ai_prompt(locale);
    Ok(Json(prompt.to_string()))
}

// ── Pin / Batch / Re-embed / Global memory.md ─────────────────

#[derive(Debug, Deserialize)]
pub struct TogglePinBody {
    pub pinned: bool,
}

/// `POST /api/memory/{id}/pin` — toggle the pinned status of a memory.
pub async fn toggle_pin(
    Path(id): Path<i64>,
    Json(body): Json<TogglePinBody>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let pinned = body.pinned;
    run_blocking(move || backend.toggle_pin(id, pinned)).await?;
    ha_core::memory::emit_memory_changed(if pinned { "pin" } else { "unpin" }, Some(id), None);
    Ok(Json(json!({ "ok": true, "pinned": body.pinned })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteBatchBody {
    pub ids: Vec<i64>,
}

/// `POST /api/memory/delete-batch` — delete multiple memories at once.
pub async fn delete_batch(Json(body): Json<DeleteBatchBody>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let deleted = run_blocking(move || backend.delete_batch(&body.ids)).await?;
    ha_core::memory::emit_memory_changed("delete_batch", None, Some(deleted));
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Debug, Deserialize)]
pub struct ReembedBody {
    #[serde(default)]
    pub ids: Option<Vec<i64>>,
}

/// `POST /api/memory/reembed` — regenerate embeddings for a subset of (or
/// all) memories. Synchronous; kept for CLI / scripted use. The desktop UI
/// instead uses `POST /api/memory/reembed-start` which spawns a cancellable
/// background `MemoryReembed` job.
pub async fn reembed(Json(body): Json<ReembedBody>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let count = run_blocking(move || match body.ids {
        Some(ids) if !ids.is_empty() => backend.reembed_batch(&ids),
        _ => backend.reembed_all(),
    })
    .await?;
    Ok(Json(json!({ "updated": count })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReembedStartBody {
    #[serde(default)]
    pub mode: ha_core::memory::ReembedMode,
}

/// `POST /api/memory/reembed-start` — spawn a cancellable background reembed
/// job under the currently active memory embedding model. Returns the job
/// snapshot; subsequent progress comes through the standard
/// `local_model_job:*` event stream.
pub async fn reembed_start(
    Json(body): Json<ReembedStartBody>,
) -> Result<Json<ha_core::local_model_jobs::LocalModelJobSnapshot>, AppError> {
    let model_id = ha_core::config::cached_config()
        .memory_embedding
        .model_config_id
        .clone()
        .ok_or_else(|| {
            AppError::bad_request("No memory embedding model is currently active".to_string())
        })?;
    let snapshot =
        run_blocking(move || ha_core::memory::start_memory_reembed_job(&model_id, body.mode, None))
            .await?;
    Ok(Json(snapshot))
}

/// `GET /api/memory/global-md` — read the user's global `memory.md` file.
pub async fn get_global_memory_md() -> Result<Json<Value>, AppError> {
    if ha_core::config::cached_config()
        .memory
        .core_repository_enabled()
    {
        let content = ha_core::blocking::run_blocking(move || {
            ha_core::memory::core_repository::load_index(
                &ha_core::memory::core_repository::CoreMemoryScope::Global,
            )
            .map(|index| index.content)
        })
        .await?;
        return Ok(Json(json!({ "content": content })));
    }
    let path = ha_core::paths::root_dir()?.join("memory.md");
    let content = run_blocking(move || {
        if path.exists() {
            std::fs::read_to_string(&path).map(Some)
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "content": content })))
}

#[derive(Debug, Deserialize)]
pub struct MemoryMdBody {
    pub content: String,
}

/// `PUT /api/memory/global-md` — write the user's global `memory.md` file.
pub async fn save_global_memory_md(
    Json(body): Json<MemoryMdBody>,
) -> Result<Json<Value>, AppError> {
    if ha_core::config::cached_config()
        .memory
        .core_repository_enabled()
    {
        ha_core::blocking::run_blocking(move || {
            ha_core::memory::core_repository::save_index_owner(
                &ha_core::memory::core_repository::CoreMemoryScope::Global,
                &body.content,
            )
            .map(|_| ())
        })
        .await?;
        return Ok(Json(json!({ "saved": true })));
    }
    let path = ha_core::paths::root_dir()?.join("memory.md");
    run_blocking(move || ha_core::platform::write_atomic(&path, body.content.as_bytes()))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "saved": true })))
}

// ── Export / Import / Find similar ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMemoryBody {
    #[serde(default)]
    pub scope: Option<ha_core::memory::MemoryScope>,
}

/// `POST /api/memory/export` — export memories as a Markdown string.
/// Mirrors the Tauri `memory_export` command.
pub async fn export_memory(Json(body): Json<ExportMemoryBody>) -> Result<Json<String>, AppError> {
    let backend = get_backend()?;
    let md = run_blocking(move || backend.export_markdown(body.scope.as_ref())).await?;
    Ok(Json(md))
}

/// `POST /api/memory/backup/export` — export a JSON memory backup bundle.
/// This is intentionally export-only; restore goes through a future preview
/// plan rather than overwriting local memory directly.
pub async fn export_memory_backup() -> Result<Json<ha_core::memory::MemoryBackupBundle>, AppError> {
    let backend = get_backend()?;
    Ok(Json(
        run_blocking(move || ha_core::memory::export_backup_bundle(backend.as_ref())).await?,
    ))
}

/// `POST /api/memory/backup/export-archive` — export a ZIP memory backup
/// package containing the JSON bundle plus large attachment sidecars.
pub async fn export_memory_backup_archive() -> Result<Response, AppError> {
    let backend = get_backend()?;
    let archive =
        run_blocking(move || ha_core::memory::export_backup_archive(backend.as_ref())).await?;
    let mut response = (axum::http::StatusCode::OK, archive).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(ha_core::memory::MEMORY_BACKUP_ARCHIVE_MIME),
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"hope-agent-memory-backup.zip\""),
    );
    Ok(response)
}

#[derive(Debug, Deserialize)]
pub struct ExportEncryptedMemoryBackupBody {
    pub passphrase: String,
}

/// `POST /api/memory/backup/export-encrypted` — export a password-encrypted
/// JSON memory backup envelope. The plaintext is the normal backup bundle.
pub async fn export_encrypted_memory_backup(
    Json(body): Json<ExportEncryptedMemoryBackupBody>,
) -> Result<Json<ha_core::memory::MemoryEncryptedBackupBundle>, AppError> {
    let backend = get_backend()?;
    Ok(Json(
        run_blocking(move || {
            ha_core::memory::export_encrypted_backup_bundle(backend.as_ref(), &body.passphrase)
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct PreviewMemoryBackupBody {
    pub content: String,
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreLegacyMemoryBackupBody {
    pub content: String,
    #[serde(default)]
    pub options: Option<ha_core::memory::MemoryBackupRestoreOptions>,
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreStructuredMemoryBackupBody {
    pub content: String,
    #[serde(default)]
    pub options: Option<ha_core::memory::MemoryBackupStructuredRestoreOptions>,
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreLegacyMemoryBackupArchiveQuery {
    #[serde(default)]
    pub dedup: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreStructuredMemoryBackupArchiveQuery {
    #[serde(default)]
    pub restore_claims: Option<bool>,
    #[serde(default)]
    pub restore_profile_snapshots: Option<bool>,
    #[serde(default)]
    pub restore_episodes: Option<bool>,
    #[serde(default)]
    pub restore_procedures: Option<bool>,
    #[serde(default)]
    pub restore_experience_history: Option<bool>,
    #[serde(default)]
    pub allow_profile_scope_conflicts: Option<bool>,
}

/// `POST /api/memory/backup/preview` — validate a backup JSON bundle and
/// return a read-only import plan. No memory rows are written here.
pub async fn preview_memory_backup(
    Json(body): Json<PreviewMemoryBackupBody>,
) -> Result<Json<ha_core::memory::MemoryBackupImportPreview>, AppError> {
    let backend = get_backend()?;
    Ok(Json(
        run_blocking(move || {
            ha_core::memory::preview_backup_bundle_with_passphrase(
                backend.as_ref(),
                &body.content,
                body.passphrase.as_deref(),
            )
        })
        .await?,
    ))
}

/// `POST /api/memory/backup/preview-archive` — validate a ZIP backup package
/// and return a read-only import plan, including checksum-verified sidecars.
pub async fn preview_memory_backup_archive(
    body: Bytes,
) -> Result<Json<ha_core::memory::MemoryBackupImportPreview>, AppError> {
    let backend = get_backend()?;
    Ok(Json(
        run_blocking(move || ha_core::memory::preview_backup_archive(backend.as_ref(), &body))
            .await?,
    ))
}

/// `POST /api/memory/backup/restore-legacy` — apply only the safe legacy-memory
/// subset of a backup bundle. Claims/profile snapshots remain preview-only.
pub async fn restore_legacy_memory_backup(
    Json(body): Json<RestoreLegacyMemoryBackupBody>,
) -> Result<Json<ha_core::memory::MemoryBackupRestoreResult>, AppError> {
    let backend = get_backend()?;
    let result = run_blocking(move || {
        ha_core::memory::restore_backup_legacy_memories_with_passphrase(
            backend.as_ref(),
            &body.content,
            body.options.unwrap_or_default(),
            body.passphrase.as_deref(),
        )
    })
    .await?;
    if result.import_result.created > 0 {
        ha_core::memory::emit_memory_changed(
            "backup_restore_legacy",
            None,
            Some(result.import_result.created),
        );
    }
    Ok(Json(result))
}

/// `POST /api/memory/backup/restore-legacy-archive` — restore legacy memories
/// from a ZIP backup package, including verified attachment sidecars.
pub async fn restore_legacy_memory_backup_archive(
    Query(query): Query<RestoreLegacyMemoryBackupArchiveQuery>,
    body: Bytes,
) -> Result<Json<ha_core::memory::MemoryBackupRestoreResult>, AppError> {
    let backend = get_backend()?;
    let result = run_blocking(move || {
        ha_core::memory::restore_backup_legacy_memories_from_archive(
            backend.as_ref(),
            &body,
            ha_core::memory::MemoryBackupRestoreOptions {
                dedup: query.dedup.unwrap_or(true),
            },
        )
    })
    .await?;
    if result.import_result.created > 0 {
        ha_core::memory::emit_memory_changed(
            "backup_restore_legacy",
            None,
            Some(result.import_result.created),
        );
    }
    Ok(Json(result))
}

/// `POST /api/memory/backup/restore-structured` — apply the structured subset
/// of a backup bundle. Additive only: missing claims/profile snapshots are
/// restored, local exact matches are skipped, and profile scope conflicts are
/// skipped unless explicitly allowed.
pub async fn restore_structured_memory_backup(
    Json(body): Json<RestoreStructuredMemoryBackupBody>,
) -> Result<Json<ha_core::memory::MemoryBackupStructuredRestoreResult>, AppError> {
    let backend = get_backend()?;
    let result = run_blocking(move || {
        ha_core::memory::restore_backup_structured_memory_with_passphrase(
            backend.as_ref(),
            &body.content,
            body.options.unwrap_or_default(),
            body.passphrase.as_deref(),
        )
    })
    .await?;
    if result.restored_claims > 0 {
        ha_core::memory::emit_claim_changed(
            "backup_restore_structured",
            None,
            Some(result.restored_claims),
        );
    }
    if result.restored_profile_snapshots > 0 {
        ha_core::memory::emit_memory_changed(
            "backup_restore_profile",
            None,
            Some(result.restored_profile_snapshots),
        );
    }
    Ok(Json(result))
}

/// `POST /api/memory/backup/restore-structured-archive` — restore structured
/// claims/profile snapshots from a ZIP backup package.
pub async fn restore_structured_memory_backup_archive(
    Query(query): Query<RestoreStructuredMemoryBackupArchiveQuery>,
    body: Bytes,
) -> Result<Json<ha_core::memory::MemoryBackupStructuredRestoreResult>, AppError> {
    let backend = get_backend()?;
    let result = run_blocking(move || {
        ha_core::memory::restore_backup_structured_memory_from_archive(
            backend.as_ref(),
            &body,
            ha_core::memory::MemoryBackupStructuredRestoreOptions {
                restore_claims: query.restore_claims.unwrap_or(true),
                restore_profile_snapshots: query.restore_profile_snapshots.unwrap_or(true),
                restore_episodes: query.restore_episodes.unwrap_or(true),
                restore_procedures: query.restore_procedures.unwrap_or(true),
                restore_experience_history: query.restore_experience_history.unwrap_or(true),
                allow_profile_scope_conflicts: query.allow_profile_scope_conflicts.unwrap_or(false),
            },
        )
    })
    .await?;
    if result.restored_claims > 0 {
        ha_core::memory::emit_claim_changed(
            "backup_restore_structured",
            None,
            Some(result.restored_claims),
        );
    }
    if result.restored_profile_snapshots > 0 {
        ha_core::memory::emit_memory_changed(
            "backup_restore_profile",
            None,
            Some(result.restored_profile_snapshots),
        );
    }
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct ImportMemoryBody {
    pub content: String,
    pub format: String,
    #[serde(default)]
    pub dedup: bool,
}

/// `POST /api/memory/import` — import memories from JSON or Markdown.
/// Mirrors the Tauri `memory_import` command. Unsupported-format errors are
/// surfaced as 400 so operator tooling can distinguish bad input from backend
/// failures.
pub async fn import_memory(
    Json(body): Json<ImportMemoryBody>,
) -> Result<Json<ha_core::memory::ImportResult>, AppError> {
    let content = body.content;
    let format = body.format;
    let dedup = body.dedup;
    let entries = run_blocking(move || ha_core::memory::parse_import(&content, &format))
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let backend = get_backend()?;
    let result = run_blocking(move || backend.import_entries(entries, dedup)).await?;
    if result.created > 0 {
        ha_core::memory::emit_memory_changed("import", None, Some(result.created));
    }
    Ok(Json(result))
}

/// `POST /api/memory/import/preview` — parse import input without writing.
pub async fn preview_import_memory(
    Json(body): Json<ImportMemoryBody>,
) -> Result<Json<ha_core::memory::MemoryImportPreview>, AppError> {
    let backend = get_backend()?;
    Ok(Json(
        run_blocking(move || {
            ha_core::memory::preview_import_with_backend(
                backend.as_ref(),
                &body.content,
                &body.format,
                None,
                body.dedup,
            )
        })
        .await,
    ))
}

#[derive(Debug, Deserialize)]
pub struct FindSimilarBody {
    pub content: String,
    #[serde(default)]
    pub threshold: Option<f32>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `POST /api/memory/find-similar` — locate memories close to a seed string.
/// Mirrors the Tauri `memory_find_similar` command.
pub async fn find_similar(
    Json(body): Json<FindSimilarBody>,
) -> Result<Json<Vec<ha_core::memory::MemoryEntry>>, AppError> {
    let backend = get_backend()?;
    let dedup_cfg = ha_core::memory::load_dedup_config();
    let threshold = body.threshold.unwrap_or(dedup_cfg.threshold_merge);
    let limit = body.limit.unwrap_or(5);
    let results =
        run_blocking(move || backend.find_similar(&body.content, None, None, threshold, limit))
            .await?;
    Ok(Json(results))
}

/// `GET /api/memory/local-embedding-models` — list the fastembed models
/// that have been downloaded into the local cache (with their sizes).
/// Used by Settings → Memory → Embedding provider dropdown. Mirror of the
/// Tauri `list_local_embedding_models` command.
pub async fn list_local_embedding_models(
) -> Result<Json<Vec<ha_core::memory::LocalEmbeddingModel>>, AppError> {
    Ok(Json(
        run_blocking(ha_core::memory::list_local_models_with_status).await,
    ))
}
