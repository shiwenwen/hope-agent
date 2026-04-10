use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::api::SlackApi;
use crate::channel::types::*;
use crate::channel::ws::{backoff_duration, WsConnection};

/// Maximum reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: usize = 50;

/// Run the Slack Socket Mode event loop.
///
/// Socket Mode protocol:
/// 1. Call `apps.connections.open` with the app token to get a one-time WSS URL
/// 2. Connect to the URL via WebSocket
/// 3. Receive event envelopes, ACK each immediately, then process
/// 4. On disconnect, get a NEW URL (old URLs are single-use)
pub async fn run_socket_mode(
    api: Arc<SlackApi>,
    app_token: String,
    account_id: String,
    bot_id: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    app_info!(
        "channel",
        "slack::socket",
        "Socket Mode loop started for account '{}'",
        account_id
    );

    let mut reconnect_attempt: usize = 0;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        // 1. Get a fresh WebSocket URL
        let ws_url = match api.connections_open(&app_token).await {
            Ok(url) => {
                app_info!(
                    "channel",
                    "slack::socket",
                    "Obtained Socket Mode URL for account '{}'",
                    account_id
                );
                url
            }
            Err(e) => {
                app_error!(
                    "channel",
                    "slack::socket",
                    "Failed to open connection for account '{}': {}",
                    account_id,
                    e
                );
                if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
                    app_error!(
                        "channel",
                        "slack::socket",
                        "Max reconnect attempts reached for account '{}', stopping",
                        account_id
                    );
                    break;
                }
                let delay = backoff_duration(reconnect_attempt);
                reconnect_attempt += 1;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(delay) => continue,
                }
            }
        };

        // 2. Connect to WebSocket
        let mut ws = match WsConnection::connect(&ws_url).await {
            Ok(ws) => {
                app_info!(
                    "channel",
                    "slack::socket",
                    "WebSocket connected for account '{}'",
                    account_id
                );
                reconnect_attempt = 0;
                ws
            }
            Err(e) => {
                app_error!(
                    "channel",
                    "slack::socket",
                    "WebSocket connect failed for account '{}': {}",
                    account_id,
                    e
                );
                if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
                    app_error!(
                        "channel",
                        "slack::socket",
                        "Max reconnect attempts reached for account '{}', stopping",
                        account_id
                    );
                    break;
                }
                let delay = backoff_duration(reconnect_attempt);
                reconnect_attempt += 1;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(delay) => continue,
                }
            }
        };

        // 3. Main event loop
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    app_info!(
                        "channel",
                        "slack::socket",
                        "Socket Mode cancelled for account '{}'",
                        account_id
                    );
                    ws.close().await;
                    return;
                }
                msg = ws.recv_text() => {
                    match msg {
                        Some(text) => {
                            handle_envelope(
                                &mut ws,
                                &text,
                                &account_id,
                                &bot_id,
                                &inbound_tx,
                            ).await;
                        }
                        None => {
                            // Connection closed - need to reconnect with a NEW URL
                            app_warn!(
                                "channel",
                                "slack::socket",
                                "WebSocket disconnected for account '{}', reconnecting",
                                account_id
                            );
                            break;
                        }
                    }
                }
            }
        }

        // Reconnect with backoff
        if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
            app_error!(
                "channel",
                "slack::socket",
                "Max reconnect attempts reached for account '{}', stopping",
                account_id
            );
            break;
        }
        let delay = backoff_duration(reconnect_attempt);
        reconnect_attempt += 1;
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(delay) => {}
        }
    }

    app_info!(
        "channel",
        "slack::socket",
        "Socket Mode loop ended for account '{}'",
        account_id
    );
}

/// Handle a single Socket Mode envelope.
///
/// Each envelope has the shape:
/// ```json
/// {
///   "envelope_id": "xxx",
///   "type": "events_api" | "slash_commands" | "interactive",
///   "payload": { ... }
/// }
/// ```
///
/// We must ACK every envelope immediately by sending `{"envelope_id": "xxx"}`.
async fn handle_envelope(
    ws: &mut WsConnection,
    text: &str,
    account_id: &str,
    bot_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) {
    let envelope: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            app_warn!(
                "channel",
                "slack::socket",
                "Failed to parse envelope: {}",
                e
            );
            return;
        }
    };

    // ACK immediately
    if let Some(envelope_id) = envelope.get("envelope_id").and_then(|v| v.as_str()) {
        let ack = serde_json::json!({"envelope_id": envelope_id});
        if let Err(e) = ws.send_json(&ack).await {
            app_warn!(
                "channel",
                "slack::socket",
                "Failed to send ACK for envelope '{}': {}",
                crate::truncate_utf8(envelope_id, 50),
                e
            );
        }
    }

    let envelope_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match envelope_type {
        "events_api" => {
            if let Some(payload) = envelope.get("payload") {
                if let Some(event) = payload.get("event") {
                    handle_event(event, account_id, bot_id, inbound_tx).await;
                }
            }
        }
        "slash_commands" => {
            if let Some(payload) = envelope.get("payload") {
                handle_slash_command(payload, account_id, bot_id, inbound_tx).await;
            }
        }
        "interactive" => {
            if let Some(payload) = envelope.get("payload") {
                if let Some(actions) = payload.get("actions").and_then(|v| v.as_array()) {
                    for action in actions {
                        if let Some(action_id) = action.get("action_id").and_then(|v| v.as_str()) {
                            crate::channel::worker::ask_user::try_dispatch_interactive_callback(
                                action_id,
                                "slack::socket",
                            );
                        }
                    }
                }
            }
        }
        "hello" => {
            // Socket Mode hello message on connect - expected, no action needed
            app_debug!(
                "channel",
                "slack::socket",
                "Received hello for account '{}'",
                account_id
            );
        }
        "disconnect" => {
            // Slack signals that we should reconnect
            app_info!(
                "channel",
                "slack::socket",
                "Received disconnect signal for account '{}'",
                account_id
            );
        }
        other => {
            app_debug!(
                "channel",
                "slack::socket",
                "Unknown envelope type '{}' for account '{}'",
                other,
                account_id
            );
        }
    }
}

/// Handle a Slack Events API event.
async fn handle_event(
    event: &serde_json::Value,
    account_id: &str,
    bot_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "message" => {
            // Skip bot messages, message edits, and subtypes we don't handle
            if let Some(subtype) = event.get("subtype").and_then(|v| v.as_str()) {
                match subtype {
                    "bot_message" | "message_changed" | "message_deleted" | "channel_join"
                    | "channel_leave" => {
                        return;
                    }
                    _ => {}
                }
            }

            // Skip messages from our own bot
            if let Some(user) = event.get("user").and_then(|v| v.as_str()) {
                if user == bot_id {
                    return;
                }
            } else {
                // No user field - likely a bot or system message
                return;
            }

            if let Some(msg_ctx) = convert_slack_event(event, account_id, bot_id, false) {
                if let Err(e) = inbound_tx.send(msg_ctx).await {
                    app_warn!(
                        "channel",
                        "slack::socket",
                        "Failed to send inbound message: {}",
                        e
                    );
                }
            }
        }
        "app_mention" => {
            // Skip messages from our own bot
            if let Some(user) = event.get("user").and_then(|v| v.as_str()) {
                if user == bot_id {
                    return;
                }
            }

            if let Some(msg_ctx) = convert_slack_event(event, account_id, bot_id, true) {
                if let Err(e) = inbound_tx.send(msg_ctx).await {
                    app_warn!(
                        "channel",
                        "slack::socket",
                        "Failed to send inbound mention: {}",
                        e
                    );
                }
            }
        }
        _ => {
            app_debug!(
                "channel",
                "slack::socket",
                "Ignoring event type '{}' for account '{}'",
                event_type,
                account_id
            );
        }
    }
}

/// Handle a Slack slash command.
async fn handle_slash_command(
    payload: &serde_json::Value,
    account_id: &str,
    bot_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) {
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let user_id = payload
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let user_name = payload.get("user_name").and_then(|v| v.as_str());
    let channel_id = payload
        .get("channel_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Construct the full command text
    let full_text = if text.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, text)
    };

    let timestamp = chrono::Utc::now();
    let message_id = format!("slash_{}", timestamp.timestamp_millis());

    let msg_ctx = MsgContext {
        channel_id: ChannelId::Slack,
        account_id: account_id.to_string(),
        sender_id: user_id.to_string(),
        sender_name: user_name.map(|s| s.to_string()),
        sender_username: user_name.map(|s| s.to_string()),
        chat_id: channel_id.to_string(),
        chat_type: ChatType::Dm, // Slash commands are treated as DMs
        chat_title: None,
        thread_id: None,
        message_id,
        text: Some(full_text),
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned: true, // Slash commands always target the bot
        raw: payload.clone(),
    };

    // Ignore messages from our own bot
    if user_id == bot_id {
        return;
    }

    if let Err(e) = inbound_tx.send(msg_ctx).await {
        app_warn!(
            "channel",
            "slack::socket",
            "Failed to send slash command inbound: {}",
            e
        );
    }
}

/// Convert a Slack event JSON to a normalized MsgContext.
fn convert_slack_event(
    event: &serde_json::Value,
    account_id: &str,
    bot_id: &str,
    is_mention: bool,
) -> Option<MsgContext> {
    let user = event.get("user").and_then(|v| v.as_str())?;
    let channel = event.get("channel").and_then(|v| v.as_str())?;
    let ts = event.get("ts").and_then(|v| v.as_str())?;
    let text = event.get("text").and_then(|v| v.as_str());

    // Determine chat type from channel_type field
    let channel_type = event
        .get("channel_type")
        .and_then(|v| v.as_str())
        .unwrap_or("channel");
    let chat_type = match channel_type {
        "im" => ChatType::Dm,
        _ => ChatType::Group,
    };

    // Determine thread_ts: if present and different from ts, this is a threaded reply
    let thread_id = event
        .get("thread_ts")
        .and_then(|v| v.as_str())
        .filter(|&thread_ts| thread_ts != ts)
        .map(|s| s.to_string());

    // Check if bot was mentioned in the text (for non-app_mention events)
    let was_mentioned = is_mention
        || text
            .map(|t| t.contains(&format!("<@{}>", bot_id)))
            .unwrap_or(false);

    // Parse timestamp from Slack ts format ("1234567890.123456")
    let timestamp = parse_slack_ts(ts).unwrap_or_else(chrono::Utc::now);

    // Parse media from files array if present
    let media = parse_slack_files(event);

    Some(MsgContext {
        channel_id: ChannelId::Slack,
        account_id: account_id.to_string(),
        sender_id: user.to_string(),
        sender_name: None, // Would need users.info call - skip for now
        sender_username: None,
        chat_id: channel.to_string(),
        chat_type,
        chat_title: None,
        thread_id,
        message_id: ts.to_string(),
        text: text.map(|s| s.to_string()),
        media,
        reply_to_message_id: None,
        timestamp,
        was_mentioned,
        raw: event.clone(),
    })
}

/// Parse a Slack timestamp string ("1234567890.123456") into a DateTime.
fn parse_slack_ts(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // Slack timestamps are Unix seconds with microsecond decimal
    let secs_str = ts.split('.').next()?;
    let secs: i64 = secs_str.parse().ok()?;
    chrono::DateTime::from_timestamp(secs, 0)
}

/// Parse Slack `files` array from an event into InboundMedia.
fn parse_slack_files(event: &serde_json::Value) -> Vec<InboundMedia> {
    let files = match event.get("files").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    files
        .iter()
        .filter_map(|file| {
            let file_id = file.get("id").and_then(|v| v.as_str())?.to_string();
            let mime_type = file
                .get("mimetype")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_size = file.get("size").and_then(|v| v.as_u64());
            let file_url = file
                .get("url_private_download")
                .or_else(|| file.get("url_private"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // Determine media type from mimetype
            let media_type = if let Some(ref mime) = mime_type {
                if mime.starts_with("image/") {
                    MediaType::Photo
                } else if mime.starts_with("video/") {
                    MediaType::Video
                } else if mime.starts_with("audio/") {
                    MediaType::Audio
                } else {
                    MediaType::Document
                }
            } else {
                MediaType::Document
            };

            Some(InboundMedia {
                media_type,
                file_id,
                file_url,
                mime_type,
                file_size,
                caption: None,
            })
        })
        .collect()
}
