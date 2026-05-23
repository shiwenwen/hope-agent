use axum::extract::{Path, Query, Request, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::error::AppError;
use crate::routes::file_serve::{
    apply_inline_media_headers, resolve_mime_for_path, HeaderOpts, MimeOpts,
};
use crate::AppContext;

type SessionMessage = ha_core::session::SessionMessage;

// ── Query / Body Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsQuery {
    pub agent_id: Option<String>,
    /// Filter to sessions inside a specific project.
    pub project_id: Option<String>,
    /// When `true`, only return sessions not assigned to any project. Overrides `project_id`.
    pub unassigned: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Currently-open session id; allowed to appear in results even if it is
    /// incognito. All other incognito sessions are filtered out.
    pub active_session_id: Option<String>,
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
pub struct MessagesBeforeQuery {
    pub before_id: i64,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesAfterQuery {
    pub after_id: i64,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchInSessionQuery {
    pub query: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileByPathQuery {
    pub path: String,
    pub download: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionBody {
    pub agent_id: Option<String>,
    /// When set, attaches the new session to this project.
    pub project_id: Option<String>,
    pub incognito: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RenameSessionBody {
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPinnedBody {
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwarenessOverrideBody {
    /// JSON string. `None` or empty clears the override.
    pub json: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIncognitoBody {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWorkingDirBody {
    /// Absolute path on the server machine. `None` or empty string clears
    /// the selection. The core layer canonicalizes + validates the path.
    pub working_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAgentBody {
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModelBody {
    pub provider_id: String,
    pub model_id: String,
}

// ── Response wrapper for paginated lists ────────────────────────

#[derive(Debug, Serialize)]
pub struct PaginatedSessions {
    pub sessions: Vec<ha_core::session::SessionMeta>,
    pub total: u32,
}

fn rewrite_messages_for_http(
    mut messages: Vec<SessionMessage>,
    api_key: Option<&str>,
) -> Vec<SessionMessage> {
    for msg in &mut messages {
        rewrite_tool_media_meta_for_http(msg, api_key);
    }
    messages
}

fn rewrite_tool_media_meta_for_http(msg: &mut SessionMessage, api_key: Option<&str>) {
    let Some(raw) = msg.attachments_meta.as_deref() else {
        return;
    };
    if !raw.contains(ha_core::session::ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS) {
        return;
    }

    let Ok(mut meta) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let Some(items) = meta
        .get(ha_core::session::ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS)
        .cloned()
    else {
        return;
    };

    let event = json!({ "media_items": items });
    let rewritten = ha_core::agent::rewrite_event_for_http(&event.to_string(), api_key);
    let Ok(rewritten_event) = serde_json::from_str::<Value>(&rewritten) else {
        return;
    };
    let Some(rewritten_items) = rewritten_event.get("media_items").cloned() else {
        return;
    };
    meta[ha_core::session::ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS] = rewritten_items;
    msg.attachments_meta = Some(meta.to_string());
}

fn collect_authorized_session_file_paths(messages: &[SessionMessage]) -> HashSet<String> {
    let mut paths = HashSet::new();
    for msg in messages {
        if let Some(raw) = msg.tool_metadata.as_deref() {
            collect_paths_from_tool_metadata(raw, &mut paths);
        }
        if let Some(raw) = msg.attachments_meta.as_deref() {
            collect_paths_from_attachments_meta(raw, &mut paths);
        }
        let tool_name = msg.tool_name.as_deref().unwrap_or_default();
        if let Some(result) = msg.tool_result.as_deref() {
            collect_legacy_saved_to_paths(tool_name, &msg.session_id, result, &mut paths);
            if tool_name == "apply_patch" {
                collect_apply_patch_result_paths(result, &mut paths);
            }
        }
        if let (Some(name), Some(args), Some(result)) = (
            msg.tool_name.as_deref(),
            msg.tool_arguments.as_deref(),
            msg.tool_result.as_deref(),
        ) {
            collect_legacy_arg_paths(name, args, result, &mut paths);
        }
    }
    paths
}

fn collect_paths_from_tool_metadata(raw: &str, paths: &mut HashSet<String>) {
    let Ok(meta) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    match meta.get("kind").and_then(Value::as_str) {
        Some("file_change") => collect_file_change_path(&meta, paths),
        Some("file_changes") => {
            if let Some(changes) = meta.get("changes").and_then(Value::as_array) {
                for change in changes {
                    collect_file_change_path(change, paths);
                }
            }
        }
        _ => {}
    }
}

fn collect_file_change_path(change: &Value, paths: &mut HashSet<String>) {
    if change.get("action").and_then(Value::as_str) == Some("delete") {
        return;
    }
    add_nonempty_path(paths, change.get("path").and_then(Value::as_str));
}

fn collect_paths_from_attachments_meta(raw: &str, paths: &mut HashSet<String>) {
    if !raw.contains(ha_core::session::ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS) {
        return;
    }
    let Ok(meta) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let Some(items) = meta
        .get(ha_core::session::ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS)
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in items {
        add_nonempty_path(paths, item.get("localPath").and_then(Value::as_str));
    }
}

fn collect_legacy_saved_to_paths(
    tool_name: &str,
    session_id: &str,
    result: &str,
    paths: &mut HashSet<String>,
) {
    if !matches!(tool_name, "send_attachment" | "image_generate") {
        return;
    }
    for line in result.lines() {
        let trimmed = line.trim();
        if let Some(path) = trimmed.strip_prefix("Saved to: ") {
            if legacy_saved_to_path_allowed(tool_name, session_id, path) {
                add_nonempty_path(paths, Some(path));
            }
        }
    }
}

fn legacy_saved_to_path_allowed(tool_name: &str, session_id: &str, raw_path: &str) -> bool {
    let path = PathBuf::from(raw_path.trim());
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }

    if let Ok(attachments_dir) = ha_core::paths::attachments_dir(session_id) {
        if path.starts_with(attachments_dir) {
            return true;
        }
    }

    if tool_name == "image_generate" {
        if let Ok(generated_images_dir) = ha_core::paths::generated_images_dir() {
            if path.starts_with(generated_images_dir) {
                return true;
            }
        }
    }

    false
}

fn collect_apply_patch_result_paths(result: &str, paths: &mut HashSet<String>) {
    if !result.starts_with("Patch applied") {
        return;
    }
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Deleted:") {
            continue;
        }
        let Some(rest) = trimmed
            .strip_prefix("Added: ")
            .or_else(|| trimmed.strip_prefix("Modified: "))
            .or_else(|| trimmed.strip_prefix("Renamed: "))
        else {
            continue;
        };
        for entry in rest.split(", ") {
            let path = entry
                .split_once(" -> ")
                .map(|(_, to)| to)
                .unwrap_or(entry)
                .trim();
            add_nonempty_path(paths, Some(path));
        }
    }
}

fn collect_legacy_arg_paths(name: &str, args: &str, result: &str, paths: &mut HashSet<String>) {
    let is_file_write =
        matches!(name, "write" | "write_file") && result.starts_with("Successfully wrote");
    let is_file_edit =
        matches!(name, "edit" | "patch_file") && result.starts_with("Successfully edited");
    if !is_file_write && !is_file_edit {
        return;
    }
    let Ok(args) = serde_json::from_str::<Value>(args) else {
        return;
    };
    add_nonempty_path(
        paths,
        args.get("path")
            .or_else(|| args.get("file_path"))
            .and_then(Value::as_str),
    );
}

fn add_nonempty_path(paths: &mut HashSet<String>, value: Option<&str>) {
    let Some(path) = value.map(str::trim).filter(|p| !p.is_empty()) else {
        return;
    };
    paths.insert(path.to_string());
}

async fn authorized_canonical_file_path(
    requested: &str,
    messages: &[SessionMessage],
) -> Result<PathBuf, AppError> {
    let requested_path = PathBuf::from(requested);
    if !requested_path.is_absolute() {
        return Err(AppError::bad_request("path must be absolute"));
    }

    let paths = collect_authorized_session_file_paths(messages);
    for raw in paths {
        if raw.trim() == requested {
            return tokio::fs::canonicalize(&requested_path)
                .await
                .map_err(|_| AppError::not_found("file not found"));
        }
    }
    Err(AppError::forbidden("file not referenced by session"))
}

fn parse_download_flag(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim) else {
        return false;
    };
    value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
}

fn content_disposition_for_file(path: &FsPath, mime: &str, force_download: bool) -> String {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let kind = if force_download {
        "attachment"
    } else if mime.starts_with("image/")
        || mime.starts_with("video/")
        || mime.starts_with("audio/")
        || mime == "application/pdf"
    {
        "inline"
    } else {
        "attachment"
    };
    let quoted = filename.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{}; filename=\"{}\"", kind, quoted)
}

// ── Handlers ────────────────────────────────────────────────────

/// `POST /api/sessions` — create a new session.
pub async fn create_session(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateSessionBody>,
) -> Result<Json<ha_core::session::SessionMeta>, AppError> {
    // Caller-supplied agent_id wins. Otherwise follow the same resolver chain
    // as the desktop command: project default → global default → "ha-main".
    let explicit_agent_id = body
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let project = match (explicit_agent_id, body.project_id.as_deref()) {
        (None, Some(project_id)) => ctx.project_db.get(project_id)?,
        _ => None,
    };
    let agent_id = explicit_agent_id.map(ToOwned::to_owned).unwrap_or_else(|| {
        ha_core::agent::resolver::resolve_default_agent_id(project.as_ref(), None)
    });
    let meta = ctx.session_db.create_session_with_project(
        &agent_id,
        body.project_id.as_deref(),
        body.incognito,
    )?;
    Ok(Json(meta))
}

/// `GET /api/sessions` — list sessions with optional filtering and pagination.
pub async fn list_sessions(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<PaginatedSessions>, AppError> {
    // Precedence: explicit `unassigned=true` wins; then `project_id`; else All.
    let project_filter = if q.unassigned.unwrap_or(false) {
        ha_core::session::ProjectFilter::Unassigned
    } else if let Some(ref pid) = q.project_id {
        ha_core::session::ProjectFilter::InProject(pid.as_str())
    } else {
        ha_core::session::ProjectFilter::All
    };

    let (mut sessions, total) = ctx.session_db.list_sessions_paged_for_sidebar(
        q.agent_id.as_deref(),
        project_filter,
        q.limit,
        q.offset,
        q.active_session_id.as_deref(),
    )?;

    ha_core::session::enrich_pending_interactions(&mut sessions, &ctx.session_db).await?;

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

/// `PATCH /api/sessions/:id/pinned` — pin/unpin a session in sidebar lists.
pub async fn set_session_pinned(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<SessionPinnedBody>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.set_session_pinned(&id, body.pinned)?;
    Ok(Json(json!({ "updated": true, "pinned": body.pinned })))
}

/// `PATCH /api/sessions/:id/incognito` — toggle per-session incognito mode.
pub async fn set_session_incognito(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<SessionIncognitoBody>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db.update_session_incognito(&id, body.enabled)?;
    Ok(Json(json!({ "updated": true })))
}

/// `PATCH /api/sessions/:id/working-dir` — persist the per-session working
/// directory. The core layer canonicalizes the path and rejects anything that
/// does not resolve to an existing directory on the server machine.
pub async fn set_session_working_dir(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<SessionWorkingDirBody>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db
        .update_session_working_dir(&id, body.working_dir)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "updated": true })))
}

/// `PATCH /api/sessions/:id/agent` — switch the agent bound to a session.
/// Only allowed before the session has any user/assistant messages — the core
/// layer enforces this and returns 400 otherwise.
pub async fn update_session_agent(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<SessionAgentBody>,
) -> Result<Json<Value>, AppError> {
    ctx.session_db
        .update_session_agent(&id, &body.agent_id)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "updated": true })))
}

/// `PATCH /api/sessions/:id/model` — pin the model used by this session.
/// Replaces the legacy "切模型 = 写全局 active_model" path. Future chat turns
/// on this session resolve provider/model from this row before falling back to
/// `agent.model.primary` / `config.active_model`.
pub async fn set_session_model(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<SessionModelBody>,
) -> Result<Json<Value>, AppError> {
    let provider_name = ha_core::config::cached_config()
        .providers
        .iter()
        .find(|p| p.id == body.provider_id && p.enabled)
        .map(|p| p.name.clone());
    ctx.session_db
        .update_session_model(
            &id,
            Some(&body.provider_id),
            provider_name.as_deref(),
            Some(&body.model_id),
        )
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "session:model_updated",
            json!({
                "sessionId": id,
                "providerId": body.provider_id,
                "modelId": body.model_id,
            }),
        );
    }
    Ok(Json(json!({ "updated": true })))
}

/// `POST /api/sessions/:id/purge-if-incognito` — hard-delete the session if
/// it is currently flagged incognito; no-op otherwise. Frontend calls this
/// when the user navigates away from the session.
pub async fn purge_session_if_incognito(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let purged = ctx.session_db.purge_session_if_incognito(&id)?;
    Ok(Json(json!({ "purged": purged })))
}

/// `GET /api/sessions/search` — full-text search message history.
pub async fn search_sessions(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<SearchSessionsQuery>,
) -> Result<Json<Vec<ha_core::session::SessionSearchResult>>, AppError> {
    let limit = q.limit.unwrap_or(80) as usize;

    let parsed_types: Option<Vec<ha_core::session::SessionTypeFilter>> =
        q.types.as_ref().map(|s| {
            s.split(',')
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .filter_map(ha_core::session::SessionTypeFilter::parse)
                .collect()
        });
    let type_slice = parsed_types.as_deref();

    let results =
        ctx.session_db
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
) -> Result<Json<Vec<ha_core::session::SessionSearchResult>>, AppError> {
    let limit = q.limit.unwrap_or(200) as usize;
    let results = ctx
        .session_db
        .search_messages(&q.query, None, Some(&id), None, limit)?;
    Ok(Json(results))
}

/// `GET /api/sessions/:id/messages/around?targetMessageId=N&before=40&after=20`
/// — load a window of messages centred on a target id.
///
/// Returns a JSON tuple `[messages, total, hasMoreBefore, hasMoreAfter]` to
/// match the shape Tauri IPC serialises for the same command.
pub async fn get_session_messages_around(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<MessagesAroundQuery>,
) -> Result<Json<Value>, AppError> {
    let before = q.before.unwrap_or(40);
    let after = q.after.unwrap_or(20);
    let (messages, total, has_more_before, has_more_after) = ctx
        .session_db
        .load_session_messages_around(&id, q.target_message_id, before, after)?;
    let messages = rewrite_messages_for_http(messages, ctx.api_key.as_deref());
    Ok(Json(json!([
        messages,
        total,
        has_more_before,
        has_more_after
    ])))
}

/// `GET /api/sessions/:id/messages/before?beforeId=N&limit=20` — page upward
/// from an anchor message, loading older messages. Returns a JSON tuple
/// `[messages, hasMoreBefore]` matching the Tauri
/// `load_session_messages_before_cmd` contract.
pub async fn get_session_messages_before(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<MessagesBeforeQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = q.limit.unwrap_or(20);
    let (messages, has_more) =
        ctx.session_db
            .load_session_messages_before(&id, q.before_id, limit)?;
    let messages = rewrite_messages_for_http(messages, ctx.api_key.as_deref());
    Ok(Json(json!([messages, has_more])))
}

/// `GET /api/sessions/:id/messages/after?afterId=N&limit=20` — page downward
/// from an anchor message, loading newer messages. Returns a JSON tuple
/// `[messages, hasMoreAfter]` matching the Tauri
/// `load_session_messages_after_cmd` contract.
pub async fn get_session_messages_after(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<MessagesAfterQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = q.limit.unwrap_or(20);
    let (messages, has_more) = ctx
        .session_db
        .load_session_messages_after(&id, q.after_id, limit)?;
    let messages = rewrite_messages_for_http(messages, ctx.api_key.as_deref());
    Ok(Json(json!([messages, has_more])))
}

/// `GET /api/sessions/:id/messages?limit=N` — load latest messages for a session.
///
/// Returns a JSON tuple `[messages, total, hasMore]` (same shape as Tauri IPC).
/// Default limit is 50.
pub async fn get_session_messages(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let (messages, total, has_more) = ctx.session_db.load_session_messages_latest(&id, limit)?;
    let messages = rewrite_messages_for_http(messages, ctx.api_key.as_deref());
    Ok(Json(json!([messages, total, has_more])))
}

/// `GET /api/sessions/:id/files/by-path?path=/abs/file&download=1` — serve a
/// file only when that exact canonical path is referenced by this session's
/// persisted tool side-output. This powers HTTP/Web clicks on the assistant
/// message's generated/modified file chips without exposing arbitrary local
/// files from the server machine.
pub async fn download_session_file_by_path(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<SessionFileByPathQuery>,
    request: Request,
) -> Result<Response, AppError> {
    let requested = q.path.trim();
    if requested.is_empty() {
        return Err(AppError::bad_request("missing path"));
    }

    let messages = ctx.session_db.load_session_messages(&id)?;
    let file_canon = authorized_canonical_file_path(requested, &messages).await?;
    let meta = tokio::fs::metadata(&file_canon)
        .await
        .map_err(|_| AppError::not_found("file not found"))?;
    if !meta.is_file() {
        return Err(AppError::bad_request("path is not a file"));
    }

    let mime = resolve_mime_for_path(
        &file_canon,
        MimeOpts {
            html_charset: false,
            sniff_fallback: true,
        },
    )
    .await;
    let disposition = content_disposition_for_file(
        &file_canon,
        &mime,
        parse_download_flag(q.download.as_deref()),
    );

    let mut response = ServeFile::new(&file_canon)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve session file: {}", e)))?
        .into_response();

    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &mime,
            cache_secs: 60,
            disposition: &disposition,
            no_referrer: false,
        },
    );

    Ok(response)
}

/// `GET /api/sessions/:id/stream-state` — snapshot of whether the session
/// currently has an active chat stream + its latest `seq` counter. Frontend
/// uses this on session switch to decide whether to attach the EventBus
/// reattach listener for a reloaded window.
pub async fn get_session_stream_state(
    Path(id): Path<String>,
) -> Result<Json<ha_core::chat_engine::SessionStreamState>, AppError> {
    Ok(Json(ha_core::chat_engine::session_stream_state(&id)))
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
/// uses camelCase to match `ha_core::context_compact::CompactResult`'s
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

/// `GET /api/sessions/:id/awareness-config` — read per-session override JSON.
pub async fn get_session_awareness_config(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let json = ctx.session_db.get_session_awareness_config_json(&id)?;
    Ok(Json(json!({ "json": json })))
}

/// `PATCH /api/sessions/:id/awareness-config` — write or clear override.
pub async fn set_session_awareness_config(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<AwarenessOverrideBody>,
) -> Result<Json<Value>, AppError> {
    // Validate the override JSON before persisting. A round-trip through
    // merge_override catches both syntax errors and type mismatches.
    if let Some(ref json_str) = body.json {
        if !json_str.trim().is_empty() {
            let base = ha_core::awareness::AwarenessConfig::default();
            ha_core::awareness::config::validate_override(&base, json_str)
                .map_err(|e| anyhow::anyhow!("invalid override JSON: {}", e))?;
        }
    }
    ctx.session_db
        .set_session_awareness_config_json(&id, body.json.as_deref())?;
    Ok(Json(json!({ "saved": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionQuery {
    pub format: String,
    #[serde(default)]
    pub include_thinking: bool,
    #[serde(default)]
    pub include_tools: bool,
}

/// `GET /api/sessions/:id/export?format=md&includeThinking=true&includeTools=true`
/// — return the serialized conversation as a binary download. Browser clients
/// consume this through `<a download>` / `URL.createObjectURL`. The Tauri
/// equivalent is the `export_session_cmd` IPC command, which writes to a
/// user-chosen path instead.
pub async fn export_session_http(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Query(q): Query<ExportSessionQuery>,
) -> Result<Response, AppError> {
    let fmt = ha_core::session::export::ExportFormat::parse(&q.format).ok_or_else(|| {
        AppError::bad_request(format!(
            "invalid export format `{}` (expected md / json / html)",
            q.format
        ))
    })?;
    let opts = ha_core::session::export::ExportOptions {
        format: fmt,
        include_thinking: q.include_thinking,
        include_tools: q.include_tools,
    };
    let payload = ha_core::session::export::export_session(ctx.session_db.as_ref(), &id, opts)?;

    // RFC 5987: provide both `filename=` (ASCII fallback) and `filename*=UTF-8''...`
    // so non-ASCII titles (e.g. CJK) survive the trip to the browser.
    let ascii_fallback = ascii_fallback_name(&payload.filename);
    let utf8_pct = percent_encode_filename(&payload.filename);
    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_fallback, utf8_pct
    );

    let mut response = (axum::http::StatusCode::OK, payload.body).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(payload.mime));
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition)
            .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"session.txt\"")),
    );
    Ok(response)
}

fn ascii_fallback_name(name: &str) -> String {
    let stripped: String = name
        .chars()
        .map(|c| if c.is_ascii() && c != '"' { c } else { '_' })
        .collect();
    if stripped.trim_matches('_').is_empty() {
        "session".to_string()
    } else {
        stripped
    }
}

fn percent_encode_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        // Conservative allow-list per RFC 5987 token chars; everything else
        // is percent-encoded so the browser reconstructs the original UTF-8.
        let ok = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            );
        if ok {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}
