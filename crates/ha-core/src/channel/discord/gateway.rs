use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::api::DiscordApi;
use crate::channel::types::*;
use crate::channel::ws::{backoff_duration, WsConnection};

// ── Discord Gateway Opcodes ─────────────────────────────────────

const OP_DISPATCH: u64 = 0;
const OP_HEARTBEAT: u64 = 1;
const OP_IDENTIFY: u64 = 2;
const OP_RESUME: u64 = 6;
const OP_RECONNECT: u64 = 7;
const OP_INVALID_SESSION: u64 = 9;
const OP_HELLO: u64 = 10;
const OP_HEARTBEAT_ACK: u64 = 11;

/// Discord Gateway Intents bitmask.
/// GUILDS(1<<0) | GUILD_MESSAGES(1<<9) | DIRECT_MESSAGES(1<<12) | MESSAGE_CONTENT(1<<15)
const GATEWAY_INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);

const MAX_RECONNECT_ATTEMPTS: usize = 50;

/// Run the Discord gateway WebSocket loop.
///
/// Connects to the Discord gateway, handles heartbeating, dispatches events,
/// and automatically reconnects with RESUME or full re-IDENTIFY on failures.
pub async fn run_gateway_loop(
    api: Arc<DiscordApi>,
    account_id: String,
    bot_id: String,
    bot_username: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    app_info!(
        "channel",
        "discord::gateway",
        "Gateway loop started for account '{}'",
        account_id
    );

    let mut reconnect_attempts: usize = 0;
    let mut session_id: Option<String> = None;
    let mut resume_gateway_url: Option<String> = None;
    let mut last_seq: Option<u64> = None;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
            app_error!(
                "channel",
                "discord::gateway",
                "Max reconnect attempts ({}) reached for account '{}', giving up",
                MAX_RECONNECT_ATTEMPTS,
                account_id
            );
            break;
        }

        // Backoff before reconnect (skip on first attempt)
        if reconnect_attempts > 0 {
            let delay = backoff_duration(reconnect_attempts.saturating_sub(1));
            app_info!(
                "channel",
                "discord::gateway",
                "Reconnecting in {:?} (attempt {}/{})",
                delay,
                reconnect_attempts,
                MAX_RECONNECT_ATTEMPTS
            );
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(delay) => {}
            }
        }

        // Determine WSS URL
        let gateway_url =
            if let (Some(ref resume_url), Some(_)) = (&resume_gateway_url, &session_id) {
                // Use resume URL if we have a session to resume
                format!("{}?v=10&encoding=json", resume_url)
            } else {
                // Get fresh gateway URL from REST API
                match api.get_gateway_bot().await {
                    Ok(info) => {
                        let url = info["url"].as_str().unwrap_or("wss://gateway.discord.gg");
                        format!("{}?v=10&encoding=json", url)
                    }
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "discord::gateway",
                            "Failed to get gateway URL: {}, using default",
                            e
                        );
                        "wss://gateway.discord.gg?v=10&encoding=json".to_string()
                    }
                }
            };

        // Connect
        let mut ws = match WsConnection::connect(&gateway_url).await {
            Ok(ws) => {
                app_info!(
                    "channel",
                    "discord::gateway",
                    "Connected to gateway for account '{}'",
                    account_id
                );
                ws
            }
            Err(e) => {
                app_warn!(
                    "channel",
                    "discord::gateway",
                    "Failed to connect to gateway: {}",
                    e
                );
                reconnect_attempts += 1;
                continue;
            }
        };

        // Receive HELLO (op=10)
        let heartbeat_interval_ms = match recv_hello(&mut ws, &cancel).await {
            Some(interval) => interval,
            None => {
                app_warn!(
                    "channel",
                    "discord::gateway",
                    "Did not receive HELLO, reconnecting"
                );
                ws.close().await;
                reconnect_attempts += 1;
                continue;
            }
        };

        app_debug!(
            "channel",
            "discord::gateway",
            "Received HELLO, heartbeat_interval={}ms",
            heartbeat_interval_ms
        );

        // Decide whether to RESUME or IDENTIFY
        let identify_success = if let (Some(ref sid), Some(seq)) = (&session_id, last_seq) {
            // Try RESUME
            let resume_payload = serde_json::json!({
                "op": OP_RESUME,
                "d": {
                    "token": api.token(),
                    "session_id": sid,
                    "seq": seq
                }
            });
            if let Err(e) = ws.send_json(&resume_payload).await {
                app_warn!(
                    "channel",
                    "discord::gateway",
                    "Failed to send RESUME: {}",
                    e
                );
                false
            } else {
                app_info!(
                    "channel",
                    "discord::gateway",
                    "Sent RESUME for session '{}'",
                    sid
                );
                true // We'll handle INVALID_SESSION in the main loop
            }
        } else {
            // Full IDENTIFY
            send_identify(&mut ws, api.token()).await
        };

        if !identify_success {
            app_warn!(
                "channel",
                "discord::gateway",
                "Failed to send IDENTIFY/RESUME, reconnecting"
            );
            ws.close().await;
            reconnect_attempts += 1;
            // Clear session on failed identify to force fresh connection
            session_id = None;
            resume_gateway_url = None;
            last_seq = None;
            continue;
        }

        // Heartbeat is integrated into the main loop via tokio::select! since
        // WsConnection doesn't support concurrent access (no Clone).
        let heartbeat_seq = Arc::new(tokio::sync::Mutex::new(last_seq));

        // Main message loop with integrated heartbeat
        let mut last_heartbeat = tokio::time::Instant::now();
        let mut heartbeat_acked = true;
        let heartbeat_duration = tokio::time::Duration::from_millis(heartbeat_interval_ms);
        // When breaking out of the inner loop, we always want to reconnect.
        // invalidate_session is set when the session cannot be resumed.
        let mut invalidate_session = false;

        loop {
            let time_until_heartbeat = heartbeat_duration
                .checked_sub(last_heartbeat.elapsed())
                .unwrap_or(tokio::time::Duration::ZERO);

            tokio::select! {
                _ = cancel.cancelled() => {
                    app_info!(
                        "channel",
                        "discord::gateway",
                        "Gateway cancelled for account '{}'",
                        account_id
                    );
                    ws.close().await;
                    return;
                }

                _ = tokio::time::sleep(time_until_heartbeat) => {
                    // Time to send heartbeat
                    if !heartbeat_acked {
                        app_warn!(
                            "channel",
                            "discord::gateway",
                            "Heartbeat not ACKed, zombie connection detected, reconnecting"
                        );
                        break;
                    }

                    let seq = { *heartbeat_seq.lock().await };
                    let hb = serde_json::json!({ "op": OP_HEARTBEAT, "d": seq });
                    if let Err(e) = ws.send_json(&hb).await {
                        app_warn!(
                            "channel",
                            "discord::gateway",
                            "Failed to send heartbeat: {}",
                            e
                        );
                        break;
                    }
                    heartbeat_acked = false;
                    last_heartbeat = tokio::time::Instant::now();
                }

                msg = ws.recv_text_with_close() => {
                    let text = match msg {
                        Some(Ok(t)) => t,
                        Some(Err(close)) => {
                            // Discord gateway close codes:
                            // - 4004 authentication failed (token invalid)
                            // - 4010 invalid shard
                            // - 4011 sharding required
                            // - 4012 invalid API version
                            // - 4013 invalid intents
                            // - 4014 disallowed intents (privileged not enabled)
                            // 这些是 fatal，不应该 RESUME / IDENTIFY 反复重试
                            // - 4007 invalid seq
                            // - 4009 session timeout
                            // 这两个清 session 后 fresh IDENTIFY
                            // - 其它（4000-4003 / 4005 / 4008）允许 RESUME
                            // <https://discord.com/developers/docs/topics/opcodes-and-status-codes#gateway>
                            match close.code {
                                4004 | 4010 | 4011 | 4012 | 4013 | 4014 => {
                                    app_error!(
                                        "channel",
                                        "discord::gateway",
                                        "Fatal close code {} ({}); aborting account '{}'",
                                        close.code,
                                        close.reason,
                                        account_id
                                    );
                                    return;
                                }
                                4007 | 4009 => {
                                    app_warn!(
                                        "channel",
                                        "discord::gateway",
                                        "Recoverable close code {} ({}); fresh IDENTIFY",
                                        close.code,
                                        close.reason
                                    );
                                    invalidate_session = true;
                                }
                                _ => {
                                    app_warn!(
                                        "channel",
                                        "discord::gateway",
                                        "WebSocket closed (code={}, reason='{}'), reconnecting",
                                        close.code,
                                        close.reason
                                    );
                                }
                            }
                            break;
                        }
                        None => {
                            app_warn!(
                                "channel",
                                "discord::gateway",
                                "WebSocket closed without close frame, reconnecting"
                            );
                            break;
                        }
                    };

                    let payload: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            app_warn!(
                                "channel",
                                "discord::gateway",
                                "Failed to parse gateway message: {}",
                                e
                            );
                            continue;
                        }
                    };

                    let op = payload["op"].as_u64().unwrap_or(u64::MAX);

                    // Track sequence number from dispatches
                    if let Some(s) = payload["s"].as_u64() {
                        last_seq = Some(s);
                        *heartbeat_seq.lock().await = Some(s);
                    }

                    match op {
                        OP_DISPATCH => {
                            let event_name = payload["t"].as_str().unwrap_or("");
                            let d = &payload["d"];

                            match event_name {
                                "READY" => {
                                    session_id = d["session_id"].as_str().map(|s| s.to_string());
                                    resume_gateway_url = d["resume_gateway_url"]
                                        .as_str()
                                        .map(|s| s.to_string());
                                    reconnect_attempts = 0;
                                    app_info!(
                                        "channel",
                                        "discord::gateway",
                                        "READY: session_id={:?}",
                                        session_id
                                    );
                                }
                                "RESUMED" => {
                                    reconnect_attempts = 0;
                                    app_info!(
                                        "channel",
                                        "discord::gateway",
                                        "Successfully resumed session"
                                    );
                                }
                                "MESSAGE_CREATE" => {
                                    if let Some(ctx) = convert_message_create(
                                        d,
                                        &account_id,
                                        &bot_id,
                                        &bot_username,
                                    ) {
                                        if let Err(e) = inbound_tx.send(ctx).await {
                                            app_error!(
                                                "channel",
                                                "discord::gateway",
                                                "Failed to send inbound message: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                                "INTERACTION_CREATE" => {
                                    // Check for component interaction (type=3) — button clicks
                                    let interaction_type = d["type"].as_u64().unwrap_or(0);
                                    if interaction_type == 3 {
                                        // Message component interaction
                                        let custom_id = d.get("data")
                                            .and_then(|data| data.get("custom_id"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");

                                        if crate::channel::worker::approval::is_approval_callback(custom_id) {
                                            let api_clone = api.clone();
                                            let interaction_id = d["id"].as_str().unwrap_or("").to_string();
                                            let interaction_token = d["token"].as_str().unwrap_or("").to_string();
                                            let custom_id_owned = custom_id.to_string();

                                            tokio::spawn(async move {
                                                handle_approval_component(
                                                    &api_clone,
                                                    &interaction_id,
                                                    &interaction_token,
                                                    &custom_id_owned,
                                                ).await;
                                            });
                                        } else if crate::channel::worker::ask_user::is_ask_user_callback(custom_id) {
                                            // Dispatch ask_user callback (uses generic
                                            // spawn_callback_handler; Discord interaction
                                            // ack is best-effort via update_message below).
                                            crate::channel::worker::ask_user::spawn_callback_handler(
                                                custom_id,
                                                "discord::gateway",
                                            );
                                            // Acknowledge the interaction (type 6 = DEFERRED_UPDATE_MESSAGE)
                                            // so Discord doesn't show "interaction failed".
                                            ack_component_interaction(api.clone(), &d);
                                        } else if let Some(rest) = custom_id.strip_prefix("slash:") {
                                            // Ack first so the button doesn't render as failed.
                                            ack_component_interaction(api.clone(), &d);
                                            // Re-inject as synthetic inbound `/cmd arg`.
                                            let chat_id = d["channel_id"].as_str().unwrap_or("").to_string();
                                            let sender_id = d
                                                .get("member")
                                                .and_then(|m| m.get("user"))
                                                .or_else(|| d.get("user"))
                                                .and_then(|u| u.get("id"))
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let message_id = d["id"].as_str().unwrap_or("").to_string();
                                            let rest_owned = rest.to_string();
                                            let inbound_tx_clone = inbound_tx.clone();
                                            let account_id_clone = account_id.clone();
                                            tokio::spawn(async move {
                                                crate::channel::worker::slash_callback::inject_slash_callback(
                                                    ChannelId::Discord,
                                                    &account_id_clone,
                                                    &chat_id,
                                                    None,
                                                    &sender_id,
                                                    &message_id,
                                                    &rest_owned,
                                                    &inbound_tx_clone,
                                                    "discord::gateway",
                                                ).await;
                                            });
                                        }
                                        // Don't pass component interactions to convert_interaction
                                    } else if let Some(ctx) = convert_interaction(
                                        d,
                                        &account_id,
                                    ) {
                                        if let Err(e) = inbound_tx.send(ctx).await {
                                            app_error!(
                                                "channel",
                                                "discord::gateway",
                                                "Failed to send interaction: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    // Ignore other dispatch events
                                }
                            }
                        }
                        OP_HEARTBEAT => {
                            // Server requests immediate heartbeat
                            let seq = { *heartbeat_seq.lock().await };
                            let hb = serde_json::json!({ "op": OP_HEARTBEAT, "d": seq });
                            if let Err(e) = ws.send_json(&hb).await {
                                app_warn!(
                                    "channel",
                                    "discord::gateway",
                                    "Failed to respond to heartbeat request: {}",
                                    e
                                );
                                break;
                            }
                        }
                        OP_HEARTBEAT_ACK => {
                            heartbeat_acked = true;
                        }
                        OP_RECONNECT => {
                            app_info!(
                                "channel",
                                "discord::gateway",
                                "Received RECONNECT (op=7), reconnecting"
                            );
                            break;
                        }
                        OP_INVALID_SESSION => {
                            let resumable = payload["d"].as_bool().unwrap_or(false);
                            app_info!(
                                "channel",
                                "discord::gateway",
                                "Received INVALID_SESSION (resumable={})",
                                resumable
                            );
                            if !resumable {
                                invalidate_session = true;
                            }
                            break;
                        }
                        _ => {
                            app_debug!(
                                "channel",
                                "discord::gateway",
                                "Unhandled gateway op={}",
                                op
                            );
                        }
                    }
                }
            }
        }

        // Cleanup before reconnect (clean exits use `return` directly)
        ws.close().await;

        if invalidate_session {
            session_id = None;
            resume_gateway_url = None;
            last_seq = None;
        }

        reconnect_attempts += 1;
    }

    app_info!(
        "channel",
        "discord::gateway",
        "Gateway loop stopped for account '{}'",
        account_id
    );
}

/// Receive the HELLO message and extract heartbeat_interval.
async fn recv_hello(ws: &mut WsConnection, cancel: &CancellationToken) -> Option<u64> {
    // Wait up to 15 seconds for HELLO
    tokio::select! {
        _ = cancel.cancelled() => None,
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(15)) => {
            app_warn!("channel", "discord::gateway", "Timed out waiting for HELLO");
            None
        }
        msg = ws.recv_text() => {
            let text = msg?;
            let payload: serde_json::Value = serde_json::from_str(&text).ok()?;
            if payload["op"].as_u64() == Some(OP_HELLO) {
                payload["d"]["heartbeat_interval"].as_u64()
            } else {
                app_warn!(
                    "channel",
                    "discord::gateway",
                    "Expected HELLO but got op={}",
                    payload["op"]
                );
                None
            }
        }
    }
}

/// Send the IDENTIFY payload.
async fn send_identify(ws: &mut WsConnection, token: &str) -> bool {
    let identify = serde_json::json!({
        "op": OP_IDENTIFY,
        "d": {
            "token": token,
            "intents": GATEWAY_INTENTS,
            "properties": {
                "os": "macos",
                "browser": "hope-agent",
                "device": "hope-agent"
            }
        }
    });

    match ws.send_json(&identify).await {
        Ok(()) => {
            app_info!("channel", "discord::gateway", "Sent IDENTIFY");
            true
        }
        Err(e) => {
            app_warn!(
                "channel",
                "discord::gateway",
                "Failed to send IDENTIFY: {}",
                e
            );
            false
        }
    }
}

// ── Event Converters ────────────────────────────────────────────

/// Convert a Discord MESSAGE_CREATE dispatch event to MsgContext.
fn convert_message_create(
    d: &serde_json::Value,
    account_id: &str,
    bot_id: &str,
    bot_username: &str,
) -> Option<MsgContext> {
    let author = d.get("author")?;
    let author_id = author["id"].as_str()?;

    // Skip messages from the bot itself
    let is_bot = author["bot"].as_bool().unwrap_or(false);
    if is_bot && author_id == bot_id {
        return None;
    }

    let channel_id_str = d["channel_id"].as_str()?.to_string();
    let message_id = d["id"].as_str()?.to_string();

    // Determine chat type: guild_id present → Group, absent → Dm
    let chat_type = if d.get("guild_id").and_then(|v| v.as_str()).is_some() {
        ChatType::Group
    } else {
        ChatType::Dm
    };

    // Sender info
    let sender_id = author_id.to_string();
    let sender_name = author
        .get("global_name")
        .and_then(|v| v.as_str())
        .or_else(|| author.get("username").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let sender_username = author
        .get("username")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Text content
    let text = d.get("content").and_then(|v| v.as_str()).and_then(|s| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    });

    // Check if bot was mentioned
    let was_mentioned = if chat_type == ChatType::Dm {
        true
    } else {
        let mention_everyone = d["mention_everyone"].as_bool().unwrap_or(false);
        let mentioned_in_list = d
            .get("mentions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().any(|m| m["id"].as_str() == Some(bot_id)))
            .unwrap_or(false);

        // Also check if message text contains @username
        let text_mentions_bot = text
            .as_ref()
            .map(|t| {
                t.contains(&format!("<@{}>", bot_id)) || t.contains(&format!("<@!{}>", bot_id))
            })
            .unwrap_or(false);

        // Slash commands (text starts with /) are always "addressed"
        let is_command = text.as_ref().map(|t| t.starts_with('/')).unwrap_or(false);

        mention_everyone || mentioned_in_list || text_mentions_bot || is_command
    };

    // Parse timestamp (ISO 8601)
    let timestamp = d["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    // Reply-to
    let reply_to_message_id = d
        .get("message_reference")
        .and_then(|r| r.get("message_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Thread: if the message is in a thread, the channel_id IS the thread
    // We don't have parent info in MESSAGE_CREATE directly, so thread_id = None for now.
    // The worker can resolve this from the channel type if needed.
    let thread_id: Option<String> = None;

    // Media: parse attachments
    let mut media = Vec::new();
    if let Some(attachments) = d.get("attachments").and_then(|v| v.as_array()) {
        for att in attachments {
            let file_id = att["id"].as_str().unwrap_or("").to_string();
            let file_url = att
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let mime_type = att
                .get("content_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_size = att.get("size").and_then(|v| v.as_u64());

            // Determine media type from MIME
            let media_type = match mime_type.as_deref() {
                Some(m) if m.starts_with("image/") => MediaType::Photo,
                Some(m) if m.starts_with("video/") => MediaType::Video,
                Some(m) if m.starts_with("audio/") => MediaType::Audio,
                _ => MediaType::Document,
            };

            media.push(InboundMedia {
                media_type,
                file_id,
                file_url,
                mime_type,
                file_size,
                caption: None,
            });
        }
    }

    // Skip if no text and no media
    if text.is_none() && media.is_empty() {
        return None;
    }

    // Strip bot mention from text for cleaner processing
    let clean_text = text.map(|t| {
        let t = t
            .replace(&format!("<@{}>", bot_id), "")
            .replace(&format!("<@!{}>", bot_id), "");
        // Also strip @username mention
        let mention = format!("@{}", bot_username);
        let t = t.replace(&mention, "");
        let trimmed = t.trim().to_string();
        if trimmed.is_empty() {
            // If stripping mention left nothing, return None-equivalent
            // but since we return String, use the trimmed form
            trimmed
        } else {
            trimmed
        }
    });

    // If text became empty after stripping mentions and there's no media, skip
    let final_text = clean_text.filter(|t| !t.is_empty());
    if final_text.is_none() && media.is_empty() {
        return None;
    }

    // Chat title: use guild name if available, otherwise None
    let chat_title = d.get("guild_id").and_then(|_| None::<String>); // Guild name not available in MESSAGE_CREATE

    Some(MsgContext {
        channel_id: ChannelId::Discord,
        account_id: account_id.to_string(),
        sender_id,
        sender_name,
        sender_username,
        chat_id: channel_id_str,
        chat_type,
        chat_title,
        thread_id,
        message_id,
        text: final_text,
        media,
        reply_to_message_id,
        timestamp,
        was_mentioned,
        raw: d.clone(),
    })
}

/// Fire-and-forget ACK for a component interaction (button click) using
/// type=6 DEFERRED_UPDATE_MESSAGE so Discord doesn't display
/// "interaction failed". Used by the ask_user and slash callback paths;
/// the approval path uses type=7 UPDATE_MESSAGE to edit the message inline.
fn ack_component_interaction(api: Arc<DiscordApi>, d: &serde_json::Value) {
    let interaction_id = d["id"].as_str().unwrap_or("").to_string();
    let interaction_token = d["token"].as_str().unwrap_or("").to_string();
    if interaction_id.is_empty() || interaction_token.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let _ = api
            .create_interaction_response(&interaction_id, &interaction_token, 6, None)
            .await;
    });
}

/// Handle an approval button component interaction: submit the approval response
/// and update the original message to show the result.
async fn handle_approval_component(
    api: &DiscordApi,
    interaction_id: &str,
    interaction_token: &str,
    custom_id: &str,
) {
    // Handle the approval
    let result_text =
        match crate::channel::worker::approval::handle_approval_callback(custom_id).await {
            Ok(label) => label.to_string(),
            Err(e) => format!("Error: {}", e),
        };

    // Respond with UPDATE_MESSAGE (type=7) to edit the original message
    // and remove the buttons
    let response_data = serde_json::json!({
        "content": result_text,
        "components": []  // Remove buttons
    });

    if let Err(e) = api
        .create_interaction_response(interaction_id, interaction_token, 7, Some(response_data))
        .await
    {
        app_warn!(
            "channel",
            "discord::gateway",
            "Failed to respond to approval interaction: {}",
            e
        );
    }
}

/// Convert a Discord INTERACTION_CREATE event (slash command) to a synthetic MsgContext.
fn convert_interaction(d: &serde_json::Value, account_id: &str) -> Option<MsgContext> {
    // Only handle application command interactions (type=2)
    let interaction_type = d["type"].as_u64()?;
    if interaction_type != 2 {
        return None;
    }

    let interaction_id = d["id"].as_str()?.to_string();
    let channel_id_str = d["channel_id"].as_str()?.to_string();

    // Command data
    let data = d.get("data")?;
    let command_name = data["name"].as_str()?;

    // Build slash command text: "/command arg1 arg2 ..."
    let mut text = format!("/{}", command_name);
    if let Some(options) = data.get("options").and_then(|v| v.as_array()) {
        for opt in options {
            if let Some(value) = opt.get("value") {
                match value {
                    serde_json::Value::String(s) => {
                        text.push(' ');
                        text.push_str(s);
                    }
                    serde_json::Value::Number(n) => {
                        text.push(' ');
                        text.push_str(&n.to_string());
                    }
                    serde_json::Value::Bool(b) => {
                        text.push(' ');
                        text.push_str(&b.to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    // Sender info — interactions use "member.user" in guilds, "user" in DMs
    let user = d
        .get("member")
        .and_then(|m| m.get("user"))
        .or_else(|| d.get("user"))?;

    let sender_id = user["id"].as_str()?.to_string();
    let sender_name = user
        .get("global_name")
        .and_then(|v| v.as_str())
        .or_else(|| user.get("username").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let sender_username = user
        .get("username")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let chat_type = if d.get("guild_id").and_then(|v| v.as_str()).is_some() {
        ChatType::Group
    } else {
        ChatType::Dm
    };

    Some(MsgContext {
        channel_id: ChannelId::Discord,
        account_id: account_id.to_string(),
        sender_id,
        sender_name,
        sender_username,
        chat_id: channel_id_str,
        chat_type,
        chat_title: None,
        thread_id: None,
        message_id: interaction_id,
        text: Some(text),
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp: chrono::Utc::now(),
        was_mentioned: true, // Slash commands are always directed at the bot
        raw: d.clone(),
    })
}
