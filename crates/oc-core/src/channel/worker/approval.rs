//! IM channel tool approval interaction.
//!
//! When a tool requires approval during an IM channel conversation, this module
//! intercepts the `"approval_required"` EventBus event, sends an approval prompt
//! to the IM channel (with buttons if supported, text fallback otherwise), and
//! routes the user's response back to `submit_approval_response()`.

use std::collections::HashMap;
use std::sync::OnceLock;

use tokio::sync::Mutex;

use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{InlineButton, ReplyPayload};
use crate::tools::approval::{submit_approval_response, ApprovalResponse};

use std::sync::Arc;

/// Callback data prefix for approval buttons across all channels.
const APPROVAL_PREFIX: &str = "approval:";

// ── Pending text-reply approvals ─────────────────────────────────

/// Tracks a pending approval that awaits a text reply (for channels without buttons).
#[derive(Debug, Clone)]
struct PendingTextApproval {
    request_id: String,
}

/// Registry of pending text-reply approvals, keyed by (account_id, chat_id).
/// Only used for channels that don't support buttons.
static TEXT_PENDING: OnceLock<Mutex<HashMap<(String, String), Vec<PendingTextApproval>>>> =
    OnceLock::new();

fn get_text_pending(
) -> &'static Mutex<HashMap<(String, String), Vec<PendingTextApproval>>> {
    TEXT_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── InlineButton helper ──────────────────────────────────────────

impl InlineButton {
    /// Returns the effective callback identifier: `callback_data` if set, otherwise `text`.
    pub fn callback_id(&self) -> &str {
        self.callback_data.as_deref().unwrap_or(&self.text)
    }
}

// ── Approval button builder ──────────────────────────────────────

/// Build the standard 3-button row for approval prompts.
/// The `callback_data` format is `approval:{request_id}:{action}`.
pub(crate) fn build_approval_buttons(request_id: &str) -> Vec<Vec<InlineButton>> {
    vec![vec![
        InlineButton {
            text: "✅ Allow Once".to_string(),
            callback_data: Some(format!("{}{}:allow_once", APPROVAL_PREFIX, request_id)),
            url: None,
        },
        InlineButton {
            text: "🔓 Always Allow".to_string(),
            callback_data: Some(format!("{}{}:allow_always", APPROVAL_PREFIX, request_id)),
            url: None,
        },
        InlineButton {
            text: "❌ Deny".to_string(),
            callback_data: Some(format!("{}{}:deny", APPROVAL_PREFIX, request_id)),
            url: None,
        },
    ]]
}

/// Format the approval prompt text (plain text, no HTML — works across all channels).
fn format_approval_text(command: &str) -> String {
    let preview = crate::truncate_utf8(command, 500);
    format!("🔐 Tool approval required\n\n{}", preview)
}

/// Format the text-only approval prompt (for channels without buttons).
fn format_text_approval(command: &str) -> String {
    let preview = crate::truncate_utf8(command, 500);
    format!(
        "🔐 Tool approval required:\n{}\n\nReply:\n1 - Allow once\n2 - Always allow\n3 - Deny",
        preview
    )
}

// ── Shared callback handler (eliminates boilerplate in channel plugins) ──

/// Spawn a background task to handle an approval callback and log the result.
/// Used by channel plugins (Slack, Feishu, QQ Bot, LINE, Google Chat) that
/// don't need platform-specific post-processing after the approval.
pub fn spawn_callback_handler(data: &str, source: &'static str) {
    let data = data.to_string();
    tokio::spawn(async move {
        match handle_approval_callback(&data).await {
            Ok(label) => app_info!("channel", source, "Approval: {}", label),
            Err(e) => app_warn!("channel", source, "Approval failed: {}", e),
        }
    });
}

// ── EventBus listener ────────────────────────────────────────────

/// Spawn a background task that listens for `"approval_required"` events on
/// the EventBus and forwards them to the appropriate IM channel.
pub fn spawn_channel_approval_listener(
    channel_db: Arc<ChannelDB>,
    registry: Arc<ChannelRegistry>,
) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app_warn!(
                        "channel",
                        "approval",
                        "Approval listener lagged {} events",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

            if event.name != "approval_required" {
                continue;
            }

            // Deserialize the approval request
            let request: crate::tools::approval::ApprovalRequest =
                match serde_json::from_value(event.payload.clone()) {
                    Ok(r) => r,
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "approval",
                            "Failed to parse approval request: {}",
                            e
                        );
                        continue;
                    }
                };

            let Some(ref session_id) = request.session_id else {
                continue;
            };

            // Look up which channel conversation this session belongs to
            let conversation = match channel_db.get_conversation_by_session(session_id) {
                Ok(Some(conv)) => conv,
                Ok(None) => continue,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "approval",
                        "Failed to look up channel session {}: {}",
                        session_id,
                        e
                    );
                    continue;
                }
            };

            // Load account config
            let store = crate::provider::cached_store();
            let account_config = match store.channels.find_account(&conversation.account_id) {
                Some(c) => c.clone(),
                None => continue,
            };

            let channel_id: crate::channel::types::ChannelId =
                match serde_json::from_value(serde_json::Value::String(
                    conversation.channel_id.clone(),
                )) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

            let supports_buttons = registry
                .get_plugin(&channel_id)
                .map(|p| p.capabilities().supports_buttons)
                .unwrap_or(false);

            // Send the approval prompt to the IM channel
            let payload = if supports_buttons {
                ReplyPayload {
                    text: Some(format_approval_text(&request.command)),
                    buttons: build_approval_buttons(&request.request_id),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            } else {
                // Register for text-reply routing
                {
                    let key = (
                        conversation.account_id.clone(),
                        conversation.chat_id.clone(),
                    );
                    let mut pending = get_text_pending().lock().await;
                    pending
                        .entry(key)
                        .or_default()
                        .push(PendingTextApproval {
                            request_id: request.request_id.clone(),
                        });
                }

                ReplyPayload {
                    text: Some(format_text_approval(&request.command)),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            };

            if let Err(e) = registry
                .send_reply(&account_config, &conversation.chat_id, &payload)
                .await
            {
                app_warn!(
                    "channel",
                    "approval",
                    "Failed to send approval prompt to channel: {}",
                    e
                );
            }
        }
    });
}

// ── Text-reply approval handler ──────────────────────────────────

/// Try to handle an inbound message as an approval text reply.
///
/// Returns `true` if the message was consumed as an approval reply,
/// `false` if it should proceed through normal message processing.
pub async fn try_handle_approval_reply(
    msg: &crate::channel::types::MsgContext,
) -> bool {
    let text = match msg.text.as_deref() {
        Some(t) => t.trim(),
        None => return false,
    };

    let response = match text {
        "1" => ApprovalResponse::AllowOnce,
        "2" => ApprovalResponse::AllowAlways,
        "3" => ApprovalResponse::Deny,
        _ => return false,
    };

    let key = (msg.account_id.clone(), msg.chat_id.clone());
    let request_id = {
        let mut pending = get_text_pending().lock().await;
        if let Some(list) = pending.get_mut(&key) {
            if list.is_empty() {
                return false;
            }
            // LIFO: the most recent approval gets the reply
            let entry = list.pop().unwrap();
            if list.is_empty() {
                pending.remove(&key);
            }
            entry.request_id
        } else {
            return false;
        }
    };

    match submit_approval_response(&request_id, response).await {
        Ok(()) => true,
        Err(e) => {
            // Approval already expired (5-min timeout) — don't consume the message
            app_warn!(
                "channel",
                "approval",
                "Approval expired or invalid ({}), passing message through",
                e
            );
            false
        }
    }
}

// ── Callback approval handler (for button-based channels) ────────

/// Parse an approval callback string and submit the response.
///
/// `callback_data` format: `approval:{request_id}:{action}`
/// where action is one of: `allow_once`, `allow_always`, `deny`.
///
/// Returns `Ok(response_label)` on success for UI feedback, or `Err` on failure.
pub async fn handle_approval_callback(callback_data: &str) -> anyhow::Result<&'static str> {
    let rest = callback_data
        .strip_prefix(APPROVAL_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an approval callback"))?;

    let (request_id, action) = rest
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("Invalid approval callback format"))?;

    let (response, label) = match action {
        "allow_once" => (ApprovalResponse::AllowOnce, "✅ Allowed (once)"),
        "allow_always" => (ApprovalResponse::AllowAlways, "🔓 Always allowed"),
        "deny" => (ApprovalResponse::Deny, "❌ Denied"),
        _ => return Err(anyhow::anyhow!("Unknown approval action: {}", action)),
    };

    submit_approval_response(request_id, response).await?;
    Ok(label)
}

/// Check if a callback data string is an approval callback.
pub fn is_approval_callback(data: &str) -> bool {
    data.starts_with(APPROVAL_PREFIX)
}
