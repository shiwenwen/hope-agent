use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::channel::webhook_server::{WebhookHandlerFn, WebhookRequest, WebhookResponse};
use ha_core::channel::types::*;

use super::api::LineApi;

/// Shared store for reply tokens. Keys are chat_id, values are (reply_token, timestamp).
/// Reply tokens expire after ~1 minute, so we store the insertion time for cleanup.
pub type ReplyTokenStore = Arc<Mutex<HashMap<String, (String, std::time::Instant)>>>;

const MAX_TRACKED_WEBHOOK_EVENTS: usize = 4096;
const MAX_TRACKED_EDIT_MESSAGES: usize = 2048;

/// Bounded, account-local replay/order state. LINE can redeliver the same
/// webhook and can deliver retried events out of order; neither condition may
/// trigger a second agent turn or roll edit observations back to older text.
#[derive(Default)]
struct WebhookEventState {
    seen_event_ids: HashSet<String>,
    seen_event_order: VecDeque<String>,
    latest_edit_timestamps: HashMap<String, i64>,
    edit_order: VecDeque<(String, i64)>,
}

impl WebhookEventState {
    fn accept_event(&mut self, event: &serde_json::Value) -> bool {
        let Some(event_id) = event
            .get("webhookEventId")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            // Older/test payloads without an event id remain compatible.
            return true;
        };
        if !self.seen_event_ids.insert(event_id.to_string()) {
            return false;
        }
        self.seen_event_order.push_back(event_id.to_string());
        while self.seen_event_order.len() > MAX_TRACKED_WEBHOOK_EVENTS {
            if let Some(oldest) = self.seen_event_order.pop_front() {
                self.seen_event_ids.remove(&oldest);
            }
        }
        true
    }

    fn accept_edit(&mut self, message_id: &str, timestamp_ms: i64) -> bool {
        if self
            .latest_edit_timestamps
            .get(message_id)
            .is_some_and(|latest| timestamp_ms <= *latest)
        {
            return false;
        }
        self.latest_edit_timestamps
            .insert(message_id.to_string(), timestamp_ms);
        self.edit_order
            .push_back((message_id.to_string(), timestamp_ms));
        // Bound the queue itself as well as the map: one heavily edited
        // message must not grow `edit_order` forever while the map stays at 1.
        while self.edit_order.len() > MAX_TRACKED_EDIT_MESSAGES {
            let Some((oldest_id, oldest_timestamp)) = self.edit_order.pop_front() else {
                break;
            };
            if self.latest_edit_timestamps.get(&oldest_id) == Some(&oldest_timestamp) {
                self.latest_edit_timestamps.remove(&oldest_id);
            }
        }
        true
    }
}

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
    inbound_tx: mpsc::Sender<InboundEvent>,
    reply_tokens: ReplyTokenStore,
) -> WebhookHandlerFn {
    let event_state = Arc::new(Mutex::new(WebhookEventState::default()));
    Arc::new(move |req: WebhookRequest| {
        let api = api.clone();
        let account_id = account_id.clone();
        let channel_secret = channel_secret.clone();
        let inbound_tx = inbound_tx.clone();
        let reply_tokens = reply_tokens.clone();
        let event_state = event_state.clone();

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
                    status: 404,
                    body: r#"{"error":"Not found"}"#.to_string(),
                };
            }

            // Verify signature
            if !verify_signature(&req.body, &signature, &channel_secret) {
                app_warn!("channel", "line", "Webhook signature verification failed");
                return WebhookResponse {
                    status: 404,
                    body: r#"{"error":"Not found"}"#.to_string(),
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

                // In standby mode another LINE module owns the conversation;
                // replying would race it and confuse the user.
                if event.get("mode").and_then(|v| v.as_str()) == Some("standby") {
                    app_debug!(
                        "channel",
                        "line",
                        "Ignoring standby webhook event type={}",
                        event_type
                    );
                    continue;
                }
                if !event_state.lock().await.accept_event(event) {
                    app_debug!(
                        "channel",
                        "line",
                        "Ignoring duplicate webhook event type={}",
                        event_type
                    );
                    continue;
                }

                match event_type {
                    "message" => {
                        handle_message_event(event, &api, &account_id, &inbound_tx, &reply_tokens)
                            .await;
                    }
                    "messageEdited" => {
                        handle_message_edited_event(event, &account_id, &inbound_tx, &event_state)
                            .await;
                    }
                    "unsend" => {
                        handle_unsend_event(event, &account_id, &inbound_tx).await;
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
                            let chat_id = chat_id_from_source(&source, source_type, &user_id);
                            if let Some(rest) = postback_data.strip_prefix("slash:") {
                                // postback 无 message_id，按 timestamp 合成一个稳定串
                                let ts = event
                                    .get("timestamp")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
                                let message_id = format!("postback_{}", ts);

                                crate::channel::worker::slash_callback::inject_slash_callback(
                                    ChannelId::Line,
                                    &account_id,
                                    &chat_id,
                                    None,
                                    &user_id,
                                    &message_id,
                                    rest,
                                    &inbound_tx,
                                    "line",
                                )
                                .await;
                            } else {
                                crate::channel::worker::ask_user::try_dispatch_interactive_callback(
                                    postback_data,
                                    "line",
                                    Some(crate::channel::worker::ask_user::InteractiveCallbackSource::new(
                                        ChannelId::Line,
                                        &account_id,
                                        &chat_id,
                                        None,
                                    )),
                                );
                            }
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

fn event_timestamp(event: &serde_json::Value) -> (i64, chrono::DateTime<chrono::Utc>) {
    let timestamp_ms = event
        .get("timestamp")
        .and_then(|value| value.as_i64())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp =
        chrono::DateTime::from_timestamp_millis(timestamp_ms).unwrap_or_else(chrono::Utc::now);
    (timestamp_ms, timestamp)
}

fn event_chat_context(event: &serde_json::Value) -> (String, String, ChatType) {
    let source = event.get("source").cloned().unwrap_or_default();
    let source_type = source
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("user");
    let sender_id = source
        .get("userId")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let chat_type = match source_type {
        "group" | "room" => ChatType::Group,
        _ => ChatType::Dm,
    };
    let chat_id = chat_id_from_source(&source, source_type, &sender_id);
    (sender_id, chat_id, chat_type)
}

/// Convert LINE's edit webhook into the shared out-of-band event. The worker
/// deliberately logs this variant without starting a new agent round.
async fn handle_message_edited_event(
    event: &serde_json::Value,
    account_id: &str,
    inbound_tx: &mpsc::Sender<InboundEvent>,
    event_state: &Arc<Mutex<WebhookEventState>>,
) {
    let Some(message) = event.get("message") else {
        return;
    };
    if message.get("type").and_then(|value| value.as_str()) != Some("text") {
        app_warn!("channel", "line", "Ignoring non-text messageEdited event");
        return;
    }
    let Some(message_id) = message
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        app_warn!("channel", "line", "messageEdited event missing message id");
        return;
    };
    let (timestamp_ms, timestamp) = event_timestamp(event);
    if !event_state
        .lock()
        .await
        .accept_edit(message_id, timestamp_ms)
    {
        app_debug!(
            "channel",
            "line",
            "Ignoring stale messageEdited event msg={}",
            message_id
        );
        return;
    }

    let (sender_id, chat_id, chat_type) = event_chat_context(event);
    let edited = EditedMessageEvent {
        common: EventCommon {
            channel_id: ChannelId::Line,
            account_id: account_id.to_string(),
            chat_id,
            chat_type,
            timestamp,
            raw: Arc::new(event.clone()),
        },
        message_id: message_id.to_string(),
        sender_id,
        new_text: message
            .get("text")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        edited_at: timestamp,
    };
    if let Err(error) = inbound_tx.send(InboundEvent::MessageEdited(edited)).await {
        app_error!(
            "channel",
            "line",
            "Failed to send messageEdited event: {}",
            error
        );
    }
}

async fn handle_unsend_event(
    event: &serde_json::Value,
    account_id: &str,
    inbound_tx: &mpsc::Sender<InboundEvent>,
) {
    let Some(message_id) = event
        .pointer("/unsend/messageId")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        app_warn!("channel", "line", "unsend event missing message id");
        return;
    };
    let (_, timestamp) = event_timestamp(event);
    let (sender_id, chat_id, chat_type) = event_chat_context(event);
    let recalled = RecalledMessageEvent {
        common: EventCommon {
            channel_id: ChannelId::Line,
            account_id: account_id.to_string(),
            chat_id,
            chat_type,
            timestamp,
            raw: Arc::new(event.clone()),
        },
        message_id: message_id.to_string(),
        recalled_by: (!sender_id.is_empty()).then_some(sender_id),
    };
    if let Err(error) = inbound_tx
        .send(InboundEvent::MessageRecalled(recalled))
        .await
    {
        app_error!("channel", "line", "Failed to send unsend event: {}", error);
    }
}

/// Resolve a LINE event's chat_id from its `source` object.
///
/// LINE encodes the conversation kind in `source.type` (`user` / `group` /
/// `room`) with the actual identifier under `groupId` / `roomId` / `userId`
/// respectively. DMs fall back to `user_id_fallback` so caller can pass the
/// already-extracted `source.userId` without a redundant lookup.
fn chat_id_from_source(
    source: &serde_json::Value,
    source_type: &str,
    user_id_fallback: &str,
) -> String {
    let key = match source_type {
        "group" => "groupId",
        "room" => "roomId",
        _ => return user_id_fallback.to_string(),
    };
    source
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Handle a LINE message event and forward to inbound_tx.
async fn handle_message_event(
    event: &serde_json::Value,
    api: &LineApi,
    account_id: &str,
    inbound_tx: &mpsc::Sender<InboundEvent>,
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

    let chat_type = match source_type {
        "group" | "room" => ChatType::Group,
        _ => ChatType::Dm,
    };
    let chat_id = chat_id_from_source(&source, source_type, &user_id);

    // Store reply token (LINE 官方约定 ~1 分钟有效；保守 55s 留 5s buffer
    // 应对时钟漂移；与 send_message 中 reply_token 选用窗口一致)
    if let Some(reply_token) = event.get("replyToken").and_then(|v| v.as_str()) {
        let mut tokens = reply_tokens.lock().await;
        let now = std::time::Instant::now();
        tokens.retain(|_, (_, ts)| now.duration_since(*ts).as_secs() < 55);
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

    // Parse binary message types (image / video / audio / file) into
    // deferred refs; LinePlugin::materialize_pending_media downloads the
    // bytes via the Content API after dispatcher gating.
    let pending_media = super::inbound_media::parse_message(message);
    let mut raw = event.clone();
    crate::channel::inbound_media_common::embed_pending_refs(&mut raw, pending_media);

    let msg_ctx = MsgContext {
        channel_id: ChannelId::Line,
        account_id: account_id.to_string(),
        sender_id: user_id.clone(),
        sender_name,
        sender_username: None,
        sender_tenant_id: None,
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
        raw,
    };

    if let Err(e) = inbound_tx.send(InboundEvent::Message(msg_ctx)).await {
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

    #[test]
    fn webhook_state_deduplicates_redelivery_and_stale_edits() {
        let mut state = WebhookEventState::default();
        let event = serde_json::json!({"webhookEventId": "evt-1"});
        assert!(state.accept_event(&event));
        assert!(!state.accept_event(&event));

        assert!(state.accept_edit("msg-1", 200));
        assert!(!state.accept_edit("msg-1", 100));
        assert!(!state.accept_edit("msg-1", 200));
        assert!(state.accept_edit("msg-1", 300));

        for timestamp in 301..3000 {
            assert!(state.accept_edit("msg-1", timestamp));
        }
        assert!(state.edit_order.len() <= MAX_TRACKED_EDIT_MESSAGES);
        assert_eq!(state.latest_edit_timestamps.len(), 1);
    }

    #[tokio::test]
    async fn edit_and_unsend_are_out_of_band_events() {
        let (tx, mut rx) = mpsc::channel(4);
        let state = Arc::new(Mutex::new(WebhookEventState::default()));
        let edit = serde_json::json!({
            "type": "messageEdited",
            "timestamp": 1_776_914_799_524_i64,
            "source": {"type": "group", "groupId": "C1", "userId": "U1"},
            "message": {"type": "text", "id": "M1", "text": "edited"}
        });
        handle_message_edited_event(&edit, "account", &tx, &state).await;
        let InboundEvent::MessageEdited(edited) = rx.recv().await.unwrap() else {
            panic!("expected MessageEdited");
        };
        assert_eq!(edited.message_id, "M1");
        assert_eq!(edited.new_text.as_deref(), Some("edited"));

        let unsend = serde_json::json!({
            "type": "unsend",
            "timestamp": 1_776_914_799_525_i64,
            "source": {"type": "group", "groupId": "C1", "userId": "U1"},
            "unsend": {"messageId": "M1"}
        });
        handle_unsend_event(&unsend, "account", &tx).await;
        let InboundEvent::MessageRecalled(recalled) = rx.recv().await.unwrap() else {
            panic!("expected MessageRecalled");
        };
        assert_eq!(recalled.message_id, "M1");
    }
}
