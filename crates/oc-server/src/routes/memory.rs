use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Query / Body Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
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
pub struct CountQuery {
    pub scope: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub scope: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportPromptQuery {
    pub locale: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────

fn get_backend() -> Result<&'static std::sync::Arc<dyn oc_core::memory::MemoryBackend>, AppError> {
    oc_core::get_memory_backend()
        .ok_or_else(|| AppError::internal("Memory backend not initialized"))
}

/// Parse scope from query params: explicit `scope` JSON or shorthand `agent_id`.
fn parse_scope(scope: &Option<String>, agent_id: &Option<String>) -> Option<oc_core::memory::MemoryScope> {
    if let Some(s) = scope {
        serde_json::from_str(s).ok()
    } else {
        agent_id.as_ref().map(|id| oc_core::memory::MemoryScope::Agent { id: id.clone() })
    }
}

/// Parse memory types from comma-separated string.
fn parse_types(types: &Option<String>) -> Option<Vec<oc_core::memory::MemoryType>> {
    types.as_ref().map(|t| {
        t.split(',')
            .map(|s| oc_core::memory::MemoryType::from_str(s.trim()))
            .collect()
    })
}

// ── Handlers ────────────────────────────────────────────────────

/// `POST /api/memory` -- add a new memory entry.
pub async fn add_memory(
    Json(entry): Json<oc_core::memory::NewMemory>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let id = backend.add(entry)?;
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
pub async fn delete_memory(
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    backend.delete(id)?;
    Ok(Json(json!({ "deleted": true })))
}

/// `GET /api/memory/{id}` -- get a single memory entry.
pub async fn get_memory(
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let entry = backend
        .get(id)?
        .ok_or_else(|| AppError::not_found(format!("memory not found: {}", id)))?;
    Ok(Json(serde_json::to_value(entry)?))
}

/// `GET /api/memory` -- list memories with optional filtering.
pub async fn list_memories(
    Query(q): Query<ListMemoryQuery>,
) -> Result<Json<Vec<oc_core::memory::MemoryEntry>>, AppError> {
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
    Json(query): Json<oc_core::memory::MemorySearchQuery>,
) -> Result<Json<Vec<oc_core::memory::MemoryEntry>>, AppError> {
    let backend = get_backend()?;
    let results = backend.search(&query)?;
    Ok(Json(results))
}

/// `GET /api/memory/count` -- get total memory count.
pub async fn memory_count(
    Query(q): Query<CountQuery>,
) -> Result<Json<Value>, AppError> {
    let backend = get_backend()?;
    let scope = parse_scope(&q.scope, &q.agent_id);
    let count = backend.count(scope.as_ref())?;
    Ok(Json(json!({ "count": count })))
}

/// `GET /api/memory/stats` -- get memory statistics.
pub async fn memory_stats(
    Query(q): Query<StatsQuery>,
) -> Result<Json<oc_core::memory::MemoryStats>, AppError> {
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
    let prompt = oc_core::memory::import_prompt::import_from_ai_prompt(locale);
    Ok(Json(prompt.to_string()))
}
