use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

use crate::process_registry::create_session_id;

// ── Command Approval System ───────────────────────────────────────

/// Approval request sent to frontend
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub command: String,
    pub cwd: String,
}

/// Approval response from frontend
#[derive(Debug, Clone, Deserialize)]
pub enum ApprovalResponse {
    AllowOnce,
    AllowAlways, // adds command pattern to allowlist
    Deny,
}

/// Global approval request registry
static PENDING_APPROVALS: OnceLock<
    TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<ApprovalResponse>>>,
> = OnceLock::new();

fn get_pending_approvals(
) -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<ApprovalResponse>>> {
    PENDING_APPROVALS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Submit an approval response (called by Tauri command from frontend)
pub async fn submit_approval_response(request_id: &str, response: ApprovalResponse) -> Result<()> {
    let mut pending = get_pending_approvals().lock().await;
    if let Some(sender) = pending.remove(request_id) {
        let _ = sender.send(response);
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "No pending approval request: {}",
            request_id
        ))
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

/// Request approval from the user for a command.
/// Emits a Tauri event and waits for the response via oneshot channel.
pub(crate) async fn check_and_request_approval(
    command: &str,
    cwd: &str,
) -> Result<ApprovalResponse> {
    use tauri::Emitter;

    let request_id = create_session_id();
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Register the pending approval
    {
        let mut pending = get_pending_approvals().lock().await;
        pending.insert(request_id.clone(), tx);
    }

    // Emit event to frontend
    let request = ApprovalRequest {
        request_id: request_id.clone(),
        command: command.to_string(),
        cwd: cwd.to_string(),
    };

    if let Some(handle) = crate::get_app_handle() {
        let event_data = serde_json::to_string(&request)?;
        handle
            .emit("approval_required", event_data)
            .map_err(|e| anyhow::anyhow!("Failed to emit approval event: {}", e))?;
        app_info!("tool", "approval",
            "Approval requested for command: {} (id: {})",
            command,
            request_id
        );
    } else {
        // No AppHandle available, clean up and return error
        let mut pending = get_pending_approvals().lock().await;
        pending.remove(&request_id);
        return Err(anyhow::anyhow!(
            "AppHandle not available for approval events"
        ));
    }

    // Wait for response with timeout (5 minutes)
    match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(response)) => {
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
        Ok(Err(_)) => {
            if let Some(logger) = crate::get_logger() {
                logger.log("warn", "tool", "approval::cancelled",
                    &format!("Approval cancelled for '{}'", command), None, None, None);
            }
            Err(anyhow::anyhow!("Approval request cancelled"))
        }
        Err(_) => {
            // Timeout — clean up
            let mut pending = get_pending_approvals().lock().await;
            pending.remove(&request_id);
            if let Some(logger) = crate::get_logger() {
                logger.log("warn", "tool", "approval::timeout",
                    &format!("Approval timed out for '{}'", command), None, None, None);
            }
            Err(anyhow::anyhow!("Approval request timed out (5 min)"))
        }
    }
}
