use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

use crate::process_registry::create_session_id;

// Shared tool state lives in process-global `OnceLock<TokioMutex<…>>` cells
// in this module on purpose: see the concurrency contract on
// [`super::execution::ToolExecContext`]. The tool loop clones the per-call
// context for every concurrent branch, so any mutable state that must be
// observed across concurrent tools or across rounds has to sit outside the
// context struct. Add new shared state here (or in a sibling module) rather
// than reaching for `Mutex<…>` inside `ToolExecContext`.
//
// Per-session permission mode (Default / Smart / Yolo) lives in the SQLite
// `sessions.permission_mode` column and is read into [`ToolExecContext.session_mode`]
// by the agent setup path. The legacy process-global `TOOL_PERMISSION_MODE`
// static was removed in the permission system v2 redesign.

// ── Command Approval System ───────────────────────────────────────

/// Approval request sent to frontend and IM channel approval listeners.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub command: String,
    pub cwd: String,
    /// Session ID for correlating with IM channel conversations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional reason emitted by the permission engine. The frontend
    /// renders a colored banner and disables AllowAlways for strict reasons
    /// (`protected_path` / `dangerous_command`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ApprovalReasonPayload>,
}

/// Reason payload — flat shape so the frontend can switch on `kind` without
/// running a full enum matcher. Mirrors [`crate::permission::AskReason`] but
/// strips internal struct fields the UI doesn't need.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApprovalReasonPayload {
    pub kind: ApprovalReasonKind,
    /// Human-readable detail (matched pattern, path, rationale…). Optional —
    /// `edit_tool` carries no extra detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// KEEP IN SYNC with the TS string union in
/// [`src/components/chat/ApprovalDialog.tsx`] (`ApprovalRequest.reason.kind`).
/// Adding a variant here without updating that union leaves the frontend
/// without a banner — TS won't catch the drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReasonKind {
    EditTool,
    EditCommand,
    DangerousCommand,
    ProtectedPath,
    AgentCustomList,
    SmartJudge,
}

impl From<&crate::permission::AskReason> for ApprovalReasonPayload {
    fn from(value: &crate::permission::AskReason) -> Self {
        use crate::permission::AskReason::*;
        match value {
            EditTool => Self {
                kind: ApprovalReasonKind::EditTool,
                detail: None,
            },
            EditCommand { matched_pattern } => Self {
                kind: ApprovalReasonKind::EditCommand,
                detail: Some(matched_pattern.clone()),
            },
            DangerousCommand { matched_pattern } => Self {
                kind: ApprovalReasonKind::DangerousCommand,
                detail: Some(matched_pattern.clone()),
            },
            ProtectedPath { matched_path } => Self {
                kind: ApprovalReasonKind::ProtectedPath,
                detail: Some(matched_path.clone()),
            },
            AgentCustomList => Self {
                kind: ApprovalReasonKind::AgentCustomList,
                detail: None,
            },
            SmartJudge { rationale } => Self {
                kind: ApprovalReasonKind::SmartJudge,
                detail: Some(rationale.clone()),
            },
        }
    }
}

/// Approval response from frontend
#[derive(Debug, Clone, Deserialize)]
pub enum ApprovalResponse {
    AllowOnce,
    AllowAlways, // adds command pattern to allowlist
    Deny,
}

/// In-memory entry for a pending approval. Stores the oneshot sender plus the
/// originating session id so we can aggregate counts per session for the
/// sidebar "needs your response" indicator.
struct PendingApprovalEntry {
    sender: tokio::sync::oneshot::Sender<ApprovalResponse>,
    session_id: Option<String>,
}

/// Global approval request registry
static PENDING_APPROVALS: OnceLock<TokioMutex<HashMap<String, PendingApprovalEntry>>> =
    OnceLock::new();

fn get_pending_approvals() -> &'static TokioMutex<HashMap<String, PendingApprovalEntry>> {
    PENDING_APPROVALS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Count pending approvals grouped by session id. Approvals registered without
/// a session id (e.g. global commands triggered outside any chat) are skipped.
pub async fn pending_approvals_per_session() -> HashMap<String, i64> {
    let pending = get_pending_approvals().lock().await;
    let mut out: HashMap<String, i64> = HashMap::new();
    for entry in pending.values() {
        if let Some(sid) = entry.session_id.as_ref() {
            *out.entry(sid.clone()).or_insert(0) += 1;
        }
    }
    out
}

/// Submit an approval response (called by Tauri command from frontend)
pub async fn submit_approval_response(request_id: &str, response: ApprovalResponse) -> Result<()> {
    let mut pending = get_pending_approvals().lock().await;
    if let Some(entry) = pending.remove(request_id) {
        let session_id = entry.session_id.clone();
        let _ = entry.sender.send(response);
        drop(pending);
        emit_pending_interactions_changed(session_id.as_deref());
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "No pending approval request: {}",
            request_id
        ))
    }
}

/// Broadcast to the frontend that some session's pending-interaction count
/// likely changed so the sidebar should reload its list. Payload carries an
/// optional `session_id` for clients that want to optimise; a missing id means
/// "any session, please refresh".
pub fn emit_pending_interactions_changed(session_id: Option<&str>) {
    if let Some(bus) = crate::globals::get_event_bus() {
        let payload = match session_id {
            Some(sid) => serde_json::json!({ "sessionId": sid }),
            None => serde_json::json!({}),
        };
        bus.emit("session_pending_interactions_changed", payload);
    }
}

/// Allowlist: command prefixes that are auto-approved
static COMMAND_ALLOWLIST: OnceLock<TokioMutex<Vec<String>>> = OnceLock::new();

fn get_allowlist() -> &'static TokioMutex<Vec<String>> {
    COMMAND_ALLOWLIST.get_or_init(|| {
        let list = load_allowlist().unwrap_or_default();
        TokioMutex::new(list)
    })
}

fn allowlist_path() -> std::path::PathBuf {
    crate::paths::root_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("exec-approvals.json")
}

fn load_allowlist() -> Result<Vec<String>> {
    let path = allowlist_path();
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(Vec::new())
    }
}

async fn save_allowlist(list: &[String]) -> Result<()> {
    let data = serde_json::to_string_pretty(list)?;
    tokio::fs::write(allowlist_path(), data).await?;
    Ok(())
}

/// Check if command is in the allowlist
pub(crate) async fn is_command_allowed(command: &str) -> bool {
    let list = get_allowlist().lock().await;
    let cmd_trimmed = command.trim();
    list.iter()
        .any(|pattern| cmd_trimmed.starts_with(pattern) || cmd_trimmed == *pattern)
}

/// Add command prefix to allowlist
pub(crate) async fn add_to_allowlist(command: &str) {
    let mut list = get_allowlist().lock().await;
    let prefix = extract_command_prefix(command);
    if !list.contains(&prefix) {
        list.push(prefix);
        let _ = save_allowlist(&list).await;
    }
}

/// Extract a meaningful command prefix for the allowlist
fn extract_command_prefix(command: &str) -> String {
    let trimmed = command.trim();
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

fn approval_timeout_secs() -> u64 {
    crate::config::cached_config()
        .permission
        .approval_timeout_secs
}

pub(crate) fn approval_timeout_action() -> crate::config::ApprovalTimeoutAction {
    crate::config::cached_config()
        .permission
        .approval_timeout_action
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalCheckError {
    RequestSerialization,
    EventBusUnavailable,
    Cancelled,
    TimedOut { timeout_secs: u64 },
}

impl fmt::Display for ApprovalCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestSerialization => write!(f, "Failed to serialize approval request"),
            Self::EventBusUnavailable => write!(f, "EventBus not available for approval events"),
            Self::Cancelled => write!(f, "Approval request cancelled"),
            Self::TimedOut { timeout_secs } => {
                write!(f, "Approval request timed out ({}s)", timeout_secs)
            }
        }
    }
}

/// Request approval from the user for a command.
/// Emits an EventBus event and waits for the response via oneshot channel.
/// `session_id` is used by the IM channel approval listener to route the
/// request to the correct chat.
pub(crate) async fn check_and_request_approval(
    command: &str,
    cwd: &str,
    session_id: Option<&str>,
    reason: Option<ApprovalReasonPayload>,
) -> std::result::Result<ApprovalResponse, ApprovalCheckError> {
    let request_id = create_session_id();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let timeout_secs = approval_timeout_secs();

    // Register the pending approval
    {
        let mut pending = get_pending_approvals().lock().await;
        pending.insert(
            request_id.clone(),
            PendingApprovalEntry {
                sender: tx,
                session_id: session_id.map(|s| s.to_string()),
            },
        );
    }

    // Emit event to frontend
    let request = ApprovalRequest {
        request_id: request_id.clone(),
        command: command.to_string(),
        cwd: cwd.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        reason,
    };

    if let Some(bus) = crate::globals::get_event_bus() {
        let event_data = match serde_json::to_value(&request) {
            Ok(value) => value,
            Err(_) => {
                let mut pending = get_pending_approvals().lock().await;
                pending.remove(&request_id);
                return Err(ApprovalCheckError::RequestSerialization);
            }
        };
        bus.emit("approval_required", event_data);
        app_info!(
            "tool",
            "approval",
            "Approval requested for command: {} (id: {})",
            command,
            request_id
        );
    } else {
        // No EventBus available, clean up and return error
        let mut pending = get_pending_approvals().lock().await;
        pending.remove(&request_id);
        return Err(ApprovalCheckError::EventBusUnavailable);
    }

    let wait_result = if timeout_secs == 0 {
        rx.await.map_err(|_| "cancelled")
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err("cancelled"),
            Err(_) => Err("timeout"),
        }
    };

    match wait_result {
        Ok(response) => {
            if let Some(logger) = crate::get_logger() {
                let response_str = match &response {
                    ApprovalResponse::AllowOnce => "allow_once",
                    ApprovalResponse::AllowAlways => "allow_always",
                    ApprovalResponse::Deny => "deny",
                };
                logger.log("info", "tool", "approval::response",
                    &format!("Approval response: {} for '{}'", response_str, command),
                    Some(serde_json::json!({"command": command, "response": response_str, "request_id": request_id}).to_string()),
                    None, None);
            }
            Ok(response)
        }
        Err("cancelled") => {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "tool",
                    "approval::cancelled",
                    &format!("Approval cancelled for '{}'", command),
                    None,
                    None,
                    None,
                );
            }
            Err(ApprovalCheckError::Cancelled)
        }
        Err("timeout") => {
            // Timeout — clean up
            {
                let mut pending = get_pending_approvals().lock().await;
                pending.remove(&request_id);
            }
            emit_pending_interactions_changed(session_id);
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "tool",
                    "approval::timeout",
                    &format!(
                        "Approval timed out for '{}' after {}s",
                        command, timeout_secs
                    ),
                    None,
                    None,
                    None,
                );
            }
            Err(ApprovalCheckError::TimedOut { timeout_secs })
        }
        Err(_) => unreachable!(),
    }
}
