use axum::extract::{Multipart, Path, State};
use axum::Json;

use super::helpers::parse_file_upload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ha_core::agent::Attachment;
use ha_core::chat_engine::{ChatEngineParams, EventSink, NoopEventSink};
use ha_core::permission::{SandboxMode, SessionMode};
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
    /// Per-session sandbox mode. When provided, the session's
    /// `sandbox_mode` column is updated before chat starts.
    #[serde(default)]
    pub sandbox_mode: Option<SandboxMode>,
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
    /// Structured payload for plan inline-comment messages. Stamped into
    /// `attachments_meta = {"plan_comment": {...}}` for the desktop GUI to
    /// render PlanCommentBubble. IM channels ignore it. (Mirrors the Tauri
    /// `chat` command.)
    #[serde(default)]
    pub plan_comment: Option<serde_json::Value>,
    /// Draft working dir picked before the session was materialized. Only
    /// honored when this call also creates the session (mirrors the Tauri
    /// `chat` command).
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Composer-staged KB attaches. Only honored when this call also creates the
    /// session (mirrors `working_dir` / the Tauri `chat` command). No-op for
    /// incognito.
    #[serde(default)]
    pub kb_attachments: Vec<ha_core::knowledge::types::KbAttachInput>,
    /// Tool-visibility scope (`"knowledge"`). Set by the knowledge-space sidebar
    /// chat to trim the injected tool set; `None` (default) for normal chats.
    #[serde(default)]
    pub tool_scope: Option<String>,
    /// Knowledge-space sidebar chat: the note open when the conversation started.
    /// Only honored on the auto-create branch — promotes the new session into a
    /// KB chat thread.
    #[serde(default)]
    pub kb_anchor_note: Option<String>,
    /// Lazy project binding: when a project draft sends its first message the
    /// client carries the project id here so the auto-create branch materializes
    /// the session inside the project. Ignored when `session_id` is set; mutually
    /// exclusive with incognito (coerced in `create_session_with_project`).
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueTurnUserMessageRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    pub session_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub display_text: Option<String>,
    #[serde(default)]
    pub is_plan_trigger: Option<bool>,
    #[serde(default)]
    pub plan_comment: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnUserMessageRequest {
    pub session_id: String,
    pub turn_id: String,
    pub request_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub session_id: String,
    pub response: String,
    pub turn_id: String,
    /// Set to the block reason when the `UserPromptSubmit` preflight hook
    /// short-circuited the turn before a stream started. `None` on the
    /// normal happy path. The HTTP transport reads this to synthesize a
    /// stream notice for the UI so the user actually sees the block (the
    /// stream end signal that normally carries the notice never fires when
    /// no stream was opened).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

fn validate_http_mention_attachment(session_id: &str, file_path: &str) -> Result<(), AppError> {
    let requested = PathBuf::from(file_path);
    if !requested.is_absolute() {
        return Err(AppError::bad_request(
            "mention attachment path must be absolute",
        ));
    }

    let canon = requested
        .canonicalize()
        .map_err(|_| AppError::forbidden("mention attachment is outside the session workspace"))?;
    let scope = ha_core::filesystem::WorkspaceScope::for_session(session_id)
        .map_err(|_| AppError::forbidden("mention attachment is outside the session workspace"))?;
    if scope.contains(&canon) {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "mention attachment is outside the session workspace",
        ))
    }
}

fn validate_http_chat_attachments(
    session_id: &str,
    attachments: &[Attachment],
) -> Result<(), AppError> {
    for att in attachments {
        // "quote" attachments carry the snippet in `data`; their `file_path` is
        // only a reference label (never read from disk), so it's safe.
        match (att.source.as_deref(), att.file_path.as_deref()) {
            (_, None) => {}
            (Some("upload") | Some("quote"), Some(_)) => {}
            (Some(source), Some(_)) if source == ha_core::attachments::PASTED_TEXT_SOURCE => {}
            (Some("mention"), Some(path)) => validate_http_mention_attachment(session_id, path)?,
            _ => {
                return Err(AppError::bad_request(
                    "HTTP chat attachments must be uploaded through /api/chat/attachment",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StopChatRequest {
    /// When omitted, cancels every running chat (mirrors the Tauri command's
    /// "stop the current chat" semantics — frontend calls `stop_chat` with
    /// no args).
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
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
    Json(mut body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    let db = ctx.session_db.clone();

    // Per-session mode fields are consumed below after we resolve the
    // session id (we need a session_id to persist).
    let permission_mode_pending = body.permission_mode;
    let sandbox_mode_pending = body.sandbox_mode;

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
    // Normalize the lazy project binding once (trim + empty→None) so a blank
    // project_id neither resolves a bogus project agent nor persists a
    // non-matching project_id (which would orphan the session and wrongly coerce
    // incognito off). Used for both agent resolution and create.
    let project_id = body
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|pid| !pid.is_empty())
        .map(str::to_owned);
    let agent_id = if let Some(id) = explicit_agent_id {
        id
    } else if let Some(session_id) = existing_session_id {
        db.get_session(session_id)?
            .map(|session| session.agent_id)
            .unwrap_or_else(|| ha_core::agent::resolver::resolve_default_agent_id(None, None))
    } else {
        // New session: resolve via the project's default-agent chain when a
        // lazy project binding is present, mirroring the create_session route.
        let project = project_id
            .as_deref()
            .and_then(|pid| ctx.project_db.get(pid).ok().flatten());
        ha_core::agent::resolver::resolve_default_agent_id(project.as_ref(), None)
    };

    // Resolve or create session
    let mut new_session_created = false;
    let sid = match body.session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            // `project_id` binds the new session to a project on this lazy-create
            // branch; incognito is coerced off when a project is set.
            let meta =
                db.create_session_with_project(&agent_id, project_id.as_deref(), body.incognito)?;
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
        ha_core::knowledge::service::apply_draft_attachments(
            &sid,
            body.incognito.unwrap_or(false),
            &body.kb_attachments,
        );
    }

    // Persist per-session permission mode if the body included one.
    if let Some(mode) = permission_mode_pending {
        db.update_session_permission_mode(&sid, mode)
            .map_err(|e| AppError::bad_request(e.to_string()))?;
    }
    if let Some(mode) = sandbox_mode_pending {
        db.update_session_sandbox_mode(&sid, mode)
            .map_err(|e| AppError::bad_request(e.to_string()))?;
    }

    // Load app/agent config before resolving per-turn settings.
    let store = ha_core::config::cached_config();
    let agent_def = ha_core::agent_loader::load_agent(&agent_id).ok();
    let agent_default_effort = agent_def
        .as_ref()
        .and_then(|def| def.config.model.reasoning_effort.clone());

    let requested_effort = body
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .map(str::to_string);
    let session_effort = db.get_session(&sid)?.and_then(|meta| meta.reasoning_effort);
    let global_effort = if let Some(cell) = ha_core::get_reasoning_effort_cell() {
        cell.lock().await.clone()
    } else {
        "medium".to_string()
    };
    let effort = requested_effort
        .or(session_effort)
        .or(agent_default_effort)
        .unwrap_or(global_effort);
    if !ha_core::agent::is_valid_reasoning_effort(&effort) {
        return Err(AppError::bad_request(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort,
            ha_core::agent::VALID_REASONING_EFFORTS
        )));
    }
    db.update_session_reasoning_effort(&sid, Some(&effort))?;
    if let Some(cell) = ha_core::get_reasoning_effort_cell() {
        *cell.lock().await = effort.clone();
    }

    let turn_id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    let _active_turn_guard = ha_core::chat_engine::active_turn::try_acquire(
        &sid,
        ha_core::chat_engine::stream_seq::ChatSource::Http,
        turn_id.clone(),
        cancel.clone(),
    )
    .map_err(|e| {
        AppError::conflict_with_code(
            ha_core::chat_engine::stream_seq::ACTIVE_STREAM_ERROR_CODE,
            e.to_string(),
        )
    })?;

    // Prefer display_text for DB/title, fall back to the LLM-bound message.
    let raw_prompt = ha_core::non_empty_trim_or(body.display_text.as_deref(), &body.message);

    // Preflight chokepoint: every user-message entry point routes through this
    // before persisting. Pass-through in Phase 0.1; PR 1.2 runs the
    // `UserPromptSubmit` hook here (may block / rewrite the prompt).
    let effective_prompt = match ha_core::agent::preflight::user_prompt_preflight(
        ha_core::agent::preflight::PreflightArgs {
            session_id: &sid,
            agent_id: Some(agent_id.as_str()),
            raw_prompt,
        },
    )
    .await
    {
        ha_core::agent::preflight::PreflightOutcome::Proceed { effective_prompt } => {
            effective_prompt
        }
        ha_core::agent::preflight::PreflightOutcome::Block { reason } => {
            // A UserPromptSubmit hook blocked the prompt: record a UI-only event
            // marker (excluded from LLM context) and return the reason as the
            // response — no user message persisted, no turn run. `blocked_reason`
            // is the structured signal the HTTP transport reads to synthesize
            // a stream notice (parity with the desktop path, which sends a
            // `{"type":"text","text":notice}` event on the on_event channel).
            let notice = format!("🚫 {reason}");
            // KB sidebar lazy-create: a blocked first message must leave NO
            // session behind (no hidden zombie, no stray regular row in the
            // main list / picker / FTS). Drop the freshly auto-created session;
            // `blocked_reason` still carries the notice to the transport.
            if new_session_created && body.tool_scope.as_deref() == Some("knowledge") {
                let _ = db.delete_session(&sid);
                return Ok(Json(ChatResponse {
                    session_id: sid,
                    response: notice.clone(),
                    turn_id,
                    blocked_reason: Some(notice),
                }));
            }
            let _ = db.append_message(&sid, &session::NewMessage::event(&notice));
            return Ok(Json(ChatResponse {
                session_id: sid,
                response: notice.clone(),
                turn_id,
                blocked_reason: Some(notice),
            }));
        }
    };

    // KB sidebar chat: promote the freshly-created session into a knowledge
    // thread now that preflight has passed (mirrors the Tauri command). Doing it
    // in the auto-create block left a hidden `kind=Knowledge` zombie + thread row
    // when a UserPromptSubmit hook blocked the first message.
    if new_session_created && body.tool_scope.as_deref() == Some("knowledge") {
        if let Some(kb_id) = body.kb_attachments.first().map(|a| a.kb_id.clone()) {
            ha_core::knowledge::service::mark_session_as_kb_thread(
                &sid,
                &kb_id,
                body.kb_anchor_note.as_deref(),
            );
        }
    }

    // Attachments: validate + persist AFTER the preflight, so a blocked prompt
    // returns above before any attachment IO touches disk. The DB content is the
    // hook-rewritten `effective_prompt`, so the separate `persisted_content`
    // main computed (identical to `raw_prompt`, now consumed by the preflight) is
    // dropped.
    validate_http_chat_attachments(&sid, &body.attachments)?;
    let attachments_meta =
        ha_core::attachments::persist_chat_user_attachments_meta(&sid, &mut body.attachments)
            .map_err(|e| AppError::internal(e.to_string()))?;

    // Save user message to DB
    let mut user_msg = session::NewMessage::user(&effective_prompt)
        .with_source(ha_core::chat_engine::ChatSource::Http);
    user_msg.attachments_meta = session::build_chat_user_attachments_meta(
        body.is_plan_trigger.unwrap_or(false),
        body.plan_comment.as_ref(),
        attachments_meta,
    );
    let user_message_id = db.append_message(&sid, &user_msg).ok();
    let _turn = db.create_chat_turn_with_id(
        &turn_id,
        &sid,
        ha_core::chat_engine::ChatSource::Http.as_str(),
        None,
        user_message_id,
    )?;

    // Auto-generate fallback title from first user message (prefer display text so titles read naturally).
    let _ = session::ensure_first_message_title(&db, &sid, &effective_prompt);

    // Resolve model chain
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();

    // Session-scoped model pin trumps agent.primary and config.active_model
    // when no explicit per-turn override was provided. Mirrors the desktop
    // commands::chat path so the two transports stay in sync.
    let session_pinned_model: Option<String> = if body.model_override.is_none() {
        db.get_session(&sid).ok().flatten().and_then(|meta| {
            match (meta.provider_id, meta.model_id) {
                (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => {
                    Some(format!("{}::{}", p, m))
                }
                _ => None,
            }
        })
    } else {
        None
    };

    let (primary, fallbacks) = if let Some(ref override_str) = body.model_override {
        let mut cfg = agent_model_config.clone();
        if provider::parse_model_ref(override_str).is_some() {
            cfg.primary = Some(override_str.clone());
        }
        provider::resolve_model_chain(&cfg, &store)
    } else if let Some(ref pinned) = session_pinned_model {
        let mut cfg = agent_model_config.clone();
        cfg.primary = Some(pinned.clone());
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
        // No LLM call was attempted → NoProfileAvailable. finalize
        // writes the marker into context_json, the role=event row, and
        // closes chat_turn — replacing the old hand-rolled
        // finish_chat_turn_once + persist_failed_turn_context +
        // error_event triad.
        let partial = ha_core::chat_engine::finalize::PartialMeta {
            user_message: Some(body.message.clone()),
            turn_id: Some(turn_id.clone()),
            ..Default::default()
        };
        let _ = ha_core::chat_engine::finalize::finalize_turn_context_blocking(
            &db,
            &sid,
            ha_core::chat_engine::finalize::TerminationReason::NoProfileAvailable,
            partial,
            ha_core::chat_engine::ChatSource::Http,
        );
        ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
            &sid,
            None,
            Some(&turn_id),
            Some(session::ChatTurnStatus::Failed),
            None,
            Some(err),
        );
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

    // Register per-session cancel flag after validation. The active-turn
    // guard above already prevents duplicate user-message persistence.
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
        turn_id: Some(turn_id.clone()),
        message: body.message.clone(),
        display_text: body.display_text.clone(),
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
        plan_context_override: None,
        skill_allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        tool_scope: ha_core::tools::ToolScope::from_str_opt(body.tool_scope.as_deref()),
        subagent_depth: 0,
        steer_run_id: None,
        // Honors `--auto-approve-tools` / `HA_SERVER_AUTO_APPROVE_TOOLS=1`
        // for headless / Docker deployments where the HTTP client doesn't
        // implement an approval handler. Engine gates (dangerous-commands,
        // protected paths, plan-mode ask) still run; this just flips the
        // same switch IM auto-approve accounts use.
        auto_approve_tools: crate::auto_approve::is_active(),
        follow_global_reasoning_effort: true,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        source: ha_core::chat_engine::stream_seq::ChatSource::Http,
        origin_source: None,
        // HTTP owner turn — KB access via attach, not the IM opt-in gate.
        channel_kb_context: None,
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
        turn_id,
        blocked_reason: None,
    }))
}

/// `POST /api/chat/turn-message` — queue a user message to be injected at the
/// next safe tool-loop boundary of the active turn.
pub async fn queue_turn_user_message(
    Json(body): Json<QueueTurnUserMessageRequest>,
) -> Result<Json<ha_core::chat_engine::turn_injection::QueueTurnUserMessageResult>, AppError> {
    validate_http_chat_attachments(&body.session_id, &body.attachments)?;
    let result = ha_core::chat_engine::turn_injection::enqueue(
        ha_core::chat_engine::turn_injection::QueueTurnUserMessageArgs {
            request_id: body.request_id,
            session_id: body.session_id,
            turn_id: body.turn_id,
            message: body.message,
            display_text: body.display_text,
            attachments: body.attachments,
            is_plan_trigger: body.is_plan_trigger.unwrap_or(false),
            plan_comment: body.plan_comment,
        },
    );
    Ok(Json(result))
}

/// `POST /api/chat/turn-message/cancel` — cancel a not-yet-injected queued
/// message for an active turn.
pub async fn cancel_queued_turn_user_message(
    Json(body): Json<CancelQueuedTurnUserMessageRequest>,
) -> Result<Json<ha_core::chat_engine::turn_injection::CancelQueuedTurnMessageResult>, AppError> {
    let result = ha_core::chat_engine::turn_injection::cancel(
        &body.session_id,
        &body.turn_id,
        &body.request_id,
    );
    Ok(Json(result))
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
    let mut watchdog_turns = Vec::new();
    {
        let cancels = ctx
            .chat_cancels
            .read()
            .map_err(|_| AppError::internal("chat cancel registry lock poisoned"))?;
        if let Some(sid) = body.session_id.as_deref() {
            if let Some(active) = ha_core::chat_engine::active_turn::current(sid) {
                let matches_turn = body
                    .turn_id
                    .as_deref()
                    .map(|id| id == active.turn_id)
                    .unwrap_or(true);
                if matches_turn {
                    active.cancel.store(true, Ordering::SeqCst);
                    let _ = ctx.session_db.mark_chat_turn_cancelling(
                        &active.turn_id,
                        session::ChatTurnInterruptReason::UserStop,
                    );
                    ha_core::chat_engine::stream_broadcast::broadcast_turn_status(
                        sid,
                        &active.turn_id,
                        session::ChatTurnStatus::Cancelling,
                        Some(session::ChatTurnInterruptReason::UserStop),
                    );
                    watchdog_turns.push((sid.to_string(), active.turn_id.clone(), active.source));
                    stopped = true;
                    stopped_count = 1;
                }
            } else if body.turn_id.is_none() {
                if let Some(cancel) = cancels.get(sid) {
                    cancel.store(true, Ordering::SeqCst);
                    stopped = true;
                    stopped_count = 1;
                }
            }
        } else {
            for (sid, cancel) in cancels.iter() {
                cancel.store(true, Ordering::SeqCst);
                if let Some(active) = ha_core::chat_engine::active_turn::current(sid) {
                    let _ = ctx.session_db.mark_chat_turn_cancelling(
                        &active.turn_id,
                        session::ChatTurnInterruptReason::UserStop,
                    );
                    ha_core::chat_engine::stream_broadcast::broadcast_turn_status(
                        sid,
                        &active.turn_id,
                        session::ChatTurnStatus::Cancelling,
                        Some(session::ChatTurnInterruptReason::UserStop),
                    );
                    watchdog_turns.push((sid.clone(), active.turn_id.clone(), active.source));
                }
                active_session_ids.push(sid.clone());
                stopped_count += 1;
            }
            stopped = stopped_count > 0;
        }
    }

    let runtime_cancellations = if let Some(sid) = body.session_id.as_deref() {
        if stopped {
            ha_core::runtime_tasks::cancel_runtime_tasks_for_session(Some(sid)).await?
        } else {
            Vec::new()
        }
    } else if active_session_ids.is_empty() {
        ha_core::runtime_tasks::cancel_runtime_tasks_for_session(None).await?
    } else {
        let mut out = Vec::new();
        for sid in active_session_ids {
            out.extend(ha_core::runtime_tasks::cancel_runtime_tasks_for_session(Some(&sid)).await?);
        }
        out
    };

    for (sid, turn_id, source) in watchdog_turns {
        ha_core::chat_engine::spawn_user_stop_watchdog(
            ctx.session_db.clone(),
            sid,
            turn_id,
            source,
        );
    }

    if body.session_id.is_some() {
        return Ok(Json(json!({
            "stopped": stopped,
            "scope": "session",
            "reason": if stopped { Value::Null } else { json!("no matching active chat for session") },
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxModeBody {
    pub mode: SandboxMode,
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

/// `POST /api/chat/sandbox-mode` — set the per-session sandbox mode.
/// Persisted to the `sessions.sandbox_mode` column.
pub async fn set_sandbox_mode(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<SandboxModeBody>,
) -> Result<Json<Value>, AppError> {
    if body.session_id.is_empty() {
        return Err(AppError::bad_request("sessionId required"));
    }
    ctx.session_db
        .update_session_sandbox_mode(&body.session_id, body.mode)?;
    ctx.event_bus.emit(
        "sandbox:mode_changed",
        json!({
            "sessionId": body.session_id,
            "mode": body.mode.as_str(),
        }),
    );
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
    tools::submit_approval_response(
        &request_id,
        approval_response,
        tools::ApprovalResolutionSource::Http,
    )
    .await
    .map_err(|e| AppError::gone(e.to_string()))?;
    Ok(Json(json!({ "approved": true })))
}

/// `GET /api/chat/system-prompt?agent_id=xxx` — return the assembled system prompt.
pub async fn get_system_prompt(
    axum::extract::Query(q): axum::extract::Query<SystemPromptQuery>,
) -> Result<Json<Value>, AppError> {
    let agent_id = q
        .agent_id
        .unwrap_or_else(|| ha_core::agent_loader::DEFAULT_AGENT_ID.to_string());

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
    tools::submit_approval_response(
        &body.request_id,
        approval_response,
        tools::ApprovalResolutionSource::Http,
    )
    .await
    .map_err(|e| AppError::gone(e.to_string()))?;
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
    let agent_id = body
        .agent_id
        .unwrap_or_else(|| ha_core::agent_loader::DEFAULT_AGENT_ID.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_chat_rejects_untrusted_file_path_attachments() {
        let attachments = vec![Attachment {
            name: "secret.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("mention".to_string()),
            data: None,
            file_path: Some("/tmp/secret.txt".to_string()),
            quote_lines: None,
        }];

        assert!(validate_http_chat_attachments("missing-session", &attachments).is_err());
    }

    #[test]
    fn http_chat_allows_uploaded_file_path_attachments() {
        let attachments = vec![Attachment {
            name: "upload.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("upload".to_string()),
            data: None,
            file_path: Some("/tmp/upload.txt".to_string()),
            quote_lines: None,
        }];

        assert!(validate_http_chat_attachments("missing-session", &attachments).is_ok());
    }

    #[test]
    fn http_chat_allows_pasted_text_file_path_attachments() {
        let attachments = vec![Attachment {
            name: "pasted-text.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some(ha_core::attachments::PASTED_TEXT_SOURCE.to_string()),
            data: None,
            file_path: Some("/tmp/pasted-text.txt".to_string()),
            quote_lines: None,
        }];

        assert!(validate_http_chat_attachments("missing-session", &attachments).is_ok());
    }

    #[test]
    fn chat_request_accepts_project_id_camel_case() {
        // The lazy project-session flow sends `projectId` on the first message so
        // the auto-create branch can bind the new session to the project.
        let body = serde_json::json!({
            "message": "hi",
            "projectId": "proj-123",
        });
        let req: ChatRequest = serde_json::from_value(body).expect("deserialize chat request");
        assert_eq!(req.project_id.as_deref(), Some("proj-123"));
        // Omitted project_id defaults to None (plain chats are unaffected).
        let plain: ChatRequest =
            serde_json::from_value(serde_json::json!({ "message": "hi" })).expect("deserialize");
        assert_eq!(plain.project_id, None);
    }
}
