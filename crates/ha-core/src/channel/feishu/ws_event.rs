use std::sync::Arc;
use std::time::Duration;

use prost::Message as _;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;
use tokio_util::sync::CancellationToken;

use crate::channel::types::*;
use crate::channel::ws;

use super::api::FeishuApi;
use super::data_cache::DataCache;
use super::proto::{Frame, Header};

/// Maximum number of consecutive reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: usize = 50;

// pbbp2 Frame.method values
const METHOD_CONTROL: i32 = 0;
const METHOD_DATA: i32 = 1;

// pbbp2 Frame.headers keys
const HK_TYPE: &str = "type";
const HK_SUM: &str = "sum";
const HK_SEQ: &str = "seq";
const HK_MESSAGE_ID: &str = "message_id";
const HK_TRACE_ID: &str = "trace_id";
const HK_BIZ_RT: &str = "biz_rt";

// Frame headers[type] values
const TY_PING: &str = "ping";
const TY_PONG: &str = "pong";
const TY_EVENT: &str = "event";
const TY_CARD: &str = "card";

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

/// Server-pushed runtime parameters carried in pong payloads. Field names
/// mirror the wire format (PascalCase). Only `PingInterval` is consumed
/// today; the reconnect fields are reserved for when we adopt server-driven
/// reconnect (currently we use a fixed local backoff).
#[derive(Debug, Default, Deserialize)]
struct PongPayload {
    #[serde(rename = "PingInterval", default)]
    ping_interval: Option<u64>,
    #[serde(rename = "ReconnectCount", default)]
    #[allow(dead_code)]
    reconnect_count: Option<i64>,
    #[serde(rename = "ReconnectInterval", default)]
    #[allow(dead_code)]
    reconnect_interval: Option<u64>,
    #[serde(rename = "ReconnectNonce", default)]
    #[allow(dead_code)]
    reconnect_nonce: Option<u64>,
}

/// Run the Feishu WebSocket gateway event loop.
///
/// Connects to Feishu's long-connection WebSocket endpoint and listens for
/// inbound events (primarily `im.message.receive_v1`). The wire format is
/// pbbp2 protobuf frames: `method=0` for control (ping/pong), `method=1` for
/// data (event/card). Data payloads are UTF-8 JSON. Every data frame must be
/// acknowledged with a response frame or the server treats it as undelivered.
///
/// Automatically reconnects with exponential backoff on disconnection.
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

        // 1. Obtain WebSocket endpoint URL + negotiated client params
        let endpoint = match api.get_ws_endpoint().await {
            Ok(info) => {
                reconnect_attempts = 0;
                info
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

        // service_id is embedded in the endpoint URL's query string and is
        // required to address ping frames to the correct gateway service.
        let service_id = parse_service_id(&endpoint.url);

        app_info!(
            "channel",
            "feishu:gateway",
            "[{}] Connecting to WebSocket endpoint (service_id={}, ping={}s)",
            account_id,
            service_id,
            endpoint.ping_interval.as_secs()
        );

        // 2. Connect to WebSocket
        let mut conn = match ws::WsConnection::connect(&endpoint.url).await {
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

        // 3. Heartbeat scheduler — initial cadence from the endpoint's
        // ClientConfig.PingInterval, mutable at runtime so pong frames can
        // adopt server-pushed interval changes. We track the next deadline
        // explicitly (instead of `tokio::time::interval`) so an interval
        // update can take effect on the very next tick.
        let mut current_interval = endpoint.ping_interval;
        let mut next_ping_at = TokioInstant::now() + current_interval;

        // Per-connection shard cache for sum>1 events. Discarded on disconnect
        // (in-flight shards from a dead connection are unrecoverable anyway).
        let cache = DataCache::new();

        // 4. Receive loop
        loop {
            enum Action {
                Frame(Vec<u8>),
                SendPing,
                Disconnected,
                Cancelled,
            }

            let action = tokio::select! {
                biased;
                _ = cancel.cancelled() => Action::Cancelled,
                _ = tokio::time::sleep_until(next_ping_at) => Action::SendPing,
                bytes = conn.recv_binary() => match bytes {
                    Some(b) => Action::Frame(b),
                    None => Action::Disconnected,
                },
            };

            match action {
                Action::Cancelled => {
                    app_info!(
                        "channel",
                        "feishu:gateway",
                        "[{}] Shutdown requested, closing WebSocket",
                        account_id
                    );
                    conn.close().await;
                    return;
                }
                Action::SendPing => {
                    let frame = build_ping_frame(service_id);
                    if let Err(e) = conn.send_binary(encode_frame(&frame)).await {
                        app_warn!(
                            "channel",
                            "feishu:gateway",
                            "[{}] Ping send failed, will reconnect: {}",
                            account_id,
                            e
                        );
                        break;
                    }
                    next_ping_at = TokioInstant::now() + current_interval;
                }
                Action::Frame(bytes) => {
                    match handle_frame(
                        &bytes,
                        &mut conn,
                        &cache,
                        &account_id,
                        &bot_open_id,
                        &inbound_tx,
                    )
                    .await
                    {
                        Ok(Some(new_interval)) if new_interval != current_interval => {
                            app_info!(
                                "channel",
                                "feishu:gateway",
                                "[{}] Server-updated ping interval: {}s → {}s",
                                account_id,
                                current_interval.as_secs(),
                                new_interval.as_secs()
                            );
                            current_interval = new_interval;
                            // Reschedule from now so the new cadence applies
                            // immediately rather than waiting out the old slot.
                            next_ping_at = TokioInstant::now() + current_interval;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            app_debug!(
                                "channel",
                                "feishu:gateway",
                                "[{}] Frame handling error: {}",
                                account_id,
                                e
                            );
                        }
                    }
                }
                Action::Disconnected => {
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

/// Parse `service_id` from the WS endpoint URL's query string. Defaults to 1
/// if unparseable — Feishu always sets it, but a sane default keeps the loop
/// running rather than crashing on a malformed URL.
fn parse_service_id(url: &str) -> i32 {
    let Some(q_idx) = url.find('?') else {
        return 1;
    };
    for kv in url[q_idx + 1..].split('&') {
        if let Some(rest) = kv.strip_prefix("service_id=") {
            if let Ok(n) = rest.parse() {
                return n;
            }
        }
    }
    1
}

fn header(key: &str, value: impl Into<String>) -> Header {
    Header {
        key: key.to_string(),
        value: value.into(),
    }
}

fn find_header<'a>(frame: &'a Frame, key: &str) -> Option<&'a str> {
    frame
        .headers
        .iter()
        .find(|h| h.key == key)
        .map(|h| h.value.as_str())
}

fn build_ping_frame(service_id: i32) -> Frame {
    Frame {
        seq_id: 0,
        log_id: 0,
        service: service_id,
        method: METHOD_CONTROL,
        headers: vec![header(HK_TYPE, TY_PING)],
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload: Vec::new(),
        log_id_new: String::new(),
    }
}

fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(frame.encoded_len());
    // prost encode is infallible into Vec<u8> (no buffer overflow possible).
    frame.encode(&mut buf).expect("prost encode infallible");
    buf
}

/// Decode a single inbound frame and dispatch by method.
///
/// Returns `Ok(Some(new_interval))` when the frame was a pong carrying an
/// updated `PingInterval` — caller reschedules the heartbeat. `Ok(None)` for
/// any other case (control without interval change, data event, unknown
/// method).
///
/// - control + pong → parse `PingInterval`, return `Some(_)` if it differs
/// - control + ping → noop (server probe)
/// - data + event/card → parse JSON (merging shards if sum>1), dispatch, ack
async fn handle_frame(
    bytes: &[u8],
    conn: &mut ws::WsConnection,
    cache: &DataCache,
    account_id: &str,
    bot_open_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) -> anyhow::Result<Option<Duration>> {
    let frame = Frame::decode(bytes)
        .map_err(|e| anyhow::anyhow!("Failed to decode pbbp2 frame: {}", e))?;

    match frame.method {
        METHOD_CONTROL => Ok(handle_control_frame(&frame)),
        METHOD_DATA => {
            handle_data_frame(&frame, conn, cache, account_id, bot_open_id, inbound_tx).await?;
            Ok(None)
        }
        other => {
            app_debug!(
                "channel",
                "feishu:gateway",
                "[{}] Ignoring frame with unknown method: {}",
                account_id,
                other
            );
            Ok(None)
        }
    }
}

/// Handle a control frame (ping/pong). Returns a fresh `Duration` if the
/// frame is a pong whose payload carries a non-zero `PingInterval`.
fn handle_control_frame(frame: &Frame) -> Option<Duration> {
    let ty = find_header(frame, HK_TYPE)?;
    if ty != TY_PONG {
        return None;
    }
    if frame.payload.is_empty() {
        return None;
    }
    let payload_str = std::str::from_utf8(&frame.payload).ok()?;
    let parsed: PongPayload = serde_json::from_str(payload_str).ok()?;
    parsed
        .ping_interval
        .filter(|n| *n > 0)
        .map(Duration::from_secs)
}

async fn handle_data_frame(
    frame: &Frame,
    conn: &mut ws::WsConnection,
    cache: &DataCache,
    account_id: &str,
    bot_open_id: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
) -> anyhow::Result<()> {
    let ty = find_header(frame, HK_TYPE).unwrap_or("");

    if ty != TY_EVENT && ty != TY_CARD {
        // Unknown subtype; still ack to clear server queue.
        let _ = send_ack(conn, frame, 200).await;
        return Ok(());
    }

    let sum: usize = find_header(frame, HK_SUM)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    // Resolve the full event JSON: fast path for sum==1 (most events), else
    // route through the shard cache. Cache returns Some(bytes) only on the
    // final shard; in-progress shards yield None — we ack but don't dispatch.
    let payload_bytes: Vec<u8> = if sum <= 1 {
        frame.payload.clone()
    } else {
        let seq: usize = find_header(frame, HK_SEQ)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let message_id = find_header(frame, HK_MESSAGE_ID).unwrap_or("");
        let trace_id = find_header(frame, HK_TRACE_ID).unwrap_or("");
        if message_id.is_empty() {
            // Sharded frame without a message_id is unmergeable — ack and skip.
            app_warn!(
                "channel",
                "feishu:gateway",
                "[{}] Sharded frame missing message_id (sum={}, seq={}); dropping",
                account_id,
                sum,
                seq
            );
            let _ = send_ack(conn, frame, 200).await;
            return Ok(());
        }
        match cache.merge(message_id, sum, seq, trace_id, frame.payload.clone()) {
            Some(merged) => {
                app_debug!(
                    "channel",
                    "feishu:gateway",
                    "[{}] Merged sharded event (message_id={}, sum={})",
                    account_id,
                    message_id,
                    sum
                );
                merged
            }
            None => {
                // More shards expected — ack this frame so the server marks it
                // delivered, but defer dispatch until the final shard arrives.
                let _ = send_ack(conn, frame, 200).await;
                return Ok(());
            }
        }
    };

    let payload_str = std::str::from_utf8(&payload_bytes)
        .map_err(|e| anyhow::anyhow!("Non-UTF8 event payload: {}", e))?;

    let parsed: FeishuWsEvent = serde_json::from_str(payload_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse Feishu WS event: {}", e))?;

    let event_type = parsed
        .header
        .as_ref()
        .and_then(|h| h.event_type.as_deref())
        .unwrap_or("");

    let dispatch_result: anyhow::Result<()> = match event_type {
        "im.message.receive_v1" => {
            if let Some(event_data) = parsed.event {
                handle_message_event(event_data, account_id, bot_open_id, inbound_tx).await
            } else {
                Ok(())
            }
        }
        "card.action.trigger" => {
            if let Some(event_data) = parsed.event {
                if let Some(action) = event_data.get("action") {
                    if let Some(value) = action.get("value").and_then(|v| v.as_str()) {
                        crate::channel::worker::ask_user::try_dispatch_interactive_callback(
                            value,
                            "feishu:gateway",
                        );
                    }
                }
            }
            Ok(())
        }
        _ => {
            app_debug!(
                "channel",
                "feishu:gateway",
                "[{}] Ignoring event type: {}",
                account_id,
                event_type
            );
            Ok(())
        }
    };

    // Always ack — server requires acknowledgement for delivery tracking.
    let code = if dispatch_result.is_ok() { 200 } else { 500 };
    let _ = send_ack(conn, frame, code).await;
    dispatch_result
}

/// Send a data-frame acknowledgement back to the gateway. Mirrors the official
/// SDK's response shape: same headers + `biz_rt`, payload `{"code":<n>}`.
async fn send_ack(conn: &mut ws::WsConnection, src: &Frame, code: i32) -> anyhow::Result<()> {
    let mut headers = src.headers.clone();
    headers.push(header(HK_BIZ_RT, "0"));

    let payload = serde_json::json!({ "code": code }).to_string().into_bytes();

    let ack = Frame {
        seq_id: src.seq_id,
        log_id: src.log_id,
        service: src.service,
        method: METHOD_DATA,
        headers,
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload,
        log_id_new: src.log_id_new.clone(),
    };

    conn.send_binary(encode_frame(&ack)).await
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

    let sender_id = sender.sender_id.and_then(|s| s.open_id).unwrap_or_default();

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
            let digit_end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
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

    #[test]
    fn test_parse_service_id_basic() {
        assert_eq!(
            parse_service_id("wss://gw.feishu.cn/ws?device_id=abc&service_id=42"),
            42
        );
    }

    #[test]
    fn test_parse_service_id_default() {
        assert_eq!(parse_service_id("wss://gw.feishu.cn/ws"), 1);
        assert_eq!(parse_service_id("wss://gw.feishu.cn/ws?other=x"), 1);
    }

    #[test]
    fn test_frame_roundtrip_ping() {
        let f = build_ping_frame(7);
        let bytes = encode_frame(&f);
        let decoded = Frame::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.method, METHOD_CONTROL);
        assert_eq!(decoded.service, 7);
        assert_eq!(find_header(&decoded, HK_TYPE), Some(TY_PING));
    }

    fn make_pong(payload: &str) -> Frame {
        Frame {
            seq_id: 0,
            log_id: 0,
            service: 1,
            method: METHOD_CONTROL,
            headers: vec![header(HK_TYPE, TY_PONG)],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: payload.as_bytes().to_vec(),
            log_id_new: String::new(),
        }
    }

    #[test]
    fn test_pong_extracts_ping_interval() {
        let frame = make_pong(
            r#"{"PingInterval":60,"ReconnectCount":-1,"ReconnectInterval":120,"ReconnectNonce":30}"#,
        );
        assert_eq!(handle_control_frame(&frame), Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_pong_zero_interval_ignored() {
        let frame = make_pong(r#"{"PingInterval":0}"#);
        assert_eq!(handle_control_frame(&frame), None);
    }

    #[test]
    fn test_pong_missing_field_ignored() {
        let frame = make_pong(r#"{"ReconnectCount":-1}"#);
        assert_eq!(handle_control_frame(&frame), None);
    }

    #[test]
    fn test_pong_empty_payload_ignored() {
        let frame = make_pong("");
        assert_eq!(handle_control_frame(&frame), None);
    }

    #[test]
    fn test_ping_frame_returns_none() {
        let frame = build_ping_frame(1);
        assert_eq!(handle_control_frame(&frame), None);
    }

    #[test]
    fn test_pong_malformed_json_ignored() {
        let frame = make_pong("not json");
        assert_eq!(handle_control_frame(&frame), None);
    }
}
