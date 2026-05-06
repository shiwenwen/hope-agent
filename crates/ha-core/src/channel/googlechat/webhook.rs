use std::sync::Arc;
use tokio::sync::mpsc;

use crate::channel::types::*;
use crate::channel::webhook_server::{WebhookHandlerFn, WebhookRequest, WebhookResponse};

use super::api::GoogleChatApi;
use super::jwt::GoogleChatJwtVerifier;

/// Per-process shared JWT verifier (caches Google public keys with 1h TTL).
static JWT_VERIFIER: tokio::sync::OnceCell<Arc<GoogleChatJwtVerifier>> =
    tokio::sync::OnceCell::const_new();

async fn get_jwt_verifier() -> Arc<GoogleChatJwtVerifier> {
    JWT_VERIFIER
        .get_or_init(|| async { Arc::new(GoogleChatJwtVerifier::new()) })
        .await
        .clone()
}

/// Create a webhook handler function for Google Chat events.
///
/// The handler receives incoming HTTP requests from Google Chat's webhook
/// endpoint, **强制**验证 Google 签发的 Bearer JWT，然后解析事件 payload
/// 并通过 `inbound_tx` 转发入站消息。`project_number` 必须与 Google Cloud
/// 项目号一致（JWT aud claim），否则任何能访问隧道 URL 的人都可以伪造事件。
pub fn create_webhook_handler(
    api: Arc<GoogleChatApi>,
    account_id: String,
    project_number: String,
    inbound_tx: mpsc::Sender<MsgContext>,
) -> WebhookHandlerFn {
    // Keep api alive for potential future use (e.g. downloading attachments)
    let _api = api;
    // 空 project_number 直接拒——避免误配置导致鉴权降级
    let project_number = Arc::new(project_number);

    Arc::new(move |req: WebhookRequest| {
        let account_id = account_id.clone();
        let inbound_tx = inbound_tx.clone();
        let project_number = project_number.clone();

        Box::pin(async move {
            if project_number.is_empty() {
                app_warn!(
                    "channel",
                    "googlechat",
                    "Rejecting webhook for '{}': project_number not configured",
                    account_id
                );
                return WebhookResponse {
                    status: 403,
                    body: r#"{"error":"project_number not configured"}"#.to_string(),
                };
            }

            // **必须**验证 Authorization Bearer JWT，否则放任任意伪造事件
            let authz = req
                .headers
                .get("authorization")
                .or_else(|| req.headers.get("Authorization"))
                .map(|s| s.as_str());
            let verifier = get_jwt_verifier().await;
            if let Err(e) = verifier.verify_authz_header(authz, &project_number).await {
                app_warn!(
                    "channel",
                    "googlechat",
                    "Rejected webhook for '{}': JWT verification failed: {}",
                    account_id,
                    e
                );
                return WebhookResponse {
                    status: 403,
                    body: r#"{"error":"unauthorized"}"#.to_string(),
                };
            }

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
                    // Card v2 按钮 onClick.action.function 在事件回调中以
                    // `common.invokedFunction` 字段返回（Google Chat API v1
                    // event schema）；老 Card v1 / FormAction 用
                    // `action.actionMethodName`。优先 v2，fallback v1，避免
                    // 用 cardsV2 发出的审批按钮被点击时静默失败。
                    let action_name = body
                        .pointer("/common/invokedFunction")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            body.pointer("/action/actionMethodName")
                                .and_then(|v| v.as_str())
                        });
                    if let Some(action) = action_name {
                        crate::channel::worker::ask_user::try_dispatch_interactive_callback(
                            action,
                            "googlechat",
                        );
                    }
                }
                other => {
                    app_debug!("channel", "googlechat", "Ignoring event type: {}", other);
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
    let space_type = space.get("type").and_then(|v| v.as_str()).unwrap_or("ROOM");
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
