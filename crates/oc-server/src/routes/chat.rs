use axum::extract::{Multipart, Path, State};
use axum::Json;

use super::helpers::parse_file_upload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use oc_core::agent::Attachment;
use oc_core::chat_engine::{ChatEngineParams, EventSink};
use oc_core::provider::{self, ActiveModel};
use oc_core::session;
use oc_core::tools;

use crate::error::AppError;
use crate::ws::chat_stream::ChatStreamRegistry;
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
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub tool_permission_mode: Option<String>,
    #[serde(default)]
    pub temperature_override: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
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

// ── WebSocket-backed EventSink ─────────────────────────────────

/// EventSink that broadcasts events to all WebSocket subscribers for a session.
///
/// Rewrites `tool_result` events before broadcast: strips `media_items[].localPath`
/// (never leak server filesystem paths to browsers) and stamps `?token=<api_key>`
/// onto `/api/attachments/*` URLs so `<img src>` / `<a href>` authenticate.
struct WsSink {
    session_id: String,
    registry: Arc<ChatStreamRegistry>,
    api_key: Option<String>,
}

impl EventSink for WsSink {
    fn send(&self, event: &str) {
        // EventSink::send is sync but broadcast is async. Use spawn_blocking-safe approach.
        let rewritten = oc_core::agent::rewrite_event_for_http(event, self.api_key.as_deref());
        let registry = self.registry.clone();
        let sid = self.session_id.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(registry.broadcast(&sid, &rewritten));
        });
    }
}

// ── Handlers ───────────────────────────────────────────────────

/// `POST /api/chat` — run chat engine, streaming events via WebSocket.
pub async fn chat(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    let db = ctx.session_db.clone();

    // Set tool permission mode if specified
    if let Some(ref mode_str) = body.tool_permission_mode {
        let mode = match mode_str.as_str() {
            "ask_every_time" => tools::ToolPermissionMode::AskEveryTime,
            "full_approve" => tools::ToolPermissionMode::FullApprove,
            _ => tools::ToolPermissionMode::Auto,
        };
        tools::set_tool_permission_mode(mode).await;
    }

    // Resolve agent ID
    let agent_id = body.agent_id.unwrap_or_else(|| "default".to_string());

    // Resolve or create session
    let sid = match body.session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            let meta = db.create_session(&agent_id)?;
            meta.id
        }
    };

    // Save user message to DB
    let user_msg = session::NewMessage::user(&body.message);
    let _ = db.append_message(&sid, &user_msg);

    // Auto-generate title from first user message
    if let Ok(Some(meta)) = db.get_session(&sid) {
        if meta.title.is_none() && meta.message_count <= 1 {
            let title = session::auto_title(&body.message);
            let _ = db.update_session_title(&sid, &title);
        }
    }

    // Load app config (cached after first call)
    let store = oc_core::config::cached_config();

    // Resolve model chain
    let agent_def = oc_core::agent_loader::load_agent(&agent_id).ok();
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
        return Err(AppError::bad_request(
            "No model configured. Please add a provider and set an active model.",
        ));
    }

    // Resolve feature flags from store
    let web_search_enabled = oc_core::tools::web_search::has_enabled_provider(&store.web_search);
    let notification_enabled = store.notification.enabled;
    let image_gen_config = {
        if oc_core::tools::image_generate::has_configured_provider_from_config(
            &store.image_generate,
        ) {
            let mut cfg = store.image_generate.clone();
            oc_core::tools::image_generate::backfill_providers(&mut cfg);
            Some(cfg)
        } else {
            None
        }
    };
    let canvas_enabled = store.canvas.enabled;
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
        let mut cancels = ctx.chat_cancels.write().unwrap();
        cancels.insert(sid.clone(), cancel.clone());
    }

    // Build event sink that broadcasts to WebSocket subscribers
    let event_sink: Arc<dyn EventSink> = Arc::new(WsSink {
        session_id: sid.clone(),
        registry: ctx.chat_streams.clone(),
        api_key: ctx.api_key.clone(),
    });

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
        web_search_enabled,
        notification_enabled,
        image_gen_config,
        canvas_enabled,
        compact_config,
        extra_system_context: None,
        reasoning_effort: Some(effort),
        cancel: cancel.clone(),
        plan_agent_mode: None,
        plan_mode_allow_paths: None,
        skill_allowed_tools: Vec::new(),
        auto_approve_tools: false,
        event_sink,
    };

    let result = oc_core::chat_engine::run_chat_engine(engine_params).await;

    // Clean up per-session cancel flag
    {
        ctx.chat_cancels.write().unwrap().remove(&sid);
    }

    let result = result.map_err(|e| AppError::internal(e))?;

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
    let cancels = ctx.chat_cancels.read().unwrap();
    if let Some(sid) = body.session_id.as_deref() {
        if let Some(cancel) = cancels.get(sid) {
            cancel.store(true, Ordering::SeqCst);
            return Ok(Json(json!({ "stopped": true, "scope": "session" })));
        }
        return Ok(Json(
            json!({ "stopped": false, "reason": "no active chat for session" }),
        ));
    }
    let mut count = 0usize;
    for cancel in cancels.values() {
        cancel.store(true, Ordering::SeqCst);
        count += 1;
    }
    Ok(Json(json!({ "stopped": true, "scope": "all", "count": count })))
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
    let store = oc_core::config::cached_config();
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

    let prompt = oc_core::agent::build_system_prompt_with_session(
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

    let path =
        oc_core::attachments::save_attachment_bytes(session_id, &upload.file_name, &upload.file_data)
            .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(json!({ "path": path })))
}

/// `POST /api/system-prompt` — body-based alias of `get_system_prompt`.
pub async fn get_system_prompt_post(
    Json(body): Json<SystemPromptBody>,
) -> Result<Json<Value>, AppError> {
    let agent_id = body.agent_id.unwrap_or_else(|| "default".to_string());
    let store = oc_core::config::cached_config();
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
    let prompt = oc_core::agent::build_system_prompt_with_session(
        &agent_id,
        &model,
        &provider_name,
        body.session_id.as_deref(),
    );
    Ok(Json(json!({ "system_prompt": prompt })))
}

/// `GET /api/chat/tools` — list available built-in tools.
pub async fn list_tools() -> Result<Json<Vec<Value>>, AppError> {
    let mut all = tools::get_available_tools();
    all.push(tools::get_notification_tool());
    let tools_json: Vec<Value> = all
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "internal": t.internal,
            })
        })
        .collect();
    Ok(Json(tools_json))
}
