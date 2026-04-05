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
pub struct ListSessionsQuery {
    pub agent_id: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
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
    let (sessions, total) =
        ctx.session_db
            .list_sessions_paged(q.agent_id.as_deref(), q.limit, q.offset)?;
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
