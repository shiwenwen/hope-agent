use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::channel::types::*;
use crate::channel::webhook_server::{WebhookHandlerFn, WebhookRequest, WebhookResponse};

use super::api::LineApi;

/// Shared store for reply tokens. Keys are chat_id, values are (reply_token, timestamp).
/// Reply tokens expire after ~1 minute, so we store the insertion time for cleanup.
pub type ReplyTokenStore = Arc<Mutex<HashMap<String, (String, std::time::Instant)>>>;

/// Create a new reply token store.
pub fn new_reply_token_store() -> ReplyTokenStore {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Verify the X-Line-Signature header using HMAC-SHA256.
///
/// Computes HMAC-SHA256 of the request body using channel_secret as the key,
/// base64-encodes the result, and compares with the provided signature.
pub fn verify_signature(body: &[u8], signature: &str, channel_secret: &str) -> bool {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let Ok(mut mac) = HmacSha256::new_from_slice(channel_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let result = mac.finalize();
    let computed = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

    // Constant-time comparison: compare byte-by-byte without short-circuit.
    // Both are base64 strings so ASCII-only.
    let a = computed.as_bytes();
    let b = signature.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Create a webhook handler function for LINE webhook events.
///
/// This handler verifies the signature, parses the webhook body,
/// and dispatches events to the inbound message channel.
pub fn create_webhook_handler(
    api: Arc<LineApi>,
    account_id: String,
    channel_secret: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    reply_tokens: ReplyTokenStore,
) -> WebhookHandlerFn {
    Arc::new(move |req: WebhookRequest| {
        let api = api.clone();
        let account_id = account_id.clone();
        let channel_secret = channel_secret.clone();
        let inbound_tx = inbound_tx.clone();
        let reply_tokens = reply_tokens.clone();

        Box::pin(async move {
            // Extract X-Line-Signature header
            let signature = req
                .headers
                .get("x-line-signature")
                .cloned()
                .unwrap_or_default();

            if signature.is_empty() {
                app_warn!("channel", "line", "Webhook missing X-Line-Signature header");
                return WebhookResponse {
                    status: 400,
                    body: r#"{"error":"Missing X-Line-Signature header"}"#.to_string(),
                };
            }

            // Verify signature
            if !verify_signature(&req.body, &signature, &channel_secret) {
                app_warn!("channel", "line", "Webhook signature verification failed");
                return WebhookResponse {
                    status: 403,
                    body: r#"{"error":"Invalid signature"}"#.to_string(),
                };
            }

            // Parse body
            let body: serde_json::Value = match serde_json::from_slice(&req.body) {
                Ok(v) => v,
                Err(e) => {
                    app_warn!("channel", "line", "Failed to parse webhook body: {}", e);
                    return WebhookResponse {
                        status: 400,
                        body: r#"{"error":"Invalid JSON"}"#.to_string(),
                    };
                }
            };

            let events = match body.get("events").and_then(|v| v.as_array()) {
                Some(events) => events.clone(),
                None => {
                    // LINE sends a verification event with empty events array
                    app_debug!(
                        "channel",
                        "line",
                        "Webhook received empty events (verification)"
                    );
                    return WebhookResponse {
                        status: 200,
                        body: r#"{"status":"ok"}"#.to_string(),
                    };
                }
            };

            app_debug!(
                "channel",
                "line",
                "Received {} webhook events",
                events.len()
            );

            for event in &events {
                let event_type = event
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                match event_type {
                    "message" => {
                        handle_message_event(event, &api, &account_id, &inbound_tx, &reply_tokens)
                            .await;
                    }
                    "follow" => {
                        let user_id = event
                            .pointer("/source/userId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        app_info!("channel", "line", "New follower: {}", user_id);
                    }
                    "unfollow" => {
                        let user_id = event
                            .pointer("/source/userId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        app_info!("channel", "line", "User unfollowed: {}", user_id);
                    }
                    "join" => {
                        let source_type = event
                            .pointer("/source/type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let source_id = if source_type == "group" {
                            event
                                .pointer("/source/groupId")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                        } else {
                            event
                                .pointer("/source/roomId")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                        };
                        app_info!(
                            "channel",
                            "line",
                            "Bot joined {} {}",
                            source_type,
                            source_id
                        );
                    }
                    "leave" => {
                        let source_type = event
                            .pointer("/source/type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let source_id = if source_type == "group" {
                            event
                                .pointer("/source/groupId")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                        } else {
                            event
                                .pointer("/source/roomId")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                        };
                        app_info!("channel", "line", "Bot left {} {}", source_type, source_id);
                    }
                    "postback" => {
                        if let Some(postback_data) =
                            event.pointer("/postback/data").and_then(|v| v.as_str())
                        {
                            crate::channel::worker::ask_user::try_dispatch_interactive_callback(
                                postback_data,
                                "line",
                            );
                        }
                    }
                    other => {
                        app_debug!("channel", "line", "Unhandled event type: {}", other);
                    }
                }
            }

            WebhookResponse {
                status: 200,
                body: r#"{"status":"ok"}"#.to_string(),
            }
        })
    })
}

/// Handle a LINE message event and forward to inbound_tx.
async fn handle_message_event(
    event: &serde_json::Value,
    api: &LineApi,
    account_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
    reply_tokens: &ReplyTokenStore,
) {
    let message = match event.get("message") {
        Some(m) => m,
        None => return,
    };

    let msg_type = message
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Currently only handle text messages
    let text = if msg_type == "text" {
        message
            .get("text")
            .and_then(|v| v.as_str())
            .map(String::from)
    } else {
        // For non-text messages, set descriptive text
        Some(format!("[{}]", msg_type))
    };

    let message_id = message
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Extract source info
    let source = event.get("source").cloned().unwrap_or_default();
    let source_type = source
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("user");

    let user_id = source
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let (chat_type, chat_id) = match source_type {
        "group" => {
            let group_id = source
                .get("groupId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (ChatType::Group, group_id)
        }
        "room" => {
            let room_id = source
                .get("roomId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (ChatType::Group, room_id)
        }
        _ => {
            // DM: chat_id is the user's ID
            (ChatType::Dm, user_id.clone())
        }
    };

    // Store reply token (valid for ~1 minute)
    if let Some(reply_token) = event.get("replyToken").and_then(|v| v.as_str()) {
        let mut tokens = reply_tokens.lock().await;
        // Clean up expired tokens (older than 50 seconds to be safe)
        let now = std::time::Instant::now();
        tokens.retain(|_, (_, ts)| now.duration_since(*ts).as_secs() < 50);
        tokens.insert(chat_id.clone(), (reply_token.to_string(), now));
    }

    // Try to get sender display name
    let sender_name = if !user_id.is_empty() {
        match source_type {
            "group" => {
                // For group messages, try group member profile first
                let group_id = source.get("groupId").and_then(|v| v.as_str()).unwrap_or("");
                if !group_id.is_empty() {
                    match api.get_group_member_profile(group_id, &user_id).await {
                        Ok(profile) => profile
                            .get("displayName")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        Err(_) => {
                            // Fall back to regular profile
                            api.get_profile(&user_id).await.ok().and_then(|p| {
                                p.get("displayName")
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                            })
                        }
                    }
                } else {
                    None
                }
            }
            _ => api.get_profile(&user_id).await.ok().and_then(|p| {
                p.get("displayName")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            }),
        }
    } else {
        None
    };

    // Check if bot was @mentioned
    // LINE provides mention info in message.mention.mentionees
    let was_mentioned = message
        .get("mention")
        .and_then(|m| m.get("mentionees"))
        .and_then(|arr| arr.as_array())
        .map(|mentionees| {
            mentionees.iter().any(|m| {
                m.get("isSelf").and_then(|v| v.as_bool()).unwrap_or(false)
                    || m.get("type").and_then(|v| v.as_str()) == Some("all")
            })
        })
        .unwrap_or(false);

    // Parse timestamp
    let timestamp = event
        .get("timestamp")
        .and_then(|v| v.as_i64())
        .map(|ts| chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now))
        .unwrap_or_else(chrono::Utc::now);

    let msg_ctx = MsgContext {
        channel_id: ChannelId::Line,
        account_id: account_id.to_string(),
        sender_id: user_id.clone(),
        sender_name,
        sender_username: None,
        chat_id,
        chat_type,
        chat_title: None,
        thread_id: None,
        message_id,
        text,
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned,
        raw: event.clone(),
    };

    if let Err(e) = inbound_tx.send(msg_ctx).await {
        app_error!("channel", "line", "Failed to send inbound message: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_signature_valid() {
        // Test vector: known HMAC-SHA256 result
        let secret = "test_secret";
        let body = b"test body";

        // Compute expected signature
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let expected =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        assert!(verify_signature(body, &expected, secret));
    }

    #[test]
    fn test_verify_signature_invalid() {
        let secret = "test_secret";
        let body = b"test body";
        assert!(!verify_signature(body, "invalid_signature", secret));
    }

    #[test]
    fn test_verify_signature_wrong_secret() {
        let body = b"test body";

        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(b"correct_secret").unwrap();
        mac.update(body);
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        assert!(!verify_signature(body, &sig, "wrong_secret"));
    }
}
