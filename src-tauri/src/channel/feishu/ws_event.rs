use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::channel::types::*;
use crate::channel::ws;

use super::api::FeishuApi;

/// Maximum number of consecutive reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: usize = 50;

// ── Event deserialization types ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct FeishuWsEvent {
    #[serde(default)]
    header: Option<EventHeader>,
    #[serde(default)]
    event: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct EventHeader {
    event_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageReceiveEvent {
    sender: Option<SenderInfo>,
    message: Option<MessageInfo>,
}

#[derive(Debug, Deserialize)]
struct SenderInfo {
    sender_id: Option<SenderIdInfo>,
    #[allow(dead_code)]
    sender_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SenderIdInfo {
    open_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageInfo {
    message_id: Option<String>,
    chat_id: Option<String>,
    chat_type: Option<String>,
    content: Option<String>,
    #[allow(dead_code)]
    message_type: Option<String>,
    #[serde(default)]
    mentions: Option<Vec<MentionInfo>>,
}

#[derive(Debug, Deserialize)]
struct MentionInfo {
    id: Option<MentionId>,
}

#[derive(Debug, Deserialize)]
struct MentionId {
    open_id: Option<String>,
}

/// Content payload for text messages.
#[derive(Debug, Deserialize)]
struct TextContent {
    text: Option<String>,
}

/// Run the Feishu WebSocket gateway event loop.
///
/// Connects to Feishu's long-connection WebSocket endpoint and listens for
/// inbound events (primarily `im.message.receive_v1`). Automatically reconnects
/// with exponential backoff on disconnection.
pub async fn run_feishu_gateway(
    api: Arc<FeishuApi>,
    account_id: String,
    bot_open_id: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    let mut reconnect_attempts: usize = 0;

    loop {
        if cancel.is_cancelled() {
            app_info!(
                "channel",
                "feishu:gateway",
                "[{}] Gateway shutdown requested",
                account_id
            );
            return;
        }

        // 1. Obtain WebSocket endpoint URL
        let ws_url = match api.get_ws_endpoint().await {
            Ok(url) => {
                reconnect_attempts = 0;
                url
            }
            Err(e) => {
                reconnect_attempts += 1;
                if reconnect_attempts > MAX_RECONNECT_ATTEMPTS {
                    app_error!(
                        "channel",
                        "feishu:gateway",
                        "[{}] Exceeded max reconnect attempts ({}), giving up: {}",
                        account_id,
                        MAX_RECONNECT_ATTEMPTS,
                        e
                    );
                    return;
                }
                let backoff = ws::backoff_duration(reconnect_attempts.saturating_sub(1));
                app_warn!(
                    "channel",
                    "feishu:gateway",
                    "[{}] Failed to get WS endpoint (attempt {}): {}. Retrying in {:?}",
                    account_id,
                    reconnect_attempts,
                    e,
                    backoff
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => continue,
                    _ = cancel.cancelled() => return,
                }
            }
        };

        app_info!(
            "channel",
            "feishu:gateway",
            "[{}] Connecting to WebSocket endpoint",
            account_id
        );

        // 2. Connect to WebSocket
        let mut conn = match ws::WsConnection::connect(&ws_url).await {
            Ok(c) => c,
            Err(e) => {
                reconnect_attempts += 1;
                if reconnect_attempts > MAX_RECONNECT_ATTEMPTS {
                    app_error!(
                        "channel",
                        "feishu:gateway",
                        "[{}] Exceeded max reconnect attempts after WS connect failure, giving up",
                        account_id
                    );
                    return;
                }
                let backoff = ws::backoff_duration(reconnect_attempts.saturating_sub(1));
                app_warn!(
                    "channel",
                    "feishu:gateway",
                    "[{}] WebSocket connect failed (attempt {}): {}. Retrying in {:?}",
                    account_id,
                    reconnect_attempts,
                    e,
                    backoff
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => continue,
                    _ = cancel.cancelled() => return,
                }
            }
        };

        app_info!(
            "channel",
            "feishu:gateway",
            "[{}] WebSocket connected, listening for events",
            account_id
        );
        reconnect_attempts = 0;

        // 3. Event receive loop
        loop {
            tokio::select! {
                msg = conn.recv_text() => {
                    match msg {
                        Some(text) => {
                            if let Err(e) = handle_ws_message(
                                &text,
                                &account_id,
                                &bot_open_id,
                                &inbound_tx,
                            ).await {
                                app_debug!(
                                    "channel",
                                    "feishu:gateway",
                                    "[{}] Error handling WS message: {}",
                                    account_id,
                                    e
                                );
                            }
                        }
                        None => {
                            // Connection closed
                            app_warn!(
                                "channel",
                                "feishu:gateway",
                                "[{}] WebSocket connection closed, will reconnect",
                                account_id
                            );
                            break;
                        }
                    }
                }
                _ = cancel.cancelled() => {
                    app_info!(
                        "channel",
                        "feishu:gateway",
                        "[{}] Shutdown requested, closing WebSocket",
                        account_id
                    );
                    conn.close().await;
                    return;
                }
            }
        }

        // Disconnected — reconnect after backoff
        reconnect_attempts += 1;
        if reconnect_attempts > MAX_RECONNECT_ATTEMPTS {
            app_error!(
                "channel",
                "feishu:gateway",
                "[{}] Exceeded max reconnect attempts ({}), giving up",
                account_id,
                MAX_RECONNECT_ATTEMPTS
            );
            return;
        }
        let backoff = ws::backoff_duration(reconnect_attempts.saturating_sub(1));
        app_warn!(
            "channel",
            "feishu:gateway",
            "[{}] Reconnecting in {:?} (attempt {})",
            account_id,
            backoff,
            reconnect_attempts
        );
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = cancel.cancelled() => return,
        }
    }
}

/// Handle a single WebSocket text message from Feishu.
async fn handle_ws_message(
    raw: &str,
    account_id: &str,
    bot_open_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) -> anyhow::Result<()> {
    let event: FeishuWsEvent = serde_json::from_str(raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse Feishu WS event: {}", e))?;

    let header = match event.header {
        Some(h) => h,
        None => return Ok(()), // Not a recognized event structure
    };

    let event_type = header.event_type.as_deref().unwrap_or("");

    match event_type {
        "im.message.receive_v1" => {
            if let Some(event_data) = event.event {
                handle_message_event(event_data, account_id, bot_open_id, inbound_tx).await?;
            }
        }
        _ => {
            app_debug!(
                "channel",
                "feishu:gateway",
                "[{}] Ignoring event type: {}",
                account_id,
                event_type
            );
        }
    }

    Ok(())
}

/// Process an `im.message.receive_v1` event and forward as MsgContext.
async fn handle_message_event(
    event_data: serde_json::Value,
    account_id: &str,
    bot_open_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) -> anyhow::Result<()> {
    let evt: MessageReceiveEvent = serde_json::from_value(event_data.clone())
        .map_err(|e| anyhow::anyhow!("Failed to parse message receive event: {}", e))?;

    let sender = evt
        .sender
        .ok_or_else(|| anyhow::anyhow!("Missing sender in message event"))?;
    let message = evt
        .message
        .ok_or_else(|| anyhow::anyhow!("Missing message in message event"))?;

    let sender_id = sender
        .sender_id
        .and_then(|s| s.open_id)
        .unwrap_or_default();

    let chat_id = message.chat_id.unwrap_or_default();
    let message_id = message.message_id.unwrap_or_default();

    // Determine chat type: "p2p" → Dm, "group" → Group
    let chat_type = match message.chat_type.as_deref() {
        Some("p2p") => ChatType::Dm,
        Some("group") => ChatType::Group,
        _ => ChatType::Group, // Default to group for unknown types
    };

    // Parse text content from the message content JSON string
    let text = message.content.as_ref().and_then(|content_str| {
        serde_json::from_str::<TextContent>(content_str)
            .ok()
            .and_then(|tc| tc.text)
            .map(|t| clean_mention_tags(&t))
    });

    // Check if the bot was mentioned in this message
    let was_mentioned = message
        .mentions
        .as_ref()
        .map(|mentions| {
            mentions.iter().any(|m| {
                m.id.as_ref()
                    .and_then(|id| id.open_id.as_deref())
                    .map(|oid| oid == bot_open_id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let msg = MsgContext {
        channel_id: ChannelId::Feishu,
        account_id: account_id.to_string(),
        sender_id,
        sender_name: None,
        sender_username: None,
        chat_id,
        chat_type,
        chat_title: None,
        thread_id: None,
        message_id,
        text,
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp: chrono::Utc::now(),
        was_mentioned,
        raw: event_data,
    };

    if let Err(e) = inbound_tx.send(msg).await {
        app_warn!(
            "channel",
            "feishu:gateway",
            "[{}] Failed to send inbound message: {}",
            account_id,
            e
        );
    }

    Ok(())
}

/// Clean Feishu @mention placeholder tags from text.
///
/// Feishu uses `@_user_1`, `@_user_2`, etc. as placeholders for @mentions
/// in the text content. This function removes them to produce clean text.
fn clean_mention_tags(text: &str) -> String {
    let mut result = text.to_string();

    // Remove @_user_N patterns (Feishu mention placeholders)
    // These appear as `@_user_1` in the text
    loop {
        let before = result.clone();
        // Match @_user_N optionally followed by a space
        if let Some(start) = result.find("@_user_") {
            let rest = &result[start + 7..]; // skip "@_user_"
            // Find where the digits end
            let digit_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            if digit_end > 0 {
                let end = start + 7 + digit_end;
                // Also consume a trailing space if present
                let final_end = if result.as_bytes().get(end) == Some(&b' ') {
                    end + 1
                } else {
                    end
                };
                result = format!("{}{}", &result[..start], &result[final_end..]);
            }
        }
        if result == before {
            break;
        }
    }

    // Also handle @_all (mention everyone)
    result = result.replace("@_all ", "").replace("@_all", "");

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_mention_tags_single() {
        assert_eq!(clean_mention_tags("@_user_1 hello"), "hello");
    }

    #[test]
    fn test_clean_mention_tags_multiple() {
        assert_eq!(
            clean_mention_tags("@_user_1 @_user_2 hello world"),
            "hello world"
        );
    }

    #[test]
    fn test_clean_mention_tags_no_mention() {
        assert_eq!(clean_mention_tags("hello world"), "hello world");
    }

    #[test]
    fn test_clean_mention_tags_at_all() {
        assert_eq!(clean_mention_tags("@_all hello"), "hello");
    }

    #[test]
    fn test_clean_mention_tags_inline() {
        assert_eq!(
            clean_mention_tags("hey @_user_1 what's up"),
            "hey what's up"
        );
    }

    #[test]
    fn test_clean_mention_tags_end() {
        assert_eq!(clean_mention_tags("hello @_user_1"), "hello");
    }
}
