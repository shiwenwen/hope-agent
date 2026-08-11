use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use axum::Json;

use super::helpers::parse_file_upload_to_temp;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tokio::sync::oneshot;

use ha_core::agent::Attachment;
use ha_core::chat_engine::{ChatEngineParams, EventSink, NoopEventSink};
use ha_core::permission::{SandboxMode, SessionMode};
use ha_core::provider::{self, ActiveModel};
use ha_core::session;
use ha_core::tools;
use ha_core::tools::dispatch::ToolDefinitionApiExt;

use crate::error::AppError;
use crate::{AppContext, UiRequestPolicy};

// ── Request / Response Types ───────────────────────────────────
//
// All HTTP request bodies use `#[serde(rename_all = "camelCase")]` because
// the frontend `transport-http.ts::call()` ships args as-is via
// `JSON.stringify(remainingArgs)`. Frontend code uses camelCase keys
// throughout (`sessionId`, `agentId`, `requestId`, ...), so the matching
// HTTP body structs MUST accept camelCase to deserialize successfully.

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialGoalRequest {
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub incoming_turn: Option<ha_core::prompt_context::IncomingTurnWire>,
    #[serde(default)]
    pub ui_surface: Option<ha_core::pet::ChatUiSurface>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// Opaque UI request identity used to target Stop before a lazily-created
    /// session id has reached the frontend.
    #[serde(default)]
    pub client_request_id: Option<String>,
    #[serde(default)]
    pub incognito: Option<bool>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    /// Real-model evaluation attribution. Accepted only by an explicitly
    /// isolated server started with `HA_MODEL_EVAL_MODE=1`.
    #[serde(default)]
    pub eval_context: Option<ha_core::eval_context::EvalRunContext>,
    /// Draft-only defaults consumed when this request creates a Session.
    #[serde(default)]
    pub session_defaults: Option<ha_core::session::SessionDefaultsInput>,
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
    /// Per-session workflow autonomy mode. When provided, the session's
    /// `workflow_mode` column is updated before chat starts.
    #[serde(default)]
    pub workflow_mode: Option<ha_core::workflow_mode::WorkflowMode>,
    #[serde(default)]
    pub temperature_override: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// See Tauri `chat` command — DB stores this while `message` goes to the LLM.
    #[serde(default)]
    pub display_text: Option<String>,
    /// Durable pending-message id. The server replaces message metadata and
    /// attachments from SQLite before starting the turn.
    #[serde(default)]
    pub queued_request_id: Option<String>,
    /// Existing latest user-message id being edited. The server atomically
    /// replaces that settled turn and registers this request's new turn.
    #[serde(default)]
    pub edit_message_id: Option<i64>,
    /// When true, persists the user row with
    /// `attachments_meta = {"plan_trigger": true}` so the UI renders it as a
    /// Plan Mode approve/resume chip (mirrors the Tauri `chat` command).
    #[serde(default)]
    pub is_plan_trigger: Option<bool>,
    /// When true, persists the user row with
    /// `attachments_meta = {"goal_trigger": true}` so the UI renders it as a
    /// normal user bubble with the Goal badge.
    #[serde(default)]
    pub goal_trigger: Option<bool>,
    /// First-turn Goal creation payload. Only honored when the chat request
    /// auto-creates a new session; ignored for existing sessions.
    #[serde(default)]
    pub initial_goal: Option<InitialGoalRequest>,
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
    pub kb_attachments: Vec<ha_knowledge::knowledge::types::KbAttachInput>,
    /// Tool-visibility scope (`"knowledge"`). Set by the knowledge-space sidebar
    /// chat to trim the injected tool set; `None` (default) for normal chats.
    #[serde(default)]
    pub tool_scope: Option<String>,
    /// Knowledge-space sidebar chat: the note open when the conversation started.
    /// Only honored on the auto-create branch — promotes the new session into a
    /// KB chat thread.
    #[serde(default)]
    pub kb_anchor_note: Option<String>,
    /// Design-space per-project chat: the design project open when the
    /// conversation started. Only honored on the auto-create branch (with
    /// `tool_scope == "design"`) — promotes the new session into a design chat
    /// thread anchored to this project.
    #[serde(default)]
    pub design_project_id: Option<String>,
    /// Lazy project binding: when a project draft sends its first message the
    /// client carries the project id here so the auto-create branch materializes
    /// the session inside the project. Ignored when `session_id` is set; mutually
    /// exclusive with incognito (coerced in `create_session_with_project`).
    #[serde(default)]
    pub project_id: Option<String>,
    /// Draft-only project launch configuration. Worktree mode is prepared and
    /// bound before the first model turn starts.
    #[serde(default)]
    pub project_bootstrap: Option<ha_core::project_bootstrap::ProjectSessionBootstrapInput>,
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
    #[serde(default)]
    pub display_text: Option<String>,
    #[serde(default)]
    pub is_plan_trigger: Option<bool>,
    #[serde(default)]
    pub goal_trigger: Option<bool>,
    #[serde(default)]
    pub plan_comment: Option<serde_json::Value>,
    #[serde(default)]
    pub plan_mode: Option<String>,
    #[serde(default)]
    pub workflow_mode: Option<String>,
    #[serde(default)]
    pub incoming_turn: Option<ha_core::prompt_context::IncomingTurnWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateQueuedTurnUserMessageRequest {
    pub session_id: String,
    pub request_id: String,
    pub message: String,
    #[serde(default)]
    pub display_text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnUserMessageRequest {
    pub session_id: String,
    pub turn_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
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
    /// True when a blocked first message caused the freshly auto-created session
    /// (design / knowledge lazy-create) to be deleted before returning. The HTTP
    /// transport reads this to SUPPRESS the synthesized `session_created` event,
    /// so the UI does not switch to a session id that no longer exists (which
    /// would break subsequent sends / history loads).
    #[serde(default, skip_serializing_if = "is_false")]
    pub session_deleted: bool,
    /// The bundled HTTP UI request has transferred execution ownership to the
    /// server and this response is only a dispatch acknowledgement. Public API
    /// and incognito requests keep their synchronous completion contract.
    #[serde(default, skip_serializing_if = "is_false")]
    pub accepted: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone)]
enum UiChatDispatchOutcome {
    Success(ChatResponse),
    Failure(AppError),
}

struct UiChatDispatchEntry {
    fingerprint: String,
    outcome: Option<UiChatDispatchOutcome>,
    finished: bool,
    waiters: Vec<oneshot::Sender<UiChatDispatchOutcome>>,
}

#[derive(Default)]
struct UiChatDispatchState {
    entries: HashMap<String, UiChatDispatchEntry>,
    order: VecDeque<String>,
}

struct UiChatDispatchSubscription {
    leader: bool,
    receiver: oneshot::Receiver<UiChatDispatchOutcome>,
}

const UI_CHAT_DISPATCH_CACHE_MAX: usize = 4096;
static UI_CHAT_DISPATCHES: OnceLock<Mutex<UiChatDispatchState>> = OnceLock::new();

#[derive(Clone)]
struct DetachedUiDispatch {
    request_id: String,
    fingerprint: String,
}

fn ui_chat_dispatches() -> &'static Mutex<UiChatDispatchState> {
    UI_CHAT_DISPATCHES.get_or_init(|| Mutex::new(UiChatDispatchState::default()))
}

fn ui_chat_request_fingerprint(body: &ChatRequest) -> Result<String, AppError> {
    let encoded = serde_json::to_vec(body)
        .map_err(|error| AppError::bad_request(format!("invalid chat request: {error}")))?;
    Ok(ha_core::sha256_hex(&encoded))
}

fn subscribe_ui_chat_dispatch(
    request_id: &str,
    fingerprint: &str,
) -> Result<UiChatDispatchSubscription, AppError> {
    let (sender, receiver) = oneshot::channel();
    let mut state = ui_chat_dispatches()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(entry) = state.entries.get_mut(request_id) {
        if entry.fingerprint != fingerprint {
            return Err(AppError::conflict_with_code(
                "client_request_id_reused",
                "clientRequestId was already used for a different chat request",
            ));
        }
        if let Some(outcome) = entry.outcome.clone() {
            let _ = sender.send(outcome);
        } else {
            entry.waiters.push(sender);
        }
        return Ok(UiChatDispatchSubscription {
            leader: false,
            receiver,
        });
    }

    while state.entries.len() >= UI_CHAT_DISPATCH_CACHE_MAX {
        let attempts = state.order.len();
        let mut evicted = false;
        for _ in 0..attempts {
            let Some(candidate) = state.order.pop_front() else {
                break;
            };
            if state
                .entries
                .get(&candidate)
                .is_some_and(|entry| entry.finished)
            {
                state.entries.remove(&candidate);
                evicted = true;
                break;
            }
            state.order.push_back(candidate);
        }
        if !evicted {
            return Err(AppError::internal(
                "too many UI chat requests are still being prepared",
            ));
        }
    }

    state.order.push_back(request_id.to_string());
    state.entries.insert(
        request_id.to_string(),
        UiChatDispatchEntry {
            fingerprint: fingerprint.to_string(),
            outcome: None,
            finished: false,
            waiters: vec![sender],
        },
    );
    Ok(UiChatDispatchSubscription {
        leader: true,
        receiver,
    })
}

/// Publish exactly one transport outcome. The accepted ACK remains cached so
/// retrying a POST whose response was lost cannot create a duplicate turn.
fn publish_ui_chat_dispatch(request_id: &str, outcome: UiChatDispatchOutcome) -> bool {
    let mut state = ui_chat_dispatches()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = state.entries.get_mut(request_id) else {
        return false;
    };
    if entry.outcome.is_some() {
        return false;
    }
    entry.finished = match &outcome {
        UiChatDispatchOutcome::Success(response) => !response.accepted,
        UiChatDispatchOutcome::Failure(_) => true,
    };
    entry.outcome = Some(outcome.clone());
    for waiter in entry.waiters.drain(..) {
        let _ = waiter.send(outcome.clone());
    }
    true
}

fn mark_ui_chat_dispatch_finished(request_id: &str) {
    let mut state = ui_chat_dispatches()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entry) = state.entries.get_mut(request_id) {
        entry.finished = true;
    }
}

struct ChatCancelRegistrationGuard {
    registry: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    session_id: String,
    cancel: Arc<AtomicBool>,
    armed: bool,
}

impl ChatCancelRegistrationGuard {
    fn new(
        registry: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
        session_id: String,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            registry,
            session_id,
            cancel,
            armed: true,
        }
    }

    fn release(&mut self) {
        if !self.armed {
            return;
        }
        let mut cancels = self
            .registry
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let should_remove = cancels
            .get(&self.session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.cancel));
        if should_remove {
            cancels.remove(&self.session_id);
        }
        self.armed = false;
    }
}

impl Drop for ChatCancelRegistrationGuard {
    fn drop(&mut self) {
        self.release();
    }
}

struct HttpChatTurnDropFinalizer {
    db: Arc<session::SessionDB>,
    session_id: String,
    turn_id: String,
    armed: bool,
}

impl HttpChatTurnDropFinalizer {
    fn new(db: Arc<session::SessionDB>, session_id: String, turn_id: String) -> Self {
        Self {
            db,
            session_id,
            turn_id,
            armed: true,
        }
    }

    fn finish_if_open(
        &mut self,
        status: session::ChatTurnStatus,
        interrupt_reason: Option<session::ChatTurnInterruptReason>,
        error: Option<&str>,
    ) {
        if !self.armed {
            return;
        }
        self.armed = false;

        // Once the unified journal run exists, its atomic convergence path is
        // the only authority allowed to terminalize the turn or emit
        // `chat:stream_end`. This includes a synchronous public/incognito
        // request disconnect and a detached worker panic: the engine lifecycle
        // schedules runtime-cancel recovery from the durable prefix, while this
        // outer HTTP guard merely records `cancelling`.
        if self
            .db
            .latest_stream_run(&self.session_id)
            .ok()
            .flatten()
            .is_some_and(|run| {
                run.status == "running" && run.turn_id.as_deref() == Some(self.turn_id.as_str())
            })
        {
            if status == session::ChatTurnStatus::Interrupted
                && interrupt_reason == Some(session::ChatTurnInterruptReason::RuntimeCancel)
            {
                let _ = self.db.mark_chat_turn_cancelling(
                    &self.turn_id,
                    session::ChatTurnInterruptReason::RuntimeCancel,
                );
            }
            return;
        }

        let turn = match self.db.get_chat_turn(&self.turn_id) {
            Ok(Some(turn)) if !turn.status.is_terminal() => turn,
            _ => return,
        };
        let stream_id = turn.stream_id.clone();
        match self
            .db
            .finish_chat_turn_once(&self.turn_id, status, interrupt_reason, error, None)
        {
            Ok(true) => {
                ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
                    &self.session_id,
                    stream_id.as_deref(),
                    Some(&self.turn_id),
                    Some(status),
                    interrupt_reason,
                    error,
                );
                ha_core::chat_engine::active_turn::force_release(&self.session_id, &self.turn_id);
            }
            Ok(false) => {}
            Err(err) => {
                ha_core::app_warn!(
                    "chat",
                    "http_turn_finalizer",
                    "failed to finalize dropped HTTP chat turn {}: {}",
                    self.turn_id,
                    err
                );
            }
        }
    }
}

impl Drop for HttpChatTurnDropFinalizer {
    fn drop(&mut self) {
        self.finish_if_open(
            session::ChatTurnStatus::Interrupted,
            Some(session::ChatTurnInterruptReason::RuntimeCancel),
            Some("chat execution owner dropped before completion"),
        );
    }
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

fn parse_plan_mention_target(target_id: &str) -> Option<(&str, u32)> {
    let (short_id, version) = target_id.split_once(":v")?;
    let version = version.parse::<u32>().ok()?;
    Some((short_id, version))
}

fn validate_http_plan_mention_attachment_with<F>(
    file_path: &str,
    incoming_turn: Option<&ha_core::prompt_context::IncomingTurnWire>,
    resolve_plan_path: &F,
) -> Result<(), AppError>
where
    F: Fn(&str, u32) -> Option<PathBuf>,
{
    let requested = PathBuf::from(file_path);
    if !requested.is_absolute() {
        return Err(AppError::bad_request(
            "plan mention attachment path must be absolute",
        ));
    }
    let requested = requested.canonicalize().map_err(|_| {
        AppError::forbidden("plan mention attachment does not match a registered plan")
    })?;

    let matches_registered_plan = incoming_turn
        .into_iter()
        .flat_map(|wire| wire.mentions.iter())
        .filter(|mention| mention.kind == ha_core::prompt_context::MentionKind::Plan)
        .filter_map(|mention| parse_plan_mention_target(&mention.target_id))
        .filter_map(|(short_id, version)| resolve_plan_path(short_id, version))
        .filter_map(|path| path.canonicalize().ok())
        .any(|registered| registered == requested);
    if matches_registered_plan {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "plan mention attachment does not match a registered plan",
        ))
    }
}

fn validate_http_chat_attachments_with_plan_resolver<F>(
    session_id: &str,
    message: &str,
    incoming_turn: Option<&ha_core::prompt_context::IncomingTurnWire>,
    attachments: &[Attachment],
    resolve_plan_path: &F,
) -> Result<(), AppError>
where
    F: Fn(&str, u32) -> Option<PathBuf>,
{
    ha_core::attachments::validate_typed_resource_attachment_bindings(
        message,
        incoming_turn,
        attachments,
    )
    .map_err(|error| {
        AppError::bad_request(format!(
            "invalid typed resource attachment binding: {error}"
        ))
    })?;

    validate_http_chat_attachments_inner(session_id, incoming_turn, attachments, resolve_plan_path)
}

fn validate_http_chat_attachments(
    session_id: &str,
    message: &str,
    incoming_turn: Option<&ha_core::prompt_context::IncomingTurnWire>,
    attachments: &[Attachment],
) -> Result<(), AppError> {
    validate_http_chat_attachments_with_plan_resolver(
        session_id,
        message,
        incoming_turn,
        attachments,
        &|short_id, version| {
            ha_core::plan::resolve_plan_mention(short_id, version)
                .ok()
                .map(|resolution| PathBuf::from(resolution.file_path))
        },
    )
}

fn validate_http_chat_attachments_inner<F>(
    session_id: &str,
    incoming_turn: Option<&ha_core::prompt_context::IncomingTurnWire>,
    attachments: &[Attachment],
    resolve_plan_path: &F,
) -> Result<(), AppError>
where
    F: Fn(&str, u32) -> Option<PathBuf>,
{
    if attachments.len() > ha_core::attachments::MAX_CHAT_ATTACHMENTS {
        return Err(AppError::bad_request(format!(
            "a message can contain at most {} attachments",
            ha_core::attachments::MAX_CHAT_ATTACHMENTS
        )));
    }
    for att in attachments {
        if att.upload_id.is_some() {
            if att.data.is_some() || att.file_path.is_some() {
                return Err(AppError::bad_request(
                    "uploadId is mutually exclusive with data and filePath",
                ));
            }
            if !matches!(
                att.source.as_deref(),
                Some("upload") | Some(ha_core::attachments::PASTED_TEXT_SOURCE)
            ) {
                return Err(AppError::bad_request(
                    "uploadId is only valid for uploaded attachments",
                ));
            }
            continue;
        }
        // "quote" attachments carry the snippet in `data`; their `file_path` is
        // only a reference label (never read from disk), so it's safe.
        match (att.source.as_deref(), att.file_path.as_deref()) {
            (_, None) => {}
            (Some("quote"), Some(_)) => {}
            (Some("upload"), Some(path)) => {
                validate_http_uploaded_attachment_path(session_id, path)?
            }
            (Some(source), Some(path)) if source == ha_core::attachments::PASTED_TEXT_SOURCE => {
                validate_http_uploaded_attachment_path(session_id, path)?
            }
            (Some("mention"), Some(path)) => validate_http_mention_attachment(session_id, path)?,
            (Some("plan_mention"), Some(path)) => {
                validate_http_plan_mention_attachment_with(path, incoming_turn, resolve_plan_path)?
            }
            _ => {
                return Err(AppError::bad_request(
                    "HTTP chat attachments must be staged through /api/chat/attachment-stage",
                ));
            }
        }
    }
    Ok(())
}

fn validate_http_uploaded_attachment_path(session_id: &str, path: &str) -> Result<(), AppError> {
    let requested = std::path::Path::new(path);
    if !requested.is_absolute() {
        return Err(AppError::forbidden("invalid uploaded attachment path"));
    }
    let canonical = requested
        .canonicalize()
        .map_err(|_| AppError::forbidden("invalid uploaded attachment path"))?;
    let root = ha_core::paths::root_dir()
        .map_err(|error| AppError::internal(error.to_string()))?
        .join("attachments");
    let temp = root.join(ha_core::attachments::TEMP_SESSION_ID);
    let session = root.join(session_id);
    let allowed = [temp, session].into_iter().any(|dir| {
        dir.canonicalize()
            .map(|canonical_dir| canonical.starts_with(canonical_dir))
            .unwrap_or(false)
    });
    if !allowed {
        return Err(AppError::forbidden("invalid uploaded attachment path"));
    }
    let size = std::fs::metadata(&canonical)
        .map_err(|_| AppError::forbidden("invalid uploaded attachment path"))?
        .len();
    if size > ha_core::attachments::max_chat_attachment_bytes() as u64 {
        return Err(AppError::bad_request(format!(
            "attachment exceeds the configured {} MB limit",
            ha_core::attachments::max_chat_attachment_mb()
        )));
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StopChatRequest {
    /// When omitted, cancels every running chat. First-party callers normally
    /// provide a session id; the global form is a legacy/emergency fallback.
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Opaque first-party request id used while a lazy session has not yet
    /// been announced to the client.
    #[serde(default)]
    pub client_request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryControlRequest {
    pub session_id: String,
    pub recovery_id: String,
    pub action: ha_core::recovery_control::RecoveryAction,
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
pub struct CapabilityMentionQuery {
    #[serde(default)]
    pub agent_id: Option<String>,
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

/// Public HTTP chat API. Product UI routing metadata is deliberately ignored:
/// callers that are not the bundled message-list/composer must never create a
/// Pet activity merely by sending a `uiSurface` field.
pub async fn chat(
    State(ctx): State<Arc<AppContext>>,
    Json(mut body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    let foreground_admission = ha_core::chat_engine::active_turn::begin_foreground_request();
    body.ui_surface = None;
    chat_inner(ctx, body, None, foreground_admission).await
}

/// Bundled first-party HTTP UI chat. Every `HttpTransport.startChat` call uses
/// this browser-provenance-gated route; `uiSurface`, when present, additionally
/// identifies a product message-list/composer surface for Pet projection.
pub async fn ui_chat(
    State(ctx): State<Arc<AppContext>>,
    Extension(ui_request_policy): Extension<UiRequestPolicy>,
    headers: HeaderMap,
    Json(mut body): Json<ChatRequest>,
) -> Result<Response, AppError> {
    if !ui_request_policy.accepts(&headers) {
        return Err(AppError::forbidden("ui_chat_browser_proof_required"));
    }
    // Capture at the HTTP transport boundary, before the incognito lookup or
    // detached-task handoff. A global Stop racing either await must reject this
    // already-arrived request even after the cleanup gate is released.
    let foreground_admission = ha_core::chat_engine::active_turn::begin_foreground_request();

    // Incognito is deliberately close-and-burn: it must never outlive the
    // request/web surface that owns it. Existing sessions do not resend the
    // draft `incognito` flag, so resolve the durable session metadata before
    // deciding whether execution may detach from this request.
    let project_bound = body
        .project_id
        .as_deref()
        .is_some_and(|project_id| !project_id.trim().is_empty());
    let incognito = if let Some(session_id) = body
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        let session_id = session_id.to_string();
        ctx.session_db
            .run(move |db| db.get_session(&session_id))
            .await?
            .is_some_and(|session| session.incognito)
    } else {
        body.incognito.unwrap_or(false) && !project_bound
    };
    if incognito {
        return chat_inner(ctx, body, None, foreground_admission)
            .await
            .map(|response| response.into_response());
    }

    let request_id = body
        .client_request_id
        .as_deref()
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if request_id.len() > 256 {
        return Err(AppError::bad_request(
            "clientRequestId must be at most 256 characters",
        ));
    }
    body.client_request_id = Some(request_id.clone());
    let fingerprint = ui_chat_request_fingerprint(&body)?;

    // SQLite is the cross-process idempotency authority. The in-memory entry
    // below only coordinates concurrent waiters while the first request is
    // still preparing its durable user-message/turn boundary.
    let durable_request_id = request_id.clone();
    if let Some(dispatch) = ctx
        .session_db
        .run(move |db| db.get_ui_chat_dispatch(&durable_request_id))
        .await?
    {
        if dispatch.request_fingerprint != fingerprint {
            return Err(AppError::conflict_with_code(
                "client_request_id_reused",
                "clientRequestId was already used for a different chat request",
            ));
        }
        return Ok((
            StatusCode::ACCEPTED,
            Json(ChatResponse {
                session_id: dispatch.session_id,
                response: String::new(),
                turn_id: dispatch.turn_id,
                blocked_reason: None,
                session_deleted: false,
                accepted: true,
            }),
        )
            .into_response());
    }

    let subscription = subscribe_ui_chat_dispatch(&request_id, &fingerprint)?;

    if subscription.leader {
        let worker_request_id = request_id.clone();
        let detached_dispatch = DetachedUiDispatch {
            request_id: worker_request_id.clone(),
            fingerprint,
        };
        tokio::spawn(async move {
            let result = AssertUnwindSafe(chat_inner(
                ctx,
                body,
                Some(detached_dispatch),
                foreground_admission,
            ))
            .catch_unwind()
            .await;
            match result {
                Ok(result) => {
                    let outcome = match result {
                        Ok(Json(response)) => UiChatDispatchOutcome::Success(response),
                        Err(error) => UiChatDispatchOutcome::Failure(error),
                    };
                    if !publish_ui_chat_dispatch(&worker_request_id, outcome.clone()) {
                        mark_ui_chat_dispatch_finished(&worker_request_id);
                        if let UiChatDispatchOutcome::Failure(error) = outcome {
                            ha_core::app_warn!(
                                "chat",
                                "detached_ui_chat_failed",
                                "Detached HTTP UI chat failed after dispatch: request={} error={}",
                                worker_request_id,
                                error.message
                            );
                        }
                    }
                }
                Err(_) => {
                    ha_core::app_error!(
                        "chat",
                        "detached_ui_chat_panicked",
                        "Detached HTTP UI chat panicked: request={}",
                        worker_request_id
                    );
                    let _ = publish_ui_chat_dispatch(
                        &worker_request_id,
                        UiChatDispatchOutcome::Failure(AppError::internal(
                            "detached UI chat worker panicked",
                        )),
                    );
                    mark_ui_chat_dispatch_finished(&worker_request_id);
                }
            }
        });
    }

    match subscription.receiver.await {
        Ok(UiChatDispatchOutcome::Success(response)) => {
            let status = if response.accepted {
                StatusCode::ACCEPTED
            } else {
                StatusCode::OK
            };
            Ok((status, Json(response)).into_response())
        }
        Ok(UiChatDispatchOutcome::Failure(error)) => Err(error),
        Err(_) => Err(AppError::internal(
            "UI chat dispatch closed before acknowledgement",
        )),
    }
}

async fn chat_inner(
    ctx: Arc<AppContext>,
    mut body: ChatRequest,
    detached_ui_dispatch: Option<DetachedUiDispatch>,
    foreground_admission: ha_core::chat_engine::active_turn::ForegroundRequestAdmission,
) -> Result<Json<ChatResponse>, AppError> {
    let db = ctx.session_db.clone();
    let eval_context_pending = body.eval_context.take();
    if eval_context_pending.is_some() && !ha_core::eval_context::model_eval_mode_enabled() {
        return Err(AppError::forbidden(
            "evalContext is accepted only when HA_MODEL_EVAL_MODE=1",
        ));
    }

    // Per-session mode fields are consumed below after we resolve the
    // session id (we need a session_id to persist).
    let permission_mode_pending = body.permission_mode;
    let sandbox_mode_pending = body.sandbox_mode;
    let mut workflow_mode_pending = body.workflow_mode;

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
    let bootstrap_request_id = body
        .project_bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.request_id.clone());
    let project_bootstrap_request_guard = match (
        body.client_request_id.as_deref(),
        bootstrap_request_id.as_deref(),
    ) {
        (Some(client_request_id), Some(bootstrap_request_id)) => Some(
            ha_core::project_bootstrap::register_project_bootstrap_client_request(
                client_request_id,
                bootstrap_request_id,
            ),
        ),
        _ => None,
    };
    let auto_create_session = existing_session_id.is_none();
    if body.edit_message_id.is_some() && auto_create_session {
        return Err(AppError::bad_request(
            "editMessageId requires an existing session",
        ));
    }
    if body.edit_message_id.is_some() && body.queued_request_id.is_some() {
        return Err(AppError::bad_request(
            "editMessageId cannot be combined with queuedRequestId",
        ));
    }
    if !auto_create_session && body.project_bootstrap.is_some() {
        return Err(AppError::bad_request(
            "projectBootstrap is only valid when creating a new project session",
        ));
    }
    if body.project_bootstrap.is_some() && project_id.is_none() {
        return Err(AppError::bad_request("projectBootstrap requires projectId"));
    }
    if body.project_bootstrap.is_some() {
        // Project bootstrap can switch the local checkout or create a managed
        // worktree, so HTTP must enforce the same default-deny write policy as
        // the session-scoped Git mutation routes before creating a Session.
        super::git_control::ensure_writes_allowed()?;
    }
    if let Some(bootstrap) = body.project_bootstrap.as_ref() {
        if bootstrap
            .base_ref
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(AppError::bad_request("project launch requires baseRef"));
        }
    }
    if auto_create_session
        && body
            .initial_goal
            .as_ref()
            .is_some_and(|goal| goal.objective.trim().is_empty())
    {
        return Err(AppError::bad_request(
            "Initial goal objective must not be empty",
        ));
    }
    if auto_create_session
        && body.initial_goal.is_some()
        && body.incognito.unwrap_or(false)
        && project_id.is_none()
    {
        return Err(AppError::bad_request(
            "Cannot create a durable goal for an incognito session",
        ));
    }
    let agent_id = if let Some(id) = explicit_agent_id {
        id
    } else if let Some(session_id) = existing_session_id {
        let session_id = session_id.to_string();
        db.run(move |db| db.get_session(&session_id))
            .await?
            .map(|session| session.agent_id)
            .unwrap_or_else(|| ha_core::agent::resolver::resolve_default_agent_id(None, None))
    } else {
        // New session: resolve via the project's default-agent chain when a
        // lazy project binding is present, mirroring the create_session route.
        let project = match project_id.clone() {
            Some(pid) => {
                let project_db = ctx.project_db.clone();
                ha_core::blocking::run_blocking(move || project_db.get(&pid).ok().flatten()).await
            }
            None => None,
        };
        ha_core::agent::resolver::resolve_default_agent_id(project.as_ref(), None)
    };
    // Acquire before creating or mutating session state. The engine keeps its
    // own admission backstop, while this outer guard closes the shell-side
    // check/create race with Agent deletion.
    let _agent_admission = ha_core::agent_lifecycle::begin_agent_run(&agent_id)
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    // Resolve or create session
    let mut new_session_created = false;
    let sid = match body.session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            // `project_id` binds the new session to a project on this lazy-create
            // branch; incognito is coerced off when a project is set.
            let meta = {
                let agent_id = agent_id.clone();
                let project_id = project_id.clone();
                let incognito = body.incognito;
                db.run(move |db| {
                    db.create_session_with_project(&agent_id, project_id.as_deref(), incognito)
                })
                .await?
            };
            new_session_created = true;
            meta.id
        }
    };
    let _eval_session_guard = eval_context_pending
        .map(|context| {
            ha_core::eval_context::register_http_turn_session(&sid, context)
                .map_err(|error| AppError::bad_request(error.to_string()))
        })
        .transpose()?;

    // Apply draft working dir picked before the session existed. Mirrors the
    // Tauri `chat` command — explicit-session callers must use the dedicated
    // setter to change it.
    if new_session_created {
        if let Some(wd) = body.working_dir.as_ref().filter(|s| !s.trim().is_empty()) {
            let sid = sid.clone();
            let wd = wd.clone();
            db.run(move |db| db.update_session_working_dir(&sid, Some(wd)))
                .await
                .map_err(|e| AppError::bad_request(e.to_string()))?;
        }
        ha_knowledge::knowledge::service::apply_draft_attachments(
            &sid,
            body.incognito.unwrap_or(false),
            &body.kb_attachments,
        );
    }

    // Persist per-session permission mode if the body included one.
    if permission_mode_pending.is_some() || sandbox_mode_pending.is_some() {
        let sid = sid.clone();
        db.run(move |db| -> anyhow::Result<()> {
            if let Some(mode) = permission_mode_pending {
                db.update_session_permission_mode(&sid, mode)?;
            }
            if let Some(mode) = sandbox_mode_pending {
                db.update_session_sandbox_mode(&sid, mode)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    }
    // Load app/agent config before resolving per-turn settings.
    let store = ha_core::config::cached_config();
    let agent_def = ha_core::agent_loader::load_agent(&agent_id).ok();

    let requested_effort = body
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .map(str::to_string);
    if new_session_created {
        let sid_for_defaults = sid.clone();
        let defaults = body.session_defaults.clone().unwrap_or_default();
        let model_for_defaults = defaults.model;
        let effort_for_defaults = defaults.reasoning_effort;
        let temperature_for_defaults = defaults.temperature;
        let apply_defaults = db
            .run(move |session_db| -> anyhow::Result<()> {
                if temperature_for_defaults.is_some_and(|value| !(0.0..=2.0).contains(&value)) {
                    anyhow::bail!("Temperature must be between 0.0 and 2.0");
                }
                if effort_for_defaults
                    .as_deref()
                    .is_some_and(|effort| !ha_core::agent::is_valid_reasoning_effort(effort))
                {
                    anyhow::bail!("Invalid reasoning effort in session defaults");
                }
                if let Some(reference) = model_for_defaults.as_deref() {
                    let model = provider::parse_model_ref(reference)
                        .ok_or_else(|| anyhow::anyhow!("Invalid model reference: {reference}"))?;
                    let config = ha_core::config::cached_config();
                    if !provider::model_ref_exists(&config.providers, &model) {
                        anyhow::bail!("Selected model no longer exists: {reference}");
                    }
                    let provider_name = config
                        .providers
                        .iter()
                        .find(|candidate| candidate.id == model.provider_id)
                        .map(|candidate| candidate.name.as_str());
                    session_db.update_session_model(
                        &sid_for_defaults,
                        Some(&model.provider_id),
                        provider_name,
                        Some(&model.model_id),
                    )?;
                }
                if let Some(temperature) = temperature_for_defaults {
                    session_db.update_session_temperature(&sid_for_defaults, Some(temperature))?;
                }
                if let Some(effort) = effort_for_defaults.as_deref() {
                    session_db.update_session_reasoning_effort(&sid_for_defaults, Some(effort))?;
                }
                Ok(())
            })
            .await;
        if let Err(error) = apply_defaults {
            // Working-dir and KB draft bindings may already have been applied
            // on the HTTP path, so delete the complete newly-created Session
            // and its cascaded side effects before returning the validation
            // error.
            let sid_for_cleanup = sid.clone();
            let _ = db
                .run(move |session_db| session_db.delete_session(&sid_for_cleanup))
                .await;
            return Err(error.into());
        }

        if let Some(bootstrap) = body.project_bootstrap.as_ref() {
            let pid = project_id
                .as_deref()
                .ok_or_else(|| AppError::bad_request("projectBootstrap requires projectId"))?;
            let project = {
                let project_db = ctx.project_db.clone();
                let pid = pid.to_string();
                ha_core::blocking::run_blocking(move || project_db.get(&pid)).await?
            }
            .ok_or_else(|| AppError::bad_request(format!("Project not found: {pid}")))?;
            if project.archived {
                let sid_for_cleanup = sid.clone();
                let _ = db
                    .run(move |session_db| session_db.delete_session(&sid_for_cleanup))
                    .await;
                return Err(AppError::bad_request(
                    "Cannot start a task in an archived project",
                ));
            }
            let source_working_dir = {
                let sid = sid.clone();
                db.run(move |session_db| -> anyhow::Result<String> {
                    let meta = session_db
                        .get_session(&sid)?
                        .ok_or_else(|| anyhow::anyhow!("session not found: {sid}"))?;
                    ha_core::session::effective_working_dir_for_meta(&meta)
                        .ok_or_else(|| anyhow::anyhow!("project session has no working directory"))
                })
                .await?
            };
            if let Err(error) = ha_core::project_bootstrap::bootstrap_project_session(
                &db,
                ha_core::project_bootstrap::PrepareProjectWorktreeInput {
                    request: bootstrap.clone(),
                    session_id: sid.clone(),
                    project_id: pid.to_string(),
                    source_working_dir,
                },
            )
            .await
            {
                let sid_for_cleanup = sid.clone();
                let _ = db
                    .run(move |session_db| session_db.delete_session(&sid_for_cleanup))
                    .await;
                return Err(AppError::bad_request(format!(
                    "project bootstrap failed: {error:#}"
                )));
            }
        }
    }
    // Bootstrap is no longer running beyond this point. Remove the temporary
    // client mapping so a later Stop for the model turn does not create a
    // stale bootstrap-id cancellation latch.
    drop(project_bootstrap_request_guard);
    let runtime_defaults = {
        let sid = sid.clone();
        db.run(move |db| ha_core::session::ensure_session_runtime_defaults(db, &sid))
            .await?
    };
    let effort = requested_effort.unwrap_or_else(|| runtime_defaults.reasoning_effort.clone());
    if !ha_core::agent::is_valid_reasoning_effort(&effort) {
        return Err(AppError::bad_request(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort,
            ha_core::agent::VALID_REASONING_EFFORTS
        )));
    }
    if let Some(request_id) = bootstrap_request_id.as_deref() {
        let request_id_owned = request_id.to_string();
        let claimed = db
            .run(move |db| db.claim_project_bootstrap_chatting(&request_id_owned))
            .await?;
        if !claimed {
            return Err(AppError::conflict_with_code(
                "project_bootstrap_already_claimed",
                "project bootstrap was already claimed by another chat request",
            ));
        }
    }

    let turn_id = uuid::Uuid::new_v4().to_string();
    let queued_request_id = body
        .queued_request_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    if let Some(request_id) = queued_request_id.as_ref() {
        let sid_for_claim = sid.clone();
        let request_id_for_claim = request_id.clone();
        let turn_for_claim = turn_id.clone();
        let claimed = db
            .run(move |db| {
                db.claim_queued_turn_message_for_dispatch(
                    &sid_for_claim,
                    &request_id_for_claim,
                    &turn_for_claim,
                )
            })
            .await?
            .ok_or_else(|| {
                AppError::conflict_with_code(
                    "queued_message_unavailable",
                    "Queued message is no longer available",
                )
            })?;
        body.message = claimed.message;
        body.attachments = claimed.attachments;
        body.display_text = claimed.display_text;
        body.is_plan_trigger = Some(claimed.is_plan_trigger);
        body.goal_trigger = Some(claimed.goal_trigger);
        body.plan_comment = claimed.plan_comment;
        body.incoming_turn = claimed.incoming_turn;
        workflow_mode_pending = claimed
            .workflow_mode
            .as_deref()
            .and_then(ha_core::workflow_mode::WorkflowMode::from_str);
    }
    if let Some(mode) = workflow_mode_pending {
        db.update_session_workflow_mode(&sid, mode)
            .map_err(|e| AppError::bad_request(e.to_string()))?;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let _active_turn_guard = match ha_core::chat_engine::active_turn::try_acquire_foreground_request(
        foreground_admission,
        &sid,
        ha_core::chat_engine::stream_seq::ChatSource::Http,
        turn_id.clone(),
        body.client_request_id.clone(),
        cancel.clone(),
    ) {
        Ok(guard) => guard,
        Err(error) => {
            let error_code = if error.cancelled_by_global_stop() {
                ha_core::agent::preflight::CHAT_CANCELLED_DURING_PREFLIGHT_CODE
            } else {
                ha_core::chat_engine::stream_seq::ACTIVE_STREAM_ERROR_CODE
            };
            // Session creation and bootstrap claiming precede active-turn
            // admission. Hold a session cleanup gate before awaiting any
            // queue release so a global Stop rejection cannot leak this
            // unpublished lazy Session or its managed worktree.
            let cleanup = ha_core::chat_engine::stop::PreTurnCancelCleanup::begin(
                db.clone(),
                sid.clone(),
                bootstrap_request_id.clone(),
                new_session_created,
                queued_request_id
                    .as_ref()
                    .map(|request_id| (request_id.clone(), turn_id.clone())),
            );
            if let Some(cleanup) = cleanup {
                cleanup.spawn();
            }
            return Err(AppError::conflict_with_code(error_code, error.to_string()));
        }
    };

    // Prefer display_text for DB/title, fall back to the LLM-bound message.
    let raw_prompt = ha_core::non_empty_trim_or(body.display_text.as_deref(), &body.message);

    // Preflight chokepoint: every user-message entry point routes through this
    // before persisting. Pass-through in Phase 0.1; PR 1.2 runs the
    // `UserPromptSubmit` hook here (may block / rewrite the prompt).
    let preflight = ha_core::agent::preflight::user_prompt_preflight_cancellable(
        ha_core::agent::preflight::PreflightArgs {
            session_id: &sid,
            agent_id: Some(agent_id.as_str()),
            raw_prompt,
            turn_id: &turn_id,
        },
        cancel.as_ref(),
    )
    .await;
    let Some(preflight) = preflight else {
        // There is no chat_turn row yet, so terminate the transport-visible
        // lifecycle and release the exact guard before Git/SQLite cleanup. The
        // cleanup gate keeps a replacement turn out until the exact queued-row
        // CAS settles or reaches its bounded timeout.
        let cleanup = ha_core::chat_engine::stop::PreTurnCancelCleanup::begin(
            db.clone(),
            sid.clone(),
            bootstrap_request_id.clone(),
            new_session_created,
            queued_request_id
                .as_ref()
                .map(|request_id| (request_id.clone(), turn_id.clone())),
        );
        ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
            &sid,
            None,
            Some(&turn_id),
            Some(session::ChatTurnStatus::Interrupted),
            Some(session::ChatTurnInterruptReason::UserStop),
            None,
        );
        ha_core::chat_engine::active_turn::force_release(&sid, &turn_id);

        if let Some(cleanup) = cleanup {
            cleanup.spawn();
        }
        return Err(AppError::conflict_with_code(
            ha_core::agent::preflight::CHAT_CANCELLED_DURING_PREFLIGHT_CODE,
            "chat stopped before prompt submission completed",
        ));
    };
    let effective_prompt = match preflight {
        ha_core::agent::preflight::PreflightOutcome::Proceed { effective_prompt } => {
            effective_prompt
        }
        ha_core::agent::preflight::PreflightOutcome::Block { reason } => {
            if body.edit_message_id.is_some() {
                return Err(AppError::bad_request(format!(
                    "Message edit was blocked: {reason}"
                )));
            }
            if let Some(request_id) = queued_request_id.as_ref() {
                let sid_for_remove = sid.clone();
                let request_id_for_remove = request_id.clone();
                let _ = db
                    .run(move |db| {
                        db.remove_claimed_turn_message(&sid_for_remove, &request_id_for_remove)
                    })
                    .await;
            }
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
            if new_session_created
                && matches!(
                    body.tool_scope.as_deref(),
                    Some("knowledge") | Some("design")
                )
            {
                let _ = {
                    let sid = sid.clone();
                    db.run(move |db| db.delete_session(&sid)).await
                };
                return Ok(Json(ChatResponse {
                    session_id: sid,
                    response: notice.clone(),
                    turn_id,
                    blocked_reason: Some(notice),
                    // Session was just deleted — tell the transport not to adopt it.
                    session_deleted: true,
                    accepted: false,
                }));
            }
            let _ = {
                let sid = sid.clone();
                let notice = notice.clone();
                db.run(move |db| db.append_message(&sid, &session::NewMessage::event(&notice)))
                    .await
            };
            if let Some(request_id) = bootstrap_request_id.as_deref() {
                let request_id = request_id.to_string();
                let _ = db
                    .run(move |db| db.mark_project_bootstrap_completed(&request_id))
                    .await;
            }
            return Ok(Json(ChatResponse {
                session_id: sid,
                response: notice.clone(),
                turn_id,
                blocked_reason: Some(notice),
                session_deleted: false,
                accepted: false,
            }));
        }
    };

    if new_session_created {
        if let Some(goal) = body.initial_goal.as_ref() {
            db.create_goal(ha_core::goal::CreateGoalInput {
                session_id: sid.clone(),
                objective: goal.objective.trim().to_string(),
                completion_criteria: goal
                    .completion_criteria
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })?;
        }
    }

    // KB sidebar chat: promote the freshly-created session into a knowledge
    // thread now that preflight has passed (mirrors the Tauri command). Doing it
    // in the auto-create block left a hidden `kind=Knowledge` zombie + thread row
    // when a UserPromptSubmit hook blocked the first message.
    if new_session_created && body.tool_scope.as_deref() == Some("knowledge") {
        if let Some(kb_id) = body.kb_attachments.first().map(|a| a.kb_id.clone()) {
            ha_knowledge::knowledge::service::mark_session_as_kb_thread(
                &sid,
                &kb_id,
                body.kb_anchor_note.as_deref(),
            );
        }
    }

    // Design-space per-project chat: promote the freshly-created session into a
    // design thread anchored to the open project (mirrors the KB branch above).
    if new_session_created && body.tool_scope.as_deref() == Some("design") {
        if let Some(project_id) = body.design_project_id.as_deref() {
            ha_design::design::service::mark_session_as_design_thread(&sid, project_id);
        }
    }

    // Attachments: validate + persist AFTER the preflight, so a blocked prompt
    // returns above before any attachment IO touches disk. The DB content is the
    // hook-rewritten `effective_prompt`, so the separate `persisted_content`
    // main computed (identical to `raw_prompt`, now consumed by the preflight) is
    // dropped.
    if let Err(error) = validate_http_chat_attachments(
        &sid,
        &body.message,
        body.incoming_turn.as_ref(),
        &body.attachments,
    ) {
        if let Some(request_id) = queued_request_id.as_ref() {
            let sid_for_release = sid.clone();
            let request_id_for_release = request_id.clone();
            let turn_for_release = turn_id.clone();
            let _ = db
                .run(move |db| {
                    db.release_queued_turn_message_dispatch(
                        &sid_for_release,
                        &request_id_for_release,
                        &turn_for_release,
                    )
                })
                .await;
        }
        return Err(error);
    }
    // Attachment persistence writes files to disk — offload so a stalled
    // filesystem can't pin the async worker (mirrors the desktop chat path).
    // `persist_*` mutates the attachments in place, so hand them into the
    // blocking closure and take them back out.
    let attachments_meta = {
        let sid_for_files = sid.clone();
        let mut moved = std::mem::take(&mut body.attachments);
        let persisted_result = ha_core::blocking::run_blocking(move || {
            let meta = ha_core::attachments::persist_chat_user_attachments_meta(
                &sid_for_files,
                &mut moved,
            )?;
            anyhow::Ok((meta, moved))
        })
        .await;
        let (meta, persisted) = match persisted_result {
            Ok(value) => value,
            Err(error) => {
                if let Some(request_id) = queued_request_id.as_ref() {
                    let sid_for_release = sid.clone();
                    let request_id_for_release = request_id.clone();
                    let turn_for_release = turn_id.clone();
                    let _ = db
                        .run(move |db| {
                            db.release_queued_turn_message_dispatch(
                                &sid_for_release,
                                &request_id_for_release,
                                &turn_for_release,
                            )
                        })
                        .await;
                }
                return Err(AppError::internal(error.to_string()));
            }
        };
        body.attachments = persisted;
        meta
    };

    // Save user message to DB
    let mut user_msg = session::NewMessage::user(&effective_prompt)
        .with_source(ha_core::chat_engine::ChatSource::Http);
    user_msg.queue_request_id = queued_request_id.clone();
    user_msg.attachments_meta = session::build_chat_user_attachments_meta(
        body.is_plan_trigger.unwrap_or(false),
        body.plan_comment.as_ref(),
        body.goal_trigger.unwrap_or(false),
        queued_request_id.is_some(),
        attachments_meta,
    );
    let title_attachments_meta = user_msg.attachments_meta.clone();
    let user_message_result = {
        let sid = sid.clone();
        let turn_id = turn_id.clone();
        let effective_prompt = effective_prompt.clone();
        let edit_message_id = body.edit_message_id;
        let ui_surface_for_turn = body.ui_surface;
        let dispatch_request_id = detached_ui_dispatch
            .as_ref()
            .map(|dispatch| dispatch.request_id.clone());
        let dispatch_fingerprint = detached_ui_dispatch
            .as_ref()
            .map(|dispatch| dispatch.fingerprint.clone());
        db.run(move |db| {
            ha_core::chat_engine::active_turn::with_persistence_target(
                &sid,
                &turn_id,
                || -> anyhow::Result<_> {
                    if let Some(message_id) = edit_message_id {
                        let replacement_id = db.replace_last_user_message_for_edit(
                            &sid,
                            message_id,
                            &user_msg,
                            &turn_id,
                            ha_core::chat_engine::ChatSource::Http.as_str(),
                            ui_surface_for_turn,
                            dispatch_request_id.as_deref(),
                            dispatch_fingerprint.as_deref(),
                        )?;
                        let turn = db.get_chat_turn(&turn_id)?.ok_or_else(|| {
                            anyhow::anyhow!("replacement chat turn was not created")
                        })?;
                        return Ok((Some(replacement_id), turn));
                    }
                    let (user_message_id, turn) = db
                        .append_message_and_create_chat_turn_with_id_surface_dispatch(
                            &turn_id,
                            &sid,
                            ha_core::chat_engine::ChatSource::Http.as_str(),
                            None,
                            &user_msg,
                            ui_surface_for_turn,
                            dispatch_request_id.as_deref(),
                            dispatch_fingerprint.as_deref(),
                        )?;
                    // Auto-generate fallback title from first user message (prefer display text so titles read naturally).
                    let _ = session::ensure_first_message_title(
                        db,
                        &sid,
                        &effective_prompt,
                        title_attachments_meta.as_deref(),
                    );
                    Ok((Some(user_message_id), turn))
                },
            )
        })
        .await
    };
    let (_user_message_id, _turn) = match user_message_result {
        Ok(ha_core::chat_engine::active_turn::PersistenceTargetOutcome::Committed(value)) => value,
        Ok(ha_core::chat_engine::active_turn::PersistenceTargetOutcome::CommittedAfterCancel(
            _value,
        )) => {
            ha_core::hooks::set_user_prompt_context(&sid, None);
            if new_session_created {
                let cleanup = ha_core::chat_engine::stop::PreTurnCancelCleanup::begin(
                    db.clone(),
                    sid.clone(),
                    bootstrap_request_id.clone(),
                    true,
                    None,
                );
                ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
                    &sid,
                    None,
                    Some(&turn_id),
                    Some(session::ChatTurnStatus::Interrupted),
                    Some(session::ChatTurnInterruptReason::UserStop),
                    None,
                );
                ha_core::chat_engine::active_turn::force_release(&sid, &turn_id);
                if let Some(cleanup) = cleanup {
                    cleanup.spawn();
                }
            } else {
                let outcome = ha_core::chat_engine::stop::finalize_persisted_user_stop(
                    db.clone(),
                    sid.clone(),
                    turn_id.clone(),
                    effective_prompt.clone(),
                    ha_core::chat_engine::ChatSource::Http,
                )
                .await;
                ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
                    &sid,
                    None,
                    Some(&turn_id),
                    outcome
                        .turn_status
                        .or(Some(session::ChatTurnStatus::Interrupted)),
                    outcome
                        .interrupt_reason
                        .or(Some(session::ChatTurnInterruptReason::UserStop)),
                    None,
                );
                ha_core::chat_engine::active_turn::force_release(&sid, &turn_id);
            }
            ha_core::app_info!(
                "chat",
                "persistence_cancelled",
                "Stopped HTTP prompt after persistence claim: session={} turn={}",
                sid,
                turn_id
            );
            return Err(AppError::conflict_with_code(
                ha_core::agent::preflight::CHAT_CANCELLED_DURING_PREFLIGHT_CODE,
                "chat stopped while prompt persistence completed",
            ));
        }
        Err(error) => {
            if let Some(request_id) = queued_request_id.as_ref() {
                let sid_for_reconcile = sid.clone();
                let request_for_reconcile = request_id.clone();
                let turn_for_reconcile = turn_id.clone();
                let _ = db
                    .run(move |db| {
                        db.reconcile_failed_turn_message_dispatch(
                            &sid_for_reconcile,
                            &request_for_reconcile,
                            &turn_for_reconcile,
                        )
                    })
                    .await;
            }
            return Err(error.into());
        }
        Ok(ha_core::chat_engine::active_turn::PersistenceTargetOutcome::CancelledBeforeCommit) => {
            ha_core::hooks::set_user_prompt_context(&sid, None);
            let cleanup = ha_core::chat_engine::stop::PreTurnCancelCleanup::begin(
                db.clone(),
                sid.clone(),
                bootstrap_request_id.clone(),
                new_session_created,
                queued_request_id
                    .as_ref()
                    .map(|request_id| (request_id.clone(), turn_id.clone())),
            );
            ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
                &sid,
                None,
                Some(&turn_id),
                Some(session::ChatTurnStatus::Interrupted),
                Some(session::ChatTurnInterruptReason::UserStop),
                None,
            );
            ha_core::chat_engine::active_turn::force_release(&sid, &turn_id);
            if let Some(cleanup) = cleanup {
                cleanup.spawn();
            }
            ha_core::app_info!(
                "chat",
                "persistence_cancelled",
                "Stopped HTTP prompt before persistence claim: session={} turn={}",
                sid,
                turn_id
            );
            return Err(AppError::conflict_with_code(
                ha_core::agent::preflight::CHAT_CANCELLED_DURING_PREFLIGHT_CODE,
                "chat stopped before prompt persistence completed",
            ));
        }
    };
    let mut turn_drop_finalizer =
        HttpChatTurnDropFinalizer::new(db.clone(), sid.clone(), turn_id.clone());

    // This is the ownership-transfer boundary for the bundled HTTP UI. The
    // session, user message, and running chat_turn are durable and the detached
    // Tokio task owns every guard below. Returning this ACK can no longer drop
    // the engine. Keep a session-scoped approval surface registered so a turn
    // that reaches Ask while every browser is temporarily disconnected waits
    // for a reconnect instead of being misclassified as headless automation.
    let _reattachable_ui_guard = detached_ui_dispatch.as_ref().map(|dispatch| {
        let guard = ha_core::permission::register_reattachable_ui_session(&sid);
        let accepted = publish_ui_chat_dispatch(
            &dispatch.request_id,
            UiChatDispatchOutcome::Success(ChatResponse {
                session_id: sid.clone(),
                response: String::new(),
                turn_id: turn_id.clone(),
                blocked_reason: None,
                session_deleted: false,
                accepted: true,
            }),
        );
        debug_assert!(accepted, "detached UI dispatch must publish exactly once");
        guard
    });

    // Resolve model chain
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();

    // Session-scoped model pin trumps agent.primary and config.active_model
    // when no explicit per-turn override was provided. Mirrors the desktop
    // commands::chat path so the two transports stay in sync.
    let session_pinned_model: Option<String> = if body.model_override.is_none() {
        let lookup = {
            let sid = sid.clone();
            db.run(move |db| db.get_session(&sid)).await
        };
        lookup
            .ok()
            .flatten()
            .and_then(|meta| match (meta.provider_id, meta.model_id) {
                (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => {
                    Some(format!("{}::{}", p, m))
                }
                _ => None,
            })
    } else {
        None
    };

    // Explicit current-turn overrides are strict. Persisted Session pins are
    // preferences and may fall through, but an invalid override must not
    // silently switch the request to another Provider.
    if let Some(override_str) = body.model_override.as_deref() {
        let override_is_available = provider::parse_model_ref(override_str)
            .is_some_and(|model| provider::model_ref_is_available(&store.providers, &model));
        if !override_is_available {
            let err = format!(
                "Selected model override is unavailable: {override_str}. Please choose an enabled provider and model."
            );
            let partial = ha_core::chat_engine::finalize::PartialMeta {
                user_message: Some(body.message.clone()),
                turn_id: Some(turn_id.clone()),
                ..Default::default()
            };
            let outcome = ha_core::chat_engine::finalize::finalize_turn_context_blocking(
                &db,
                &sid,
                ha_core::chat_engine::finalize::TerminationReason::Other {
                    message: err.clone(),
                },
                partial,
                ha_core::chat_engine::ChatSource::Http,
            );
            ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
                &sid,
                None,
                Some(&turn_id),
                outcome.turn_status,
                outcome.interrupt_reason,
                Some(&err),
            );
            return Err(AppError::bad_request(err));
        }
    }

    let preferred_model = body
        .model_override
        .as_deref()
        .or(session_pinned_model.as_deref());
    let (primary, fallbacks) =
        provider::resolve_model_chain_with_preferred(preferred_model, &agent_model_config, &store);

    let model_chain: Vec<ActiveModel> = primary.into_iter().chain(fallbacks).collect();

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

    // Explicit API override remains per-turn; otherwise use the immutable
    // Session snapshot.
    let resolved_temperature = body.temperature_override.or(runtime_defaults.temperature);

    // Register per-session cancel flag after validation. The active-turn
    // guard above already prevents duplicate user-message persistence.
    {
        let mut cancels = ctx
            .chat_cancels
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cancels.insert(sid.clone(), cancel.clone());
    }
    let mut cancel_registration_guard =
        ChatCancelRegistrationGuard::new(ctx.chat_cancels.clone(), sid.clone(), cancel.clone());

    // HTTP stream delivery uses `/ws/events` via `chat:stream_delta`; the
    // EventBus bridge performs the HTTP attachment URL rewrite there.
    let event_sink: Arc<dyn EventSink> = Arc::new(NoopEventSink);

    let engine_params = ChatEngineParams {
        session_id: sid.clone(),
        agent_id: agent_id.clone(),
        turn_id: Some(turn_id.clone()),
        message: body.message.clone(),
        incoming_turn: body.incoming_turn,
        display_text: body.display_text.clone(),
        attachments: body.attachments,
        session_db: db.clone(),
        model_chain,
        providers: store.providers.clone(),
        codex_token: None,
        resolved_temperature,
        compact_config,
        run_context: None,
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
        follow_global_reasoning_effort: false,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        source: ha_core::chat_engine::stream_seq::ChatSource::Http,
        ui_surface: body.ui_surface,
        origin_source: None,
        // HTTP owner turn — KB access via attach, not the IM opt-in gate.
        channel_kb_context: None,
        event_sink,
    };

    if let Some(request_id) = bootstrap_request_id.as_deref() {
        let request_id = request_id.to_string();
        db.run(move |db| db.mark_project_bootstrap_completed(&request_id))
            .await?;
    }
    let result = ha_core::chat_engine::run_chat_engine(engine_params).await;

    if let Err(error) = &result {
        turn_drop_finalizer.finish_if_open(
            session::ChatTurnStatus::Failed,
            Some(session::ChatTurnInterruptReason::Unknown),
            Some(&format!(
                "chat engine returned before finalizing turn: {error}"
            )),
        );
    } else {
        turn_drop_finalizer.finish_if_open(session::ChatTurnStatus::Completed, None, None);
    }
    cancel_registration_guard.release();

    let result = result.map_err(AppError::internal)?;

    Ok(Json(ChatResponse {
        session_id: sid,
        response: result.response,
        turn_id,
        blocked_reason: None,
        session_deleted: false,
        accepted: false,
    }))
}

/// Exact turn status used by the HTTP UI after a dispatch ACK. Unlike the
/// session-level stream snapshot, this cannot accidentally observe a newer
/// turn after reconnecting or missing a terminal WebSocket event.
pub async fn get_chat_turn(
    State(ctx): State<Arc<AppContext>>,
    Path(turn_id): Path<String>,
) -> Result<Json<session::ChatTurn>, AppError> {
    let lookup_id = turn_id.clone();
    let turn = ctx
        .session_db
        .run(move |db| db.get_chat_turn(&lookup_id))
        .await?
        .ok_or_else(|| AppError::not_found(format!("chat turn not found: {turn_id}")))?;
    Ok(Json(turn))
}

/// Isolated model-evaluation telemetry snapshot. The endpoint is physically
/// present in the server binary but fails closed unless the process was
/// explicitly launched in model-eval mode. It returns aggregate counters and
/// hashes only; prompts, model responses, tool arguments, and secrets are not
/// retained by the registry.
pub async fn model_eval_trial_telemetry(
    Path(trial_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    if !ha_core::eval_context::model_eval_mode_enabled() {
        return Err(AppError::not_found(
            "model evaluation telemetry is disabled",
        ));
    }
    let value = ha_core::eval_context::telemetry_snapshot(&trial_id)
        .ok_or_else(|| AppError::not_found("model evaluation trial was not found"))?;
    Ok(Json(value))
}

/// Cancel and delete all synthetic Sessions created under a model-eval trial.
/// This is intentionally unavailable in normal server mode and remains behind
/// the owner bearer-token middleware. The Harness calls it after scoring every
/// attempt so queued jobs/subagents cannot leak into the next trial.
pub async fn cleanup_model_eval_trial(
    State(ctx): State<Arc<AppContext>>,
    Path(trial_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    if !ha_core::eval_context::model_eval_mode_enabled() {
        return Err(AppError::not_found("model evaluation cleanup is disabled"));
    }
    ha_core::eval_context::finish_trial_root(&trial_id)
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    let session_ids = ha_core::eval_context::session_ids_for_trial(&trial_id)
        .ok_or_else(|| AppError::not_found("model evaluation trial was not found"))?;
    let count = session_ids.len();
    ctx.session_db
        .run(move |db| {
            for session_id in session_ids {
                db.delete_session(&session_id)?;
            }
            anyhow::Ok(())
        })
        .await?;
    Ok(Json(json!({ "cleanedSessions": count })))
}

/// Mark the final scripted/replay user turn complete without deleting state,
/// allowing the Harness to inspect terminal owner APIs before cleanup.
pub async fn finish_model_eval_trial(
    Path(trial_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    if !ha_core::eval_context::model_eval_mode_enabled() {
        return Err(AppError::not_found(
            "model evaluation finalization is disabled",
        ));
    }
    ha_core::eval_context::finish_trial_root(&trial_id)
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(json!({ "finished": true })))
}

/// `POST /api/chat/turn-message` — queue a user message to be injected at the
/// next safe tool-loop boundary of the active turn.
pub async fn queue_turn_user_message(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<QueueTurnUserMessageRequest>,
) -> Result<Json<ha_core::chat_engine::turn_injection::QueueTurnUserMessageResult>, AppError> {
    validate_http_chat_attachments(
        &body.session_id,
        &body.message,
        body.incoming_turn.as_ref(),
        &body.attachments,
    )?;
    let request_id = body
        .request_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let session_id = body.session_id;
    let sid_for_files = session_id.clone();
    let request_for_files = request_id.clone();
    let mut attachments = body.attachments;
    attachments = ha_core::blocking::run_blocking(move || {
        ha_core::attachments::persist_queued_chat_attachments(
            &sid_for_files,
            &request_for_files,
            &mut attachments,
        )?;
        anyhow::Ok(attachments)
    })
    .await
    .map_err(|error| AppError::internal(error.to_string()))?;
    let attachments_for_cleanup = attachments.clone();
    let input = ha_core::session::NewQueuedTurnMessage {
        request_id: request_id.clone(),
        session_id: session_id.clone(),
        message: body.message,
        display_text: body.display_text,
        attachments,
        is_plan_trigger: body.is_plan_trigger.unwrap_or(false),
        goal_trigger: body.goal_trigger.unwrap_or(false),
        plan_comment: body.plan_comment,
        plan_mode: body.plan_mode,
        workflow_mode: body.workflow_mode,
        incoming_turn: body.incoming_turn,
        skill_allowed_tools: Vec::new(),
        source: ha_core::session::QueuedTurnMessageSource::Http,
        channel_origin: None,
    };
    let item_result = ctx
        .session_db
        .run(move |db| db.enqueue_turn_user_message(input))
        .await;
    let item = match item_result {
        Ok(outcome) => {
            if !outcome.inserted {
                ha_core::attachments::remove_discarded_queued_attachments(
                    &session_id,
                    &request_id,
                    &attachments_for_cleanup,
                );
            }
            outcome.item
        }
        Err(error) => {
            ha_core::attachments::remove_discarded_queued_attachments(
                &session_id,
                &request_id,
                &attachments_for_cleanup,
            );
            return Err(AppError::bad_request(error.to_string()));
        }
    };
    Ok(Json(
        ha_core::chat_engine::turn_injection::QueueTurnUserMessageResult {
            queued: true,
            request_id,
            reason: None,
            item: Some(item),
        },
    ))
}

pub async fn list_queued_turn_user_messages(
    State(ctx): State<Arc<AppContext>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ha_core::session::QueuedTurnMessageView>>, AppError> {
    let items = ctx
        .session_db
        .run(move |db| db.list_queued_turn_user_messages(&session_id))
        .await?;
    Ok(Json(items))
}

pub async fn update_queued_turn_user_message(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<UpdateQueuedTurnUserMessageRequest>,
) -> Result<Json<bool>, AppError> {
    let changed = ctx
        .session_db
        .run(move |db| {
            db.update_queued_turn_user_message(
                &body.session_id,
                &body.request_id,
                &body.message,
                body.display_text.as_deref(),
            )
        })
        .await?;
    Ok(Json(changed))
}

pub async fn delete_queued_turn_user_message(
    State(ctx): State<Arc<AppContext>>,
    Path((session_id, request_id)): Path<(String, String)>,
) -> Result<Json<bool>, AppError> {
    let changed = ctx
        .session_db
        .run(move |db| db.delete_queued_turn_user_message(&session_id, &request_id))
        .await?;
    Ok(Json(changed))
}

pub async fn insert_queued_turn_user_message(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CancelQueuedTurnUserMessageRequest>,
) -> Result<Json<ha_core::chat_engine::turn_injection::QueueTurnUserMessageResult>, AppError> {
    let result = ctx
        .session_db
        .run(move |db| {
            ha_core::chat_engine::turn_injection::request_insertion(
                db,
                &body.session_id,
                &body.turn_id,
                &body.request_id,
            )
        })
        .await?;
    Ok(Json(result))
}

/// `POST /api/chat/turn-message/cancel` — cancel a not-yet-injected queued
/// message for an active turn.
pub async fn cancel_queued_turn_user_message(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CancelQueuedTurnUserMessageRequest>,
) -> Result<Json<ha_core::chat_engine::turn_injection::CancelQueuedTurnMessageResult>, AppError> {
    let result = ctx
        .session_db
        .run(move |db| {
            ha_core::chat_engine::turn_injection::cancel_insertion(
                db,
                &body.session_id,
                &body.turn_id,
                &body.request_id,
            )
        })
        .await?;
    Ok(Json(result))
}

/// `POST /api/chat/stop` — stop ongoing chat(s).
///
/// When the request body provides `sessionId`, only that session's cancel
/// flag is flipped. Otherwise every running chat is cancelled. Accepts
/// either `{}` or omitted body — `axum::Json` with a `Default` body handles
/// `{}`; for a completely empty body the Tauri caller wouldn't reach this
/// route anyway.
pub async fn stop_chat(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<StopChatRequest>,
) -> Result<Json<Value>, AppError> {
    let request_scoped_stop =
        body.client_request_id.is_some() && (body.session_id.is_none() || body.turn_id.is_none());
    let _bootstrap_signalled = body
        .client_request_id
        .as_deref()
        .is_some_and(ha_core::project_bootstrap::cancel_project_bootstrap_for_client_request);
    let request_cancel = if request_scoped_stop {
        body.client_request_id.as_deref().map(|request_id| {
            ha_core::chat_engine::active_turn::cancel_or_latch_client_request(
                request_id,
                body.session_id.as_deref(),
            )
        })
    } else {
        None
    };
    if matches!(
        request_cancel.as_ref(),
        Some(ha_core::chat_engine::active_turn::ClientRequestCancelOutcome::SessionMismatch)
    ) {
        return Ok(Json(json!({
            "stopped": false,
            "scope": if body.session_id.is_some() { "session" } else { "request" },
            "reason": "client request is not owned by the target session",
            "runtimeCancellations": [],
            "runtimeCancellationError": Value::Null,
        })));
    }
    if matches!(
        request_cancel.as_ref(),
        Some(ha_core::chat_engine::active_turn::ClientRequestCancelOutcome::Latched)
    ) {
        // The opaque request is not registered yet. Its Stop is now
        // authoritatively represented by the in-memory request latch; a session id supplied
        // by the caller is only an ownership constraint, not permission to
        // cancel a different active turn in that session.
        return Ok(Json(json!({
            "stopped": true,
            "scope": "request",
            "reason": Value::Null,
            "runtimeCancellations": [],
            "runtimeCancellationError": Value::Null,
        })));
    }
    let request_target = request_cancel.as_ref().and_then(|outcome| match outcome {
        ha_core::chat_engine::active_turn::ClientRequestCancelOutcome::Active(active) => {
            Some(active.clone())
        }
        _ => None,
    });
    let mut already_signalled = matches!(
        request_cancel,
        Some(ha_core::chat_engine::active_turn::ClientRequestCancelOutcome::Active(_))
    );
    let (target_session_id, target_turn_id) =
        ha_core::chat_engine::active_turn::resolve_stop_target(
            body.session_id.as_deref(),
            body.turn_id.as_deref(),
            request_target.as_ref(),
        );
    let global_stop = body.session_id.is_none() && body.client_request_id.is_none();
    let registered_cancels: std::collections::HashMap<_, _> = ctx
        .chat_cancels
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|(sid, cancel)| (sid.clone(), cancel.clone()))
        .collect();

    if let Some(sid) = target_session_id.as_deref() {
        // A session-only Stop is authoritative during the HTTP shell's
        // pre-active-turn window. An exact turn Stop must not flip this
        // session-keyed handle because it may already belong to a newer turn.
        if target_turn_id.is_none() {
            if let Some(cancel) = registered_cancels.get(sid) {
                cancel.store(true, Ordering::SeqCst);
                already_signalled = true;
            }
        }

        let outcome = ha_core::chat_engine::stop::stop_session(
            ctx.session_db.clone(),
            sid,
            target_turn_id.as_deref(),
            already_signalled,
        )
        .await;
        return Ok(Json(json!({
            "stopped": outcome.stopped,
            "scope": if body.session_id.is_some() { "session" } else { "request" },
            "reason": if outcome.stopped { Value::Null } else { json!("no matching active chat for target") },
            "runtimeCancellations": outcome.runtime_cancellations,
            "runtimeCancellationError": outcome.runtime_cancellation_error,
        })));
    }

    if !global_stop {
        return Ok(Json(json!({
            "stopped": already_signalled,
            "scope": "request",
            "reason": if already_signalled { Value::Null } else { json!("no matching active chat for target") },
            "runtimeCancellations": [],
            "runtimeCancellationError": Value::Null,
        })));
    }

    // Signal transport-local handles before the shared service's first await.
    // Core owns active turns, Channel inbounds, durable state, interaction
    // waits, runtime work and watchdog convergence for every shell.
    let pre_signalled_sessions = registered_cancels
        .iter()
        .map(|(sid, cancel)| {
            cancel.store(true, Ordering::SeqCst);
            sid.clone()
        })
        .collect::<Vec<_>>();
    let outcome = ha_core::chat_engine::stop::stop_all_sessions(
        ctx.session_db.clone(),
        pre_signalled_sessions,
        false,
    )
    .await;
    Ok(Json(json!({
        "stopped": outcome.stopped,
        "scope": "all",
        "count": outcome.stopped_session_count,
        "runtimeCancellations": outcome.runtime_cancellations,
        "runtimeCancellationError": outcome.runtime_cancellation_error,
    })))
}

/// `POST /api/chat/recovery/control` — control the exact visible recovery
/// wait. A random recovery id prevents a stale card from affecting a later
/// retry in the same session.
pub async fn control_model_recovery(
    Json(body): Json<RecoveryControlRequest>,
) -> Result<Json<ha_core::recovery_control::RecoveryControlResult>, AppError> {
    Ok(Json(ha_core::recovery_control::request(
        &body.session_id,
        &body.recovery_id,
        body.action,
    )))
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
        .run(move |db| db.update_session_permission_mode(&body.session_id, body.mode))
        .await?;
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
    {
        let session_id = body.session_id.clone();
        let mode = body.mode;
        ctx.session_db
            .run(move |db| db.update_session_sandbox_mode(&session_id, mode))
            .await?;
    }
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

    let prompt = {
        let session_id = q.session_id.clone();
        ha_core::blocking::run_blocking(move || {
            ha_core::agent::build_system_prompt_with_session(
                &agent_id,
                &model,
                &provider_name,
                session_id.as_deref(),
            )
        })
        .await
    };
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

/// `GET /api/chat/approvals/pending` — authoritative recovery snapshot for
/// owner UIs after renderer reloads or WebSocket event gaps.
pub async fn list_pending_approvals() -> Result<Json<Vec<tools::ApprovalRequest>>, AppError> {
    Ok(Json(tools::list_pending_approval_requests().await))
}

/// `POST /api/chat/attachment` — persist an uploaded attachment (multipart/form-data).
///
/// Multipart fields: `file` (required), `sessionId` / `fileName` / `mimeType` (optional text).
pub async fn save_attachment(multipart: Multipart) -> Result<Json<Value>, AppError> {
    let upload = parse_file_upload_to_temp(
        multipart,
        ha_core::attachments::legacy_chat_attachment_bytes(),
    )
    .await?;
    let session_id = upload.extra_fields.get("sessionId").cloned();
    let file_name = upload.file_name;
    let file_path = upload.file_path;
    let path = ha_core::blocking::run_blocking(move || {
        ha_core::attachments::save_attachment_file(
            session_id.as_deref(),
            &file_name,
            file_path.as_ref(),
        )
    })
    .await
    .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(Json(json!({ "path": path })))
}

/// `POST /api/chat/attachment-stage` — create an opaque one-hour upload lease.
pub async fn stage_chat_attachment(
    multipart: Multipart,
) -> Result<Json<ha_core::attachments::AttachmentUploadLease>, AppError> {
    let upload = parse_file_upload_to_temp(
        multipart,
        ha_core::attachments::legacy_chat_attachment_bytes(),
    )
    .await?;
    let file_name = upload.file_name;
    let mime_type = upload
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let file_path = upload.file_path;
    let lease = ha_core::blocking::run_blocking(move || {
        ha_core::attachments::stage_chat_attachment_file(&file_name, &mime_type, file_path.as_ref())
    })
    .await
    .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(lease))
}

/// `DELETE /api/chat/attachment-stage/{upload_id}` — release an unclaimed lease.
pub async fn discard_chat_attachment_upload(
    Path(upload_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::attachments::discard_chat_attachment_upload(&upload_id)
    })
    .await
    .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(json!({ "ok": true })))
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
    let prompt = {
        let session_id = body.session_id.clone();
        ha_core::blocking::run_blocking(move || {
            ha_core::agent::build_system_prompt_with_session(
                &agent_id,
                &model,
                &provider_name,
                session_id.as_deref(),
            )
        })
        .await
    };
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

/// `GET /api/chat/capability-mentions` — local, bounded Plugin/Connector
/// discovery metadata for the typed composer. This endpoint does not connect
/// to remote services and does not return credentials or tool schemas.
pub async fn list_capability_mentions(
    axum::extract::Query(q): axum::extract::Query<CapabilityMentionQuery>,
) -> Result<Json<Vec<ha_core::mention_hooks::MentionCapabilityCandidate>>, AppError> {
    let agent_id = q
        .agent_id
        .unwrap_or_else(|| ha_core::agent_loader::DEFAULT_AGENT_ID.to_string());
    Ok(Json(ha_core::mention_hooks::list_capability_mentions(
        &agent_id,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_direct_and_queue_boundary_rejects_unbound_typed_payload() {
        let forged = Attachment {
            name: "forged.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("mention".to_string()),
            data: Some("client-inline-data".to_string()),
            file_path: Some("/tmp/forged.txt".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_role: None,
        };
        assert!(
            validate_http_chat_attachments("missing-session", "plain", None, &[forged]).is_err()
        );

        let ordinary = Attachment {
            name: "upload.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("upload".to_string()),
            data: None,
            file_path: None,
            upload_id: Some("opaque-lease".to_string()),
            quote_lines: None,
            quote_role: None,
        };
        assert!(
            validate_http_chat_attachments("missing-session", "plain", None, &[ordinary]).is_ok()
        );
    }

    #[test]
    fn http_chat_rejects_untrusted_file_path_attachments() {
        let attachments = vec![Attachment {
            name: "secret.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("mention".to_string()),
            data: None,
            file_path: Some("/tmp/secret.txt".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_role: None,
        }];

        assert!(
            validate_http_chat_attachments("missing-session", "hi", None, &attachments).is_err()
        );
    }

    #[test]
    fn http_chat_rejects_forged_uploaded_file_path_attachments() {
        let attachments = vec![Attachment {
            name: "upload.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("upload".to_string()),
            data: None,
            file_path: Some("/tmp/upload.txt".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_role: None,
        }];

        assert!(
            validate_http_chat_attachments("missing-session", "hi", None, &attachments).is_err()
        );
    }

    #[test]
    fn http_chat_rejects_forged_pasted_text_file_path_attachments() {
        let attachments = vec![Attachment {
            name: "pasted-text.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some(ha_core::attachments::PASTED_TEXT_SOURCE.to_string()),
            data: None,
            file_path: Some("/tmp/pasted-text.txt".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_role: None,
        }];

        assert!(
            validate_http_chat_attachments("missing-session", "hi", None, &attachments).is_err()
        );
    }

    #[test]
    fn http_chat_accepts_opaque_upload_lease() {
        let attachments = vec![Attachment {
            name: "upload.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("upload".to_string()),
            data: None,
            file_path: None,
            upload_id: Some(uuid::Uuid::new_v4().to_string()),
            quote_lines: None,
            quote_role: None,
        }];

        assert!(
            validate_http_chat_attachments("missing-session", "hi", None, &attachments).is_ok()
        );
    }

    #[test]
    fn http_chat_rejects_mixed_upload_lease_and_path() {
        let attachments = vec![Attachment {
            name: "upload.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("upload".to_string()),
            data: None,
            file_path: Some("/tmp/upload.txt".to_string()),
            upload_id: Some(uuid::Uuid::new_v4().to_string()),
            quote_lines: None,
            quote_role: None,
        }];

        assert!(
            validate_http_chat_attachments("missing-session", "hi", None, &attachments).is_err()
        );
    }

    #[test]
    fn http_chat_accepts_only_registry_bound_plan_mention_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registered = temp.path().join("plan.md");
        std::fs::write(&registered, "# Plan").expect("write plan");
        let other = temp.path().join("other.md");
        std::fs::write(&other, "# Other").expect("write other");

        let message = "@plan:abcdef12:v3";
        let digest = ha_core::prompt_context::canonical_text_digest(message);
        let incoming_turn = ha_core::prompt_context::IncomingTurnWire {
            prompt_contract_version: ha_core::prompt_context::PROMPT_CONTRACT_VERSION,
            mention_wire_version: ha_core::prompt_context::MENTION_WIRE_VERSION,
            user_input: ha_core::prompt_context::CanonicalUserInput {
                input_item_id: "input-1".into(),
                canonicalization_version: 1,
                text: message.into(),
                digest: digest.clone(),
            },
            mentions: vec![ha_core::prompt_context::MentionBindingWire {
                id: "plan-1".into(),
                kind: ha_core::prompt_context::MentionKind::Plan,
                target_id: "abcdef12:v3".into(),
                display_label: "Plan".into(),
                origin: ha_core::prompt_context::StructuredMentionOrigin::FirstPartyComposerGesture,
                source_anchor: ha_core::prompt_context::SourceAnchor::Inline {
                    input_item_id: "input-1".into(),
                    canonical_text_digest: digest,
                    start_utf8: 0,
                    end_utf8: message.len() as u64,
                },
            }],
        };
        let attachment_for = |path: &std::path::Path| Attachment {
            name: "plan.md".to_string(),
            mime_type: "text/markdown".to_string(),
            source: Some("plan_mention".to_string()),
            data: None,
            file_path: Some(path.to_string_lossy().into_owned()),
            upload_id: None,
            quote_lines: None,
            quote_role: None,
        };
        let resolver = |short_id: &str, version: u32| {
            (short_id == "abcdef12" && version == 3).then(|| registered.clone())
        };

        assert!(validate_http_chat_attachments_with_plan_resolver(
            "missing-session",
            message,
            Some(&incoming_turn),
            &[attachment_for(&registered)],
            &resolver,
        )
        .is_ok());
        assert!(validate_http_chat_attachments_with_plan_resolver(
            "missing-session",
            message,
            Some(&incoming_turn),
            &[attachment_for(&other)],
            &resolver,
        )
        .is_err());
        assert!(validate_http_chat_attachments_with_plan_resolver(
            "missing-session",
            message,
            None,
            &[attachment_for(&registered)],
            &resolver,
        )
        .is_err());
    }

    #[test]
    fn chat_request_accepts_project_id_camel_case() {
        // The lazy project-session flow sends `projectId` on the first message so
        // the auto-create branch can bind the new session to the project.
        let body = serde_json::json!({
            "message": "hi",
            "projectId": "proj-123",
            "workflowMode": "on",
        });
        let req: ChatRequest = serde_json::from_value(body).expect("deserialize chat request");
        assert_eq!(req.project_id.as_deref(), Some("proj-123"));
        assert_eq!(
            req.workflow_mode,
            Some(ha_core::workflow_mode::WorkflowMode::On)
        );
        // Omitted project_id defaults to None (plain chats are unaffected).
        let plain: ChatRequest =
            serde_json::from_value(serde_json::json!({ "message": "hi" })).expect("deserialize");
        assert_eq!(plain.project_id, None);
    }

    #[tokio::test]
    async fn ui_chat_dispatch_is_idempotent_for_the_same_request() {
        let request_id = uuid::Uuid::new_v4().to_string();
        let first =
            subscribe_ui_chat_dispatch(&request_id, "fingerprint-7").expect("first subscription");
        let duplicate = subscribe_ui_chat_dispatch(&request_id, "fingerprint-7")
            .expect("duplicate subscription");
        assert!(first.leader);
        assert!(!duplicate.leader);

        let response = ChatResponse {
            session_id: "session-1".to_string(),
            response: String::new(),
            turn_id: "turn-1".to_string(),
            blocked_reason: None,
            session_deleted: false,
            accepted: true,
        };
        assert!(publish_ui_chat_dispatch(
            &request_id,
            UiChatDispatchOutcome::Success(response)
        ));

        for receiver in [first.receiver, duplicate.receiver] {
            match receiver.await.expect("dispatch outcome") {
                UiChatDispatchOutcome::Success(response) => {
                    assert_eq!(response.session_id, "session-1");
                    assert_eq!(response.turn_id, "turn-1");
                    assert!(response.accepted);
                }
                UiChatDispatchOutcome::Failure(_) => panic!("unexpected failure"),
            }
        }

        let replay =
            subscribe_ui_chat_dispatch(&request_id, "fingerprint-7").expect("cached replay");
        assert!(!replay.leader);
        assert!(matches!(
            replay.receiver.await.expect("cached outcome"),
            UiChatDispatchOutcome::Success(ChatResponse { accepted: true, .. })
        ));
        assert!(subscribe_ui_chat_dispatch(&request_id, "fingerprint-8").is_err());
    }

    #[tokio::test]
    async fn dropped_ui_request_receiver_does_not_lose_the_dispatch_ack() {
        let request_id = uuid::Uuid::new_v4().to_string();
        let disconnected =
            subscribe_ui_chat_dispatch(&request_id, "fingerprint-11").expect("first subscription");
        assert!(disconnected.leader);
        drop(disconnected.receiver);

        assert!(publish_ui_chat_dispatch(
            &request_id,
            UiChatDispatchOutcome::Success(ChatResponse {
                session_id: "session-after-disconnect".to_string(),
                response: String::new(),
                turn_id: "turn-after-disconnect".to_string(),
                blocked_reason: None,
                session_deleted: false,
                accepted: true,
            })
        ));

        let reconnected = subscribe_ui_chat_dispatch(&request_id, "fingerprint-11")
            .expect("reconnect subscription");
        match reconnected.receiver.await.expect("cached ACK") {
            UiChatDispatchOutcome::Success(response) => {
                assert_eq!(response.session_id, "session-after-disconnect");
                assert_eq!(response.turn_id, "turn-after-disconnect");
                assert!(response.accepted);
            }
            UiChatDispatchOutcome::Failure(_) => panic!("unexpected failure"),
        }
    }
}
