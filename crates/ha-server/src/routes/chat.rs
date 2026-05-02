use axum::extract::{Multipart, Path, State};
use axum::Json;

use super::helpers::parse_file_upload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ha_core::agent::Attachment;
use ha_core::chat_engine::{ChatEngineParams, EventSink, NoopEventSink};
use ha_core::permission::SessionMode;
use ha_core::provider::{self, ActiveModel};
use ha_core::session;
use ha_core::tools;

use crate::error::AppError;
use crate::AppContext;

// ── Request / Response Types ───────────────────────────────────
//
// All HTTP request bodies use `#[serde(rename_all = "camelCase")]` because
// the frontend `transport-http.ts::call()` ships args as-is via
// `JSON.stringify(remainingArgs)`. Frontend code uses camelCase keys
// throughout (`sessionId`, `agentId`, `requestId`, ...), so the matching
// HTTP body structs MUST accept camelCase to deserialize successfully.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub incognito: Option<bool>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Per-session permission mode. When provided, the session's
    /// `permission_mode` column is updated before chat starts.
    #[serde(default)]
    pub permission_mode: Option<SessionMode>,
    #[serde(default)]
    pub temperature_override: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// See Tauri `chat` command — DB stores this while `message` goes to the LLM.
    #[serde(default)]
    pub display_text: Option<String>,
    /// When true, persists the user row with
    /// `attachments_meta = {"plan_trigger": true}` so the UI renders it as a
    /// Plan Mode approve/resume chip (mirrors the Tauri `chat` command).
    #[serde(default)]
    pub is_plan_trigger: Option<bool>,
    /// Draft working dir picked before the session was materialized. Only
    /// honored when this call also creates the session (mirrors the Tauri
    /// `chat` command).
    #[serde(default)]
    pub working_dir: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub session_id: String,
    pub response: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StopChatRequest {
    /// When omitted, cancels every running chat (mirrors the Tauri command's
    /// "stop the current chat" semantics — frontend calls `stop_chat` with
    /// no args).
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub response: String,
}

/// Body-based approval response: alias for `/api/chat/approval` (no path
/// param) — matches the frontend `respond_to_approval` command which sends
/// `{requestId, response}` in the JSON body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalBodyRequest {
    pub request_id: String,
    pub response: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptQuery {
    pub agent_id: Option<String>,
    /// Optional session id — when set, the returned prompt is built with
    /// project context (if the session belongs to a project).
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptBody {
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Optional session id — when set, the returned prompt is built with
    /// project context (if the session belongs to a project).
    #[serde(default)]
    pub session_id: Option<String>,
}

// ── Handlers ───────────────────────────────────────────────────

/// `POST /api/chat` — run chat engine, streaming events via WebSocket.
pub async fn chat(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    let db = ctx.session_db.clone();

    // `permission_mode` body field is consumed below after we resolve the
    // session id (we need a session_id to persist).
    let permission_mode_pending = body.permission_mode;

    // Resolve agent ID. Explicit caller wins; otherwise existing sessions use
    // their stored agent, while new sessions inherit the app-wide default.
    let explicit_agent_id = body
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned);
    let existing_session_id = body
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let agent_id = if let Some(id) = explicit_agent_id {
        id
    } else if let Some(session_id) = existing_session_id {
        db.get_session(session_id)?
            .map(|session| session.agent_id)
            .unwrap_or_else(|| ha_core::agent::resolver::resolve_default_agent_id(None, None))
    } else {
        ha_core::agent::resolver::resolve_default_agent_id(None, None)
    };

    // Resolve or create session
    let mut new_session_created = false;
    let sid = match body.session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            let meta = db.create_session_with_project(&agent_id, None, body.incognito)?;
            new_session_created = true;
            meta.id
        }
    };

    // Apply draft working dir picked before the session existed. Mirrors the
    // Tauri `chat` command — explicit-session callers must use the dedicated
    // setter to change it.
    if new_session_created {
        if let Some(wd) = body.working_dir.as_ref().filter(|s| !s.trim().is_empty()) {
            db.update_session_working_dir(&sid, Some(wd.clone()))
                .map_err(|e| AppError::bad_request(e.to_string()))?;
        }
    }

    // Persist per-session permission mode if the body included one.
    if let Some(mode) = permission_mode_pending {
        db.update_session_permission_mode(&sid, mode)
            .map_err(|e| AppError::bad_request(e.to_string()))?;
    }

    let _active_turn_guard = ha_core::chat_engine::active_turn::try_acquire(
        &sid,
        ha_core::chat_engine::stream_seq::ChatSource::Http,
    )
    .map_err(|e| {
        AppError::conflict_with_code(
            ha_core::chat_engine::stream_seq::ACTIVE_STREAM_ERROR_CODE,
            e.to_string(),
        )
    })?;

    // Prefer display_text for DB/title, fall back to the LLM-bound message.
    let persisted_content = ha_core::non_empty_trim_or(body.display_text.as_deref(), &body.message);

    // Save user message to DB
    let mut user_msg = session::NewMessage::user(persisted_content);
    // Plan Mode trigger marker — UI renders these as a system chip instead of
    // a regular user bubble. Mirrors the Tauri command.
    if body.is_plan_trigger.unwrap_or(false) {
        user_msg.attachments_meta = Some(serde_json::json!({ "plan_trigger": true }).to_string());
    }
    let _ = db.append_message(&sid, &user_msg);

    // Auto-generate fallback title from first user message (prefer display text so titles read naturally).
    let _ = session::ensure_first_message_title(&db, &sid, persisted_content);

    // Load app config (cached after first call)
    let store = ha_core::config::cached_config();

    // Resolve model chain
    let agent_def = ha_core::agent_loader::load_agent(&agent_id).ok();
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();

    let (primary, fallbacks) = if let Some(ref override_str) = body.model_override {
        let mut cfg = agent_model_config.clone();
        if provider::parse_model_ref(override_str).is_some() {
            cfg.primary = Some(override_str.clone());
        }
        provider::resolve_model_chain(&cfg, &store)
    } else {
        provider::resolve_model_chain(&agent_model_config, &store)
    };

    let mut model_chain: Vec<ActiveModel> = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain
            .iter()
            .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
        {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        let err = "No model configured. Please add a provider and set an active model.";
        ha_core::chat_engine::persist_failed_turn_context(&db, &sid, &body.message, err);
        let _ = db.append_message(&sid, &session::NewMessage::error_event(err));
        return Err(AppError::bad_request(err));
    }

    let compact_config = store.compact.clone();

    // Resolve temperature: request > agent > global
    let resolved_temperature = body.temperature_override.or_else(|| {
        agent_def
            .as_ref()
            .and_then(|def| def.config.model.temperature)
            .or(store.temperature)
    });

    let effort = body
        .reasoning_effort
        .unwrap_or_else(|| "medium".to_string());

    // Create per-session cancel flag
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut cancels = ctx
            .chat_cancels
            .write()
            .map_err(|_| AppError::internal("chat cancel registry lock poisoned"))?;
        cancels.insert(sid.clone(), cancel.clone());
    }

    // HTTP stream delivery uses `/ws/events` via `chat:stream_delta`; the
    // EventBus bridge performs the HTTP attachment URL rewrite there.
    let event_sink: Arc<dyn EventSink> = Arc::new(NoopEventSink);

    let engine_params = ChatEngineParams {
        session_id: sid.clone(),
        agent_id: agent_id.clone(),
        message: body.message.clone(),
        attachments: body.attachments,
        session_db: db.clone(),
        model_chain,
        providers: store.providers.clone(),
        codex_token: None,
        resolved_temperature,
        compact_config,
        extra_system_context: None,
        reasoning_effort: Some(effort),
        cancel: cancel.clone(),
        plan_agent_mode: None,
        plan_mode_allow_paths: None,
        skill_allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        subagent_depth: 0,
        steer_run_id: None,
        auto_approve_tools: false,
        follow_global_reasoning_effort: true,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        source: ha_core::chat_engine::stream_seq::ChatSource::Http,
        event_sink,
    };

    let result = ha_core::chat_engine::run_chat_engine(engine_params).await;

    // Clean up per-session cancel flag
    {
        let mut cancels = ctx
            .chat_cancels
            .write()
            .map_err(|_| AppError::internal("chat cancel registry lock poisoned"))?;
        cancels.remove(&sid);
    }

    let result = result.map_err(AppError::internal)?;

    Ok(Json(ChatResponse {
        session_id: sid,
        response: result.response,
    }))
}

/// `POST /api/chat/stop` — stop ongoing chat(s).
///
/// When the request body provides `sessionId`, only that session's cancel
/// flag is flipped. Otherwise every running chat is cancelled (this matches
/// the desktop Tauri command which has no per-session targeting). Accepts
/// either `{}` or omitted body — `axum::Json` with a `Default` body handles
/// `{}`; for a completely empty body the Tauri caller wouldn't reach this
/// route anyway.
pub async fn stop_chat(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<StopChatRequest>,
) -> Result<Json<Value>, AppError> {
    let mut stopped = false;
    let mut stopped_count = 0usize;
    let mut active_session_ids = Vec::new();
    {
        let cancels = ctx
            .chat_cancels
            .read()
            .map_err(|_| AppError::internal("chat cancel registry lock poisoned"))?;
        if let Some(sid) = body.session_id.as_deref() {
            if let Some(cancel) = cancels.get(sid) {
                cancel.store(true, Ordering::SeqCst);
                stopped = true;
                stopped_count = 1;
            }
        } else {
            for (sid, cancel) in cancels.iter() {
                cancel.store(true, Ordering::SeqCst);
                active_session_ids.push(sid.clone());
                stopped_count += 1;
            }
            stopped = stopped_count > 0;
        }
    }

    let runtime_cancellations = if let Some(sid) = body.session_id.as_deref() {
        ha_core::runtime_tasks::cancel_runtime_tasks_for_session(Some(sid)).await?
    } else if active_session_ids.is_empty() {
        ha_core::runtime_tasks::cancel_runtime_tasks_for_session(None).await?
    } else {
        let mut out = Vec::new();
        for sid in active_session_ids {
            out.extend(ha_core::runtime_tasks::cancel_runtime_tasks_for_session(Some(&sid)).await?);
        }
        out
    };

    if body.session_id.is_some() {
        return Ok(Json(json!({
            "stopped": stopped,
            "scope": "session",
            "reason": if stopped { Value::Null } else { json!("no active chat for session") },
            "runtimeCancellations": runtime_cancellations,
        })));
    }
    Ok(Json(json!({
        "stopped": stopped,
        "scope": "all",
        "count": stopped_count,
        "runtimeCancellations": runtime_cancellations,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionModeBody {
    pub mode: SessionMode,
    pub session_id: String,
}

/// `POST /api/chat/permission-mode` — set the per-session permission mode.
/// Persisted to the `sessions.permission_mode` column.
pub async fn set_permission_mode(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<PermissionModeBody>,
) -> Result<Json<Value>, AppError> {
    if body.session_id.is_empty() {
        return Err(AppError::bad_request("sessionId required"));
    }
    ctx.session_db
        .update_session_permission_mode(&body.session_id, body.mode)?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/chat/approval/{request_id}` — respond to a tool approval request.
pub async fn respond_to_approval(
    Path(request_id): Path<String>,
    Json(body): Json<ApprovalRequest>,
) -> Result<Json<Value>, AppError> {
    let approval_response = match body.response.as_str() {
        "allow_once" => tools::ApprovalResponse::AllowOnce,
        "allow_always" => tools::ApprovalResponse::AllowAlways,
        "deny" => tools::ApprovalResponse::Deny,
        _ => {
            return Err(AppError::bad_request(format!(
                "Invalid approval response: {}. Expected: allow_once, allow_always, deny",
                body.response
            )));
        }
    };
    tools::submit_approval_response(&request_id, approval_response).await?;
    Ok(Json(json!({ "approved": true })))
}

/// `GET /api/chat/system-prompt?agent_id=xxx` — return the assembled system prompt.
pub async fn get_system_prompt(
    axum::extract::Query(q): axum::extract::Query<SystemPromptQuery>,
) -> Result<Json<Value>, AppError> {
    let agent_id = q.agent_id.unwrap_or_else(|| "default".to_string());

    // Resolve model and provider name from active model in store
    let store = ha_core::config::cached_config();
    let (model, provider_name) = if let Some(ref active) = store.active_model {
        let prov = store.providers.iter().find(|p| p.id == active.provider_id);
        let model_id = active.model_id.clone();
        let pname = prov
            .map(|p| p.api_type.display_name().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        (model_id, pname)
    } else {
        ("unknown".to_string(), "Unknown".to_string())
    };

    let prompt = ha_core::agent::build_system_prompt_with_session(
        &agent_id,
        &model,
        &provider_name,
        q.session_id.as_deref(),
    );
    Ok(Json(json!({ "system_prompt": prompt })))
}

/// `POST /api/chat/approval` — body-based alias of `respond_to_approval`.
///
/// Frontend `transport-http` maps `respond_to_approval` to this path without
/// a `{request_id}` path parameter; the id ships in the JSON body instead.
pub async fn respond_to_approval_body(
    Json(body): Json<ApprovalBodyRequest>,
) -> Result<Json<Value>, AppError> {
    let approval_response = match body.response.as_str() {
        "allow_once" => tools::ApprovalResponse::AllowOnce,
        "allow_always" => tools::ApprovalResponse::AllowAlways,
        "deny" => tools::ApprovalResponse::Deny,
        _ => {
            return Err(AppError::bad_request(format!(
                "Invalid approval response: {}. Expected: allow_once, allow_always, deny",
                body.response
            )));
        }
    };
    tools::submit_approval_response(&body.request_id, approval_response).await?;
    Ok(Json(json!({ "approved": true })))
}

/// `POST /api/chat/attachment` — persist an uploaded attachment (multipart/form-data).
///
/// Multipart fields: `file` (required), `sessionId` / `fileName` / `mimeType` (optional text).
pub async fn save_attachment(multipart: Multipart) -> Result<Json<Value>, AppError> {
    let upload = parse_file_upload(multipart).await?;
    let session_id = upload.extra_fields.get("sessionId").map(|s| s.as_str());

    let path = ha_core::attachments::save_attachment_bytes(
        session_id,
        &upload.file_name,
        &upload.file_data,
    )
    .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(json!({ "path": path })))
}

/// `POST /api/system-prompt` — body-based alias of `get_system_prompt`.
pub async fn get_system_prompt_post(
    Json(body): Json<SystemPromptBody>,
) -> Result<Json<Value>, AppError> {
    let agent_id = body.agent_id.unwrap_or_else(|| "default".to_string());
    let store = ha_core::config::cached_config();
    let (model, provider_name) = if let Some(ref active) = store.active_model {
        let prov = store.providers.iter().find(|p| p.id == active.provider_id);
        let model_id = active.model_id.clone();
        let pname = prov
            .map(|p| p.api_type.display_name().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        (model_id, pname)
    } else {
        ("unknown".to_string(), "Unknown".to_string())
    };
    let prompt = ha_core::agent::build_system_prompt_with_session(
        &agent_id,
        &model,
        &provider_name,
        body.session_id.as_deref(),
    );
    Ok(Json(json!({ "system_prompt": prompt })))
}

/// `GET /api/chat/tools` — list available built-in tools (mirrors the Tauri
/// `list_builtin_tools` command). Each entry carries tier metadata so the
/// frontend can group + style by tier.
pub async fn list_tools() -> Result<Json<Vec<Value>>, AppError> {
    let cfg = ha_core::config::cached_config();
    let tools_json: Vec<Value> = tools::dispatch::all_dispatchable_tools()
        .iter()
        .map(|t| t.to_api_metadata(&cfg))
        .collect();
    Ok(Json(tools_json))
}
