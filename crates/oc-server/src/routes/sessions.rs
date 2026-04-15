use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::AppError;
use crate::AppContext;

// ── Query / Body Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsQuery {
    pub agent_id: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSessionsQuery {
    pub query: String,
    pub agent_id: Option<String>,
    /// Comma-separated list of session types (`regular,cron,subagent,channel`).
    pub types: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesAroundQuery {
    pub target_message_id: i64,
    pub before: Option<u32>,
    pub after: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchInSessionQuery {
    pub query: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionBody {
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RenameSessionBody {
    pub title: String,
}

// ── Response wrapper for paginated lists ────────────────────────

#[derive(Debug, Serialize)]
pub struct PaginatedSessions {
    pub sessions: Vec<oc_core::session::SessionMeta>,
    pub total: u32,
}

// ── Handlers ────────────────────────────────────────────────────

/// `POST /api/sessions` — create a new session.
pub async fn create_session(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateSessionBody>,
) -> Result<Json<oc_core::session::SessionMeta>, AppError> {
    let agent_id = body.agent_id.as_deref().unwrap_or("default");
    let meta = ctx.session_db.create_session(agent_id)?;
    Ok(Json(meta))
}

/// `GET /api/sessions` — list sessions with optional filtering and pagination.
pub async fn list_sessions(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<PaginatedSessions>, AppError> {
    let (mut sessions, total) =
        ctx.session_db
            .list_sessions_paged(q.agent_id.as_deref(), q.limit, q.offset)?;

    oc_core::session::enrich_pending_interactions(&mut sessions, &ctx.session_db).await?;

    Ok(Json(PaginatedSessions { sessions, total }))
}

/// `GET /api/sessions/:id` — get a single session.
pub async fn get_session(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let meta = ctx
        .session_db
        .get_session(&id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {}", id))?;
    Ok(Json(serde_json::to_value(meta)?))
}

/// `DELETE /api/sessions/:id` — delete a session and all its messages.
pub async fn delete_session(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.delete_session(&id)?;
    Ok(Json(json!({ "deleted": true })))
}

/// `PATCH /api/sessions/:id` — rename a session.
pub async fn rename_session(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<RenameSessionBody>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.update_session_title(&id, &body.title)?;
    Ok(Json(json!({ "updated": true })))
}

/// `GET /api/sessions/search` — full-text search message history.
pub async fn search_sessions(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<SearchSessionsQuery>,
) -> Result<Json<Vec<oc_core::session::SessionSearchResult>>, AppError> {
    let limit = q.limit.unwrap_or(80) as usize;

    let parsed_types: Option<Vec<oc_core::session::SessionTypeFilter>> = q.types.as_ref().map(|s| {
        s.split(',')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .filter_map(oc_core::session::SessionTypeFilter::parse)
            .collect()
    });
    let type_slice = parsed_types.as_deref();

    let results = ctx
        .session_db
        .search_messages(&q.query, q.agent_id.as_deref(), None, type_slice, limit)?;
    Ok(Json(results))
}

/// `GET /api/sessions/:id/messages/search?query=...&limit=...` — FTS5
/// full-text search scoped to a single session (used by the in-chat
/// "find in page" search bar).
pub async fn search_session_messages(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<SearchInSessionQuery>,
) -> Result<Json<Vec<oc_core::session::SessionSearchResult>>, AppError> {
    let limit = q.limit.unwrap_or(200) as usize;
    let results = ctx
        .session_db
        .search_messages(&q.query, None, Some(&id), None, limit)?;
    Ok(Json(results))
}

/// `GET /api/sessions/:id/messages/around?targetMessageId=N&before=40&after=20`
/// — load a window of messages centred on a target id.
pub async fn get_session_messages_around(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<MessagesAroundQuery>,
) -> Result<Json<Value>, AppError> {
    let before = q.before.unwrap_or(40);
    let after = q.after.unwrap_or(20);
    let (messages, total) =
        ctx.session_db
            .load_session_messages_around(&id, q.target_message_id, before, after)?;
    Ok(Json(json!({ "messages": messages, "total": total })))
}

/// `GET /api/sessions/:id/messages?limit=N` — load latest messages for a session.
/// Returns `{ messages: [...], total: N }`. Default limit is 50.
pub async fn get_session_messages(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let (messages, total) = ctx.session_db.load_session_messages_latest(&id, limit)?;
    Ok(Json(json!({ "messages": messages, "total": total })))
}

// ── Read-state / Compact ───────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadBatchBody {
    pub session_ids: Vec<String>,
}

/// `POST /api/sessions/:id/read` — mark a single session as read.
pub async fn mark_session_read(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.mark_session_read(&id)?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/sessions/read-batch` — mark a list of sessions as read.
pub async fn mark_session_read_batch(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<ReadBatchBody>,
) -> Result<Json<Value>, AppError> {
    let count = body.session_ids.len();
    ctx.session_db.mark_session_read_batch(&body.session_ids)?;
    Ok(Json(json!({ "ok": true, "count": count })))
}

/// `POST /api/sessions/read-all` — mark every session as read.
pub async fn mark_all_sessions_read(
    State(ctx): State<Arc<AppContext>>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.mark_all_sessions_read()?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/sessions/:id/compact` — stub: manual context compaction.
///
/// In the Tauri desktop shell this runs against the live in-memory agent.
/// The HTTP server is stateless (each `POST /api/chat` spins up a fresh
/// agent), so there is no persistent conversation to compact here. Returns
/// a zero-result so the settings UI can still display a value. The response
/// uses camelCase to match `oc_core::context_compact::CompactResult`'s
/// `#[serde(rename_all = "camelCase")]`.
pub async fn compact_context_now(
    State(_ctx): State<Arc<AppContext>>,
    Path(_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "tierApplied": 0,
        "tokensBefore": 0,
        "tokensAfter": 0,
        "messagesAffected": 0,
        "description": "not_supported_in_server_mode",
        "details": null,
    })))
}
