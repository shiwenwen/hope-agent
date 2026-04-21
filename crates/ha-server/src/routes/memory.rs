use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsQuery {
    pub scope: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportPromptQuery {
    pub locale: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────

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

// ── Handlers ────────────────────────────────────────────────────

/// `POST /api/memory` -- add a new memory entry.
pub async fn add_memory(Json(body): Json<AddMemoryBody>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let id = backend.add(body.entry)?;
    Ok(Json(json!({ "id": id })))
}

/// `PUT /api/memory/{id}` -- update an existing memory entry.
pub async fn update_memory(
    Path(id): Path<i64>,
    Json(body): Json<UpdateMemoryBody>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    backend.update(id, &body.content, &body.tags)?;
    Ok(Json(json!({ "updated": true })))
}

/// `DELETE /api/memory/{id}` -- delete a memory entry.
pub async fn delete_memory(Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    backend.delete(id)?;
    Ok(Json(json!({ "deleted": true })))
}

/// `GET /api/memory/{id}` -- get a single memory entry.
pub async fn get_memory(Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let entry = backend
        .get(id)?
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
    let entries = backend.list(
        scope.as_ref(),
        types.as_deref(),
        q.limit.unwrap_or(50),
        q.offset.unwrap_or(0),
    )?;
    Ok(Json(entries))
}

/// `POST /api/memory/search` -- semantic search over memories.
pub async fn search_memories(
    Json(body): Json<SearchMemoryBody>,
) -> Result<Json<Vec<ha_core::memory::MemoryEntry>>, AppError> {
    let backend = get_backend()?;
    let results = backend.search(&body.query)?;
    Ok(Json(results))
}

/// `GET /api/memory/count` -- get total memory count.
pub async fn memory_count(Query(q): Query<CountQuery>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let scope = parse_scope(&q.scope, &q.agent_id);
    let count = backend.count(scope.as_ref())?;
    Ok(Json(json!({ "count": count })))
}

/// `GET /api/memory/stats` -- get memory statistics.
pub async fn memory_stats(
    Query(q): Query<StatsQuery>,
) -> Result<Json<ha_core::memory::MemoryStats>, AppError> {
    let backend = get_backend()?;
    let scope = parse_scope(&q.scope, &q.agent_id);
    let stats = backend.stats(scope.as_ref())?;
    Ok(Json(stats))
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
    backend.toggle_pin(id, body.pinned)?;
    Ok(Json(json!({ "ok": true, "pinned": body.pinned })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteBatchBody {
    pub ids: Vec<i64>,
}

/// `POST /api/memory/delete-batch` — delete multiple memories at once.
pub async fn delete_batch(Json(body): Json<DeleteBatchBody>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let deleted = backend.delete_batch(&body.ids)?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Debug, Deserialize)]
pub struct ReembedBody {
    #[serde(default)]
    pub ids: Option<Vec<i64>>,
}

/// `POST /api/memory/reembed` — regenerate embeddings for a subset of (or
/// all) memories.
pub async fn reembed(Json(body): Json<ReembedBody>) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let count = match body.ids {
        Some(ids) if !ids.is_empty() => backend.reembed_batch(&ids)?,
        _ => backend.reembed_all()?,
    };
    Ok(Json(json!({ "updated": count })))
}

/// `GET /api/memory/global-md` — read the user's global `memory.md` file.
pub async fn get_global_memory_md() -> Result<Json<Value>, AppError> {
    let path = ha_core::paths::root_dir()?.join("memory.md");
    let content = if path.exists() {
        Some(std::fs::read_to_string(&path).map_err(|e| AppError::internal(e.to_string()))?)
    } else {
        None
    };
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
    let path = ha_core::paths::root_dir()?.join("memory.md");
    std::fs::write(&path, body.content).map_err(|e| AppError::internal(e.to_string()))?;
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
    let md = backend.export_markdown(body.scope.as_ref())?;
    Ok(Json(md))
}

#[derive(Debug, Deserialize)]
pub struct ImportMemoryBody {
    pub content: String,
    pub format: String,
    #[serde(default)]
    pub dedup: bool,
}

/// `POST /api/memory/import` — import memories from JSON or Markdown.
/// Mirrors the Tauri `memory_import` command.
pub async fn import_memory(
    Json(body): Json<ImportMemoryBody>,
) -> Result<Json<ha_core::memory::ImportResult>, AppError> {
    let entries = match body.format.as_str() {
        "json" => ha_core::memory::parse_import_json(&body.content)?,
        "markdown" | "md" => ha_core::memory::parse_import_markdown(&body.content)?,
        other => {
            return Err(AppError::bad_request(format!(
                "Unsupported format: {}",
                other
            )))
        }
    };
    let backend = get_backend()?;
    let result = backend.import_entries(entries, body.dedup)?;
    Ok(Json(result))
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
    let results = backend.find_similar(&body.content, None, None, threshold, limit)?;
    Ok(Json(results))
}

/// `GET /api/memory/local-embedding-models` — list the fastembed models
/// that have been downloaded into the local cache (with their sizes).
/// Used by Settings → Memory → Embedding provider dropdown. Mirror of the
/// Tauri `list_local_embedding_models` command.
pub async fn list_local_embedding_models(
) -> Result<Json<Vec<ha_core::memory::LocalEmbeddingModel>>, AppError> {
    Ok(Json(ha_core::memory::list_local_models_with_status()))
}
