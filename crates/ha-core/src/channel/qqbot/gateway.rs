use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::api::QqBotApi;
use super::auth::format_auth_value;
use crate::channel::types::*;
use crate::channel::ws::{backoff_duration, WsConnection};

/// Maximum reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: usize = 50;

/// QQ Bot Gateway opcodes.
mod opcode {
    pub const DISPATCH: u64 = 0;
    pub const HEARTBEAT: u64 = 1;
    pub const IDENTIFY: u64 = 2;
    pub const RESUME: u64 = 6;
    pub const RECONNECT: u64 = 7;
    pub const INVALID_SESSION: u64 = 9;
    pub const HELLO: u64 = 10;
    pub const HEARTBEAT_ACK: u64 = 11;
}

/// QQ Bot Gateway intents.
///
/// - PUBLIC_GUILD_MESSAGES (1 << 30) = 1073741824
/// - DIRECT_MESSAGE (1 << 12) = 4096
/// - GROUP_AND_C2C (1 << 25) = 33554432
const GATEWAY_INTENTS: u64 = (1 << 30) | (1 << 12) | (1 << 25);

/// Session file for resume support.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SessionFile {
    session_id: String,
    seq: u64,
}

/// Load saved session from disk for resume.
fn load_session(account_id: &str) -> Option<SessionFile> {
    let dir = crate::paths::channel_dir("qqbot").ok()?;
    let path = dir.join(format!("{}.session.json", account_id));
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save session to disk for resume.
fn save_session(account_id: &str, session_id: &str, seq: u64) {
    if let Ok(dir) = crate::paths::channel_dir("qqbot") {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.session.json", account_id));
        let session = SessionFile {
            session_id: session_id.to_string(),
            seq,
        };
        if let Ok(data) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(&path, data);
        }
    }
}

/// Run the QQ Bot WebSocket gateway event loop.
///
/// QQ Bot Gateway protocol (similar to Discord):
/// 1. Get access token and gateway URL
/// 2. Connect via WebSocket
/// 3. Receive HELLO (op=10) with heartbeat_interval
/// 4. Send IDENTIFY (op=2) or RESUME (op=6)
/// 5. Receive READY (op=0, t="READY") on identify, or RESUMED on resume
/// 6. Start heartbeat loop, process dispatch events
pub async fn run_qq_gateway(
    api: Arc<QqBotApi>,
    account_id: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    app_info!(
        "channel",
        "qqbot::gateway",
        "Gateway loop started for account '{}'",
        account_id
    );

    let mut reconnect_attempt: usize = 0;
    let mut saved_session_id: Option<String> = None;
    let mut last_seq: u64 = 0;

    // Try to load saved session for resume
    if let Some(session) = load_session(&account_id) {
        saved_session_id = Some(session.session_id);
        last_seq = session.seq;
        app_info!(
            "channel",
            "qqbot::gateway",
            "Loaded saved session for account '{}' (seq={})",
            account_id,
            last_seq
        );
    }

    loop {
        if cancel.is_cancelled() {
            break;
        }

        // 1. Get access token
        let token = match api.auth.get_token().await {
            Ok(t) => t,
            Err(e) => {
                app_error!(
                    "channel",
                    "qqbot::gateway",
                    "Failed to get access token for account '{}': {}",
                    account_id,
                    e
                );
                if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
                    app_error!(
                        "channel",
                        "qqbot::gateway",
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

        // 2. Get gateway URL
        let gateway_url = match api.get_gateway_url().await {
            Ok(url) => url,
            Err(e) => {
                app_error!(
                    "channel",
                    "qqbot::gateway",
                    "Failed to get gateway URL for account '{}': {}",
                    account_id,
                    e
                );
                if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
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

        app_info!(
            "channel",
            "qqbot::gateway",
            "Connecting to gateway for account '{}'",
            account_id
        );

        // 3. Connect via WebSocket
        let mut ws = match WsConnection::connect(&gateway_url).await {
            Ok(ws) => ws,
            Err(e) => {
                app_error!(
                    "channel",
                    "qqbot::gateway",
                    "WebSocket connect failed for account '{}': {}",
                    account_id,
                    e
                );
                if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
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

        // 4. Receive HELLO (op=10)
        let heartbeat_interval_ms = match recv_hello(&mut ws, &cancel).await {
            Some(interval) => {
                app_info!(
                    "channel",
                    "qqbot::gateway",
                    "Received HELLO for account '{}' (heartbeat_interval={}ms)",
                    account_id,
                    interval
                );
                interval
            }
            None => {
                app_error!(
                    "channel",
                    "qqbot::gateway",
                    "Failed to receive HELLO for account '{}', reconnecting",
                    account_id
                );
                ws.close().await;
                let delay = backoff_duration(reconnect_attempt);
                reconnect_attempt += 1;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(delay) => continue,
                }
            }
        };

        // 5. Send IDENTIFY or RESUME
        let mut use_resume = saved_session_id.is_some() && last_seq > 0;
        if let Some(session_id) = saved_session_id.as_ref().filter(|_| use_resume) {
            let resume_payload = serde_json::json!({
                "op": opcode::RESUME,
                "d": {
                    "token": format_auth_value(&token),
                    "session_id": session_id,
                    "seq": last_seq,
                }
            });
            if let Err(e) = ws.send_json(&resume_payload).await {
                app_warn!(
                    "channel",
                    "qqbot::gateway",
                    "Failed to send RESUME for account '{}': {}, falling back to IDENTIFY",
                    account_id,
                    e
                );
                use_resume = false;
            }
        }

        if !use_resume {
            let identify_payload = serde_json::json!({
                "op": opcode::IDENTIFY,
                "d": {
                    "token": format_auth_value(&token),
                    "intents": GATEWAY_INTENTS,
                    "shard": [0, 1],
                    "properties": {}
                }
            });
            if let Err(e) = ws.send_json(&identify_payload).await {
                app_error!(
                    "channel",
                    "qqbot::gateway",
                    "Failed to send IDENTIFY for account '{}': {}",
                    account_id,
                    e
                );
                ws.close().await;
                let delay = backoff_duration(reconnect_attempt);
                reconnect_attempt += 1;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(delay) => continue,
                }
            }
        }

        // Reset reconnect counter on successful connection
        reconnect_attempt = 0;

        // 6. Start heartbeat and main event loop
        let heartbeat_cancel = CancellationToken::new();
        let heartbeat_token = heartbeat_cancel.clone();

        // Shared seq counter for heartbeat
        let seq_holder = Arc::new(tokio::sync::Mutex::new(last_seq));
        let _heartbeat_seq = seq_holder.clone();

        // Spawn heartbeat task
        let heartbeat_ws_url = gateway_url.clone();
        let heartbeat_account_id = account_id.clone();
        let main_cancel = cancel.clone();
        tokio::spawn(async move {
            let interval = std::time::Duration::from_millis(heartbeat_interval_ms);
            loop {
                tokio::select! {
                    _ = heartbeat_cancel.cancelled() => break,
                    _ = main_cancel.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {
                        // Heartbeat is sent from the main loop since we can't share ws
                        // The main loop will handle heartbeat timing
                    }
                }
            }
            app_debug!(
                "channel",
                "qqbot::gateway",
                "Heartbeat task ended for account '{}' (url={})",
                heartbeat_account_id,
                crate::truncate_utf8(&heartbeat_ws_url, 60)
            );
        });

        // Main event loop with integrated heartbeat
        let mut heartbeat_timer =
            tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval_ms));
        heartbeat_timer.tick().await; // consume the immediate first tick

        let mut session_active = true;
        while session_active {
            tokio::select! {
                _ = cancel.cancelled() => {
                    app_info!(
                        "channel",
                        "qqbot::gateway",
                        "Gateway cancelled for account '{}'",
                        account_id
                    );
                    heartbeat_token.cancel();
                    ws.close().await;
                    // Save session before exit
                    if let Some(ref sid) = saved_session_id {
                        let seq = *seq_holder.lock().await;
                        save_session(&account_id, sid, seq);
                    }
                    return;
                }
                _ = heartbeat_timer.tick() => {
                    let seq = *seq_holder.lock().await;
                    let hb = serde_json::json!({
                        "op": opcode::HEARTBEAT,
                        "d": if seq > 0 { serde_json::Value::Number(seq.into()) } else { serde_json::Value::Null }
                    });
                    if let Err(e) = ws.send_json(&hb).await {
                        app_warn!(
                            "channel",
                            "qqbot::gateway",
                            "Failed to send heartbeat for account '{}': {}",
                            account_id,
                            e
                        );
                        session_active = false;
                    }
                }
                msg = ws.recv_text() => {
                    match msg {
                        Some(text) => {
                            match handle_gateway_message(
                                &text,
                                &account_id,
                                &inbound_tx,
                                &seq_holder,
                                &mut saved_session_id,
                            ).await {
                                GatewayAction::Continue => {}
                                GatewayAction::Reconnect => {
                                    session_active = false;
                                }
                                GatewayAction::ReidentifyAndReconnect => {
                                    saved_session_id = None;
                                    // Reset the sequence so the outer-loop resume check
                                    // (`saved_session_id.is_some() && last_seq > 0`) sees
                                    // a fresh-session state on the next iteration.
                                    #[allow(unused_assignments)]
                                    {
                                        last_seq = 0;
                                    }
                                    session_active = false;
                                }
                            }
                        }
                        None => {
                            app_warn!(
                                "channel",
                                "qqbot::gateway",
                                "WebSocket disconnected for account '{}', reconnecting",
                                account_id
                            );
                            session_active = false;
                        }
                    }
                }
            }
        }

        heartbeat_token.cancel();

        // Save session state before reconnect
        {
            let seq = *seq_holder.lock().await;
            last_seq = seq;
            if let Some(ref sid) = saved_session_id {
                save_session(&account_id, sid, seq);
            }
        }

        // Reconnect with backoff
        if reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
            app_error!(
                "channel",
                "qqbot::gateway",
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
        "qqbot::gateway",
        "Gateway loop ended for account '{}'",
        account_id
    );
}

/// Receive the HELLO message (op=10) and extract heartbeat_interval.
async fn recv_hello(ws: &mut WsConnection, cancel: &CancellationToken) -> Option<u64> {
    let timeout = std::time::Duration::from_secs(30);
    tokio::select! {
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(timeout, ws.recv_text()) => {
            match result {
                Ok(Some(text)) => {
                    let msg: serde_json::Value = serde_json::from_str(&text).ok()?;
                    let op = msg.get("op")?.as_u64()?;
                    if op != opcode::HELLO {
                        app_warn!(
                            "channel",
                            "qqbot::gateway",
                            "Expected HELLO (op=10) but got op={}",
                            op
                        );
                        return None;
                    }
                    let interval = msg.get("d")?.get("heartbeat_interval")?.as_u64()?;
                    Some(interval)
                }
                _ => None,
            }
        }
    }
}

/// Result of handling a gateway message.
enum GatewayAction {
    Continue,
    Reconnect,
    ReidentifyAndReconnect,
}

/// Handle a single gateway message.
async fn handle_gateway_message(
    text: &str,
    account_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
    seq_holder: &Arc<tokio::sync::Mutex<u64>>,
    saved_session_id: &mut Option<String>,
) -> GatewayAction {
    let msg: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            app_warn!(
                "channel",
                "qqbot::gateway",
                "Failed to parse gateway message: {}",
                e
            );
            return GatewayAction::Continue;
        }
    };

    let op = msg.get("op").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);

    // Update sequence number if present
    if let Some(s) = msg.get("s").and_then(|v| v.as_u64()) {
        let mut seq = seq_holder.lock().await;
        *seq = s;
    }

    match op {
        opcode::DISPATCH => {
            let event_type = msg.get("t").and_then(|v| v.as_str()).unwrap_or("");
            let data = msg.get("d");

            match event_type {
                "READY" => {
                    // Extract session_id and bot info from READY event
                    if let Some(d) = data {
                        if let Some(session_id) = d.get("session_id").and_then(|v| v.as_str()) {
                            *saved_session_id = Some(session_id.to_string());
                            app_info!(
                                "channel",
                                "qqbot::gateway",
                                "READY received for account '{}' (session_id={})",
                                account_id,
                                crate::truncate_utf8(session_id, 30)
                            );
                        }
                        if let Some(user) = d.get("user") {
                            let bot_id = user.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let bot_name =
                                user.get("username").and_then(|v| v.as_str()).unwrap_or("?");
                            app_info!(
                                "channel",
                                "qqbot::gateway",
                                "Bot identified: {} (id={}) for account '{}'",
                                bot_name,
                                bot_id,
                                account_id
                            );
                        }
                    }
                }
                "RESUMED" => {
                    app_info!(
                        "channel",
                        "qqbot::gateway",
                        "RESUMED successfully for account '{}'",
                        account_id
                    );
                }
                "C2C_MESSAGE_CREATE" => {
                    if let Some(d) = data {
                        if let Some(msg_ctx) = convert_c2c_message(d, account_id) {
                            if let Err(e) = inbound_tx.send(msg_ctx).await {
                                app_error!(
                                    "channel",
                                    "qqbot::gateway",
                                    "Failed to send C2C inbound message: {}",
                                    e
                                );
                            }
                        }
                    }
                }
                "GROUP_AT_MESSAGE_CREATE" => {
                    if let Some(d) = data {
                        if let Some(msg_ctx) = convert_group_message(d, account_id) {
                            if let Err(e) = inbound_tx.send(msg_ctx).await {
                                app_error!(
                                    "channel",
                                    "qqbot::gateway",
                                    "Failed to send group inbound message: {}",
                                    e
                                );
                            }
                        }
                    }
                }
                "AT_MESSAGE_CREATE" => {
                    if let Some(d) = data {
                        if let Some(msg_ctx) = convert_channel_message(d, account_id) {
                            if let Err(e) = inbound_tx.send(msg_ctx).await {
                                app_error!(
                                    "channel",
                                    "qqbot::gateway",
                                    "Failed to send channel inbound message: {}",
                                    e
                                );
                            }
                        }
                    }
                }
                "DIRECT_MESSAGE_CREATE" => {
                    if let Some(d) = data {
                        if let Some(msg_ctx) = convert_dms_message(d, account_id) {
                            if let Err(e) = inbound_tx.send(msg_ctx).await {
                                app_error!(
                                    "channel",
                                    "qqbot::gateway",
                                    "Failed to send DMS inbound message: {}",
                                    e
                                );
                            }
                        }
                    }
                }
                "INTERACTION_CREATE" => {
                    if let Some(d) = data {
                        if let Some(button_data) = d
                            .pointer("/data/resolved/button_data")
                            .and_then(|v| v.as_str())
                        {
                            crate::channel::worker::ask_user::try_dispatch_interactive_callback(
                                button_data,
                                "qqbot::gateway",
                            );
                        }
                    }
                }
                other => {
                    app_debug!(
                        "channel",
                        "qqbot::gateway",
                        "Ignoring event '{}' for account '{}'",
                        other,
                        account_id
                    );
                }
            }

            GatewayAction::Continue
        }
        opcode::HEARTBEAT => {
            // Server requesting a heartbeat — we already send them on a timer
            app_debug!(
                "channel",
                "qqbot::gateway",
                "Server requested heartbeat for account '{}'",
                account_id
            );
            GatewayAction::Continue
        }
        opcode::RECONNECT => {
            app_info!(
                "channel",
                "qqbot::gateway",
                "Received RECONNECT (op=7) for account '{}', reconnecting",
                account_id
            );
            GatewayAction::Reconnect
        }
        opcode::INVALID_SESSION => {
            // d=false means we must re-identify; d=true means we can resume
            let can_resume = msg.get("d").and_then(|v| v.as_bool()).unwrap_or(false);
            if can_resume {
                app_info!(
                    "channel",
                    "qqbot::gateway",
                    "INVALID_SESSION (resumable) for account '{}', reconnecting with resume",
                    account_id
                );
                GatewayAction::Reconnect
            } else {
                app_info!(
                    "channel",
                    "qqbot::gateway",
                    "INVALID_SESSION (not resumable) for account '{}', re-identifying",
                    account_id
                );
                GatewayAction::ReidentifyAndReconnect
            }
        }
        opcode::HEARTBEAT_ACK => {
            app_debug!(
                "channel",
                "qqbot::gateway",
                "Heartbeat ACK received for account '{}'",
                account_id
            );
            GatewayAction::Continue
        }
        _ => {
            app_debug!(
                "channel",
                "qqbot::gateway",
                "Unknown opcode {} for account '{}'",
                op,
                account_id
            );
            GatewayAction::Continue
        }
    }
}

/// Strip QQ mention tags like `<@!bot_id>` from message content.
fn strip_mention_tags(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' && chars.peek() == Some(&'@') {
            // Skip until we find '>'
            let mut found_close = false;
            for c in chars.by_ref() {
                if c == '>' {
                    found_close = true;
                    break;
                }
            }
            if !found_close {
                // Unclosed tag — output what we skipped
                result.push('<');
                result.push('@');
            }
            // Add a space if the mention was followed by content
            if !result.is_empty() && !result.ends_with(' ') {
                // Only add space if there's more content coming
                if chars.peek().is_some() {
                    // Don't double space
                }
            }
        } else {
            result.push(ch);
        }
    }

    result.trim().to_string()
}

/// Convert a C2C_MESSAGE_CREATE event to MsgContext.
fn convert_c2c_message(d: &serde_json::Value, account_id: &str) -> Option<MsgContext> {
    let message_id = d.get("id").and_then(|v| v.as_str())?.to_string();
    let user_openid = d
        .get("author")
        .and_then(|a| a.get("user_openid"))
        .and_then(|v| v.as_str())?
        .to_string();

    let content = d
        .get("content")
        .and_then(|v| v.as_str())
        .map(strip_mention_tags)
        .unwrap_or_default();

    let timestamp = parse_qq_timestamp(d.get("timestamp").and_then(|v| v.as_str()))
        .unwrap_or_else(chrono::Utc::now);

    Some(MsgContext {
        channel_id: ChannelId::QqBot,
        account_id: account_id.to_string(),
        sender_id: user_openid.clone(),
        sender_name: None,
        sender_username: None,
        chat_id: format!("c2c:{}", user_openid),
        chat_type: ChatType::Dm,
        chat_title: None,
        thread_id: None,
        message_id,
        text: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned: false,
        raw: d.clone(),
    })
}

/// Convert a GROUP_AT_MESSAGE_CREATE event to MsgContext.
fn convert_group_message(d: &serde_json::Value, account_id: &str) -> Option<MsgContext> {
    let message_id = d.get("id").and_then(|v| v.as_str())?.to_string();
    let group_openid = d.get("group_openid").and_then(|v| v.as_str())?.to_string();

    let member_openid = d
        .get("author")
        .and_then(|a| a.get("member_openid"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let content = d
        .get("content")
        .and_then(|v| v.as_str())
        .map(strip_mention_tags)
        .unwrap_or_default();

    let timestamp = parse_qq_timestamp(d.get("timestamp").and_then(|v| v.as_str()))
        .unwrap_or_else(chrono::Utc::now);

    Some(MsgContext {
        channel_id: ChannelId::QqBot,
        account_id: account_id.to_string(),
        sender_id: member_openid,
        sender_name: None,
        sender_username: None,
        chat_id: format!("group:{}", group_openid),
        chat_type: ChatType::Group,
        chat_title: None,
        thread_id: None,
        message_id,
        text: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned: true, // GROUP_AT means bot was @mentioned
        raw: d.clone(),
    })
}

/// Convert an AT_MESSAGE_CREATE event to MsgContext (guild channel).
fn convert_channel_message(d: &serde_json::Value, account_id: &str) -> Option<MsgContext> {
    let message_id = d.get("id").and_then(|v| v.as_str())?.to_string();
    let channel_id_str = d.get("channel_id").and_then(|v| v.as_str())?.to_string();

    let author_id = d
        .get("author")
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let author_name = d
        .get("author")
        .and_then(|a| a.get("username"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let content = d
        .get("content")
        .and_then(|v| v.as_str())
        .map(strip_mention_tags)
        .unwrap_or_default();

    let timestamp = parse_qq_timestamp(d.get("timestamp").and_then(|v| v.as_str()))
        .unwrap_or_else(chrono::Utc::now);

    Some(MsgContext {
        channel_id: ChannelId::QqBot,
        account_id: account_id.to_string(),
        sender_id: author_id,
        sender_name: author_name,
        sender_username: None,
        chat_id: format!("channel:{}", channel_id_str),
        chat_type: ChatType::Channel,
        chat_title: None,
        thread_id: None,
        message_id,
        text: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned: true, // AT_MESSAGE means bot was @mentioned
        raw: d.clone(),
    })
}

/// Convert a DIRECT_MESSAGE_CREATE event to MsgContext (guild DM).
fn convert_dms_message(d: &serde_json::Value, account_id: &str) -> Option<MsgContext> {
    let message_id = d.get("id").and_then(|v| v.as_str())?.to_string();
    let guild_id = d.get("guild_id").and_then(|v| v.as_str())?.to_string();

    let author_id = d
        .get("author")
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let author_name = d
        .get("author")
        .and_then(|a| a.get("username"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let content = d
        .get("content")
        .and_then(|v| v.as_str())
        .map(strip_mention_tags)
        .unwrap_or_default();

    let timestamp = parse_qq_timestamp(d.get("timestamp").and_then(|v| v.as_str()))
        .unwrap_or_else(chrono::Utc::now);

    Some(MsgContext {
        channel_id: ChannelId::QqBot,
        account_id: account_id.to_string(),
        sender_id: author_id,
        sender_name: author_name,
        sender_username: None,
        chat_id: format!("dms:{}", guild_id),
        chat_type: ChatType::Dm,
        chat_title: None,
        thread_id: None,
        message_id,
        text: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned: false,
        raw: d.clone(),
    })
}

/// Parse a QQ Bot timestamp string (ISO 8601 format) into a DateTime<Utc>.
fn parse_qq_timestamp(ts: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let ts = ts?;
    // QQ Bot timestamps can be ISO 8601 format
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|| {
            // Try parsing without timezone
            chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .map(|ndt| ndt.and_utc())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_mention_tags() {
        assert_eq!(strip_mention_tags("<@!12345> hello"), "hello");
        assert_eq!(strip_mention_tags("<@12345> hello"), "hello");
        assert_eq!(strip_mention_tags("hello <@!12345> world"), "hello  world");
        assert_eq!(strip_mention_tags("no mentions here"), "no mentions here");
        assert_eq!(strip_mention_tags(""), "");
    }

    #[test]
    fn test_parse_qq_timestamp() {
        let ts = "2026-04-02T10:30:00+08:00";
        let dt = parse_qq_timestamp(Some(ts));
        assert!(dt.is_some());

        let ts2 = "2026-04-02T02:30:00Z";
        let dt2 = parse_qq_timestamp(Some(ts2));
        assert!(dt2.is_some());
    }

    #[test]
    fn test_parse_qq_timestamp_none() {
        assert!(parse_qq_timestamp(None).is_none());
        assert!(parse_qq_timestamp(Some("invalid")).is_none());
    }
}
