use std::sync::Arc;
use tokio::sync::mpsc;

use crate::channel::types::*;
use crate::channel::webhook_server::{WebhookHandlerFn, WebhookRequest, WebhookResponse};

use super::api::GoogleChatApi;

/// Create a webhook handler function for Google Chat events.
///
/// The handler receives incoming HTTP requests from Google Chat's webhook endpoint,
/// parses the event payload, and forwards inbound messages through `inbound_tx`.
pub fn create_webhook_handler(
    api: Arc<GoogleChatApi>,
    account_id: String,
    inbound_tx: mpsc::Sender<MsgContext>,
) -> WebhookHandlerFn {
    // Keep api alive for potential future use (e.g. downloading attachments)
    let _api = api;

    Arc::new(move |req: WebhookRequest| {
        let account_id = account_id.clone();
        let inbound_tx = inbound_tx.clone();

        Box::pin(async move {
            // Parse the JSON body
            let body: serde_json::Value = match serde_json::from_slice(&req.body) {
                Ok(v) => v,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "googlechat",
                        "Failed to parse webhook body: {}",
                        e
                    );
                    return WebhookResponse {
                        status: 400,
                        body: r#"{"text":"invalid payload"}"#.to_string(),
                    };
                }
            };

            // Extract event type
            let event_type = body
                .get("type")
                .or_else(|| body.get("eventType"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match event_type {
                "MESSAGE" => {
                    handle_message_event(&body, &account_id, &inbound_tx).await;
                }
                "ADDED_TO_SPACE" => {
                    let space_name = body
                        .pointer("/space/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let space_type = body
                        .pointer("/space/type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    app_info!(
                        "channel",
                        "googlechat",
                        "Bot added to space: {} (type={})",
                        space_name,
                        space_type
                    );
                }
                "REMOVED_FROM_SPACE" => {
                    let space_name = body
                        .pointer("/space/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    app_info!(
                        "channel",
                        "googlechat",
                        "Bot removed from space: {}",
                        space_name
                    );
                }
                "CARD_CLICKED" => {
                    if let Some(action) = body
                        .pointer("/action/actionMethodName")
                        .and_then(|v| v.as_str())
                    {
                        if crate::channel::worker::approval::is_approval_callback(action) {
                            crate::channel::worker::approval::spawn_callback_handler(
                                action,
                                "googlechat",
                            );
                        }
                    }
                }
                other => {
                    app_debug!(
                        "channel",
                        "googlechat",
                        "Ignoring event type: {}",
                        other
                    );
                }
            }

            // Google Chat expects a JSON response as acknowledgment
            WebhookResponse {
                status: 200,
                body: r#"{}"#.to_string(),
            }
        })
    })
}

/// Handle a MESSAGE event from Google Chat.
async fn handle_message_event(
    body: &serde_json::Value,
    account_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) {
    let message = match body.get("message") {
        Some(m) => m,
        None => {
            app_warn!(
                "channel",
                "googlechat",
                "MESSAGE event missing 'message' field"
            );
            return;
        }
    };

    let space = match body.get("space") {
        Some(s) => s,
        None => {
            app_warn!(
                "channel",
                "googlechat",
                "MESSAGE event missing 'space' field"
            );
            return;
        }
    };

    // Extract space info
    let space_name = space
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let space_type = space
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("ROOM");
    let space_display_name = space
        .get("displayName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Map space type to ChatType
    let chat_type = match space_type {
        "DM" => ChatType::Dm,
        _ => ChatType::Group, // ROOM, SPACE, etc.
    };

    // Extract sender info
    let sender = message.get("sender").unwrap_or(&serde_json::Value::Null);
    let sender_name_str = sender
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_display_name = sender
        .get("displayName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract message text — prefer argumentText (strips bot mention), fallback to text
    let text = message
        .get("argumentText")
        .and_then(|v| v.as_str())
        .or_else(|| message.get("text").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string());

    // Extract message ID
    let message_id = message
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Extract thread ID
    let thread_id = message
        .pointer("/thread/name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Check if bot was mentioned
    let was_mentioned = check_bot_mentioned(message);

    // Parse timestamp
    let timestamp = body
        .get("eventTime")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let msg = MsgContext {
        channel_id: ChannelId::GoogleChat,
        account_id: account_id.to_string(),
        sender_id: sender_name_str,
        sender_name: sender_display_name,
        sender_username: None,
        chat_id: space_name,
        chat_type,
        chat_title: space_display_name,
        thread_id,
        message_id,
        text,
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned,
        raw: body.clone(),
    };

    if let Err(e) = inbound_tx.send(msg).await {
        app_error!(
            "channel",
            "googlechat",
            "Failed to send inbound message: {}",
            e
        );
    }
}

/// Check if the bot was @mentioned in the message by inspecting annotations.
fn check_bot_mentioned(message: &serde_json::Value) -> bool {
    let annotations = match message.get("annotations") {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return false,
    };

    annotations.iter().any(|ann| {
        ann.get("type")
            .and_then(|v| v.as_str())
            .map(|t| t == "USER_MENTION")
            .unwrap_or(false)
            && ann
                .pointer("/userMention/type")
                .and_then(|v| v.as_str())
                .map(|t| t == "BOT")
                .unwrap_or(false)
    })
}
