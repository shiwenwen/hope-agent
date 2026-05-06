use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::channel::types::{ChannelId, ChatType, MsgContext};

use super::api::{BridgeMessage, WhatsAppApi};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_DELAY: Duration = Duration::from_secs(2);
const BACKOFF_DELAY: Duration = Duration::from_secs(30);

/// 时间戳单位防御：bridge HTTP 协议约定 Unix 秒（UTC），但 WhatsApp 原生
/// （whatsmeow / Baileys）用毫秒。若 ts 大于 4_000_000_000（2096 年）即
/// 视为毫秒自动除 1000，避免 since= 串永远拿不到新消息。
fn normalize_unix_seconds(ts: i64) -> i64 {
    if ts > 4_000_000_000 {
        ts / 1000
    } else {
        ts
    }
}

/// Run the WhatsApp bridge polling loop.
///
/// Follows the same pattern as the WeChat polling loop:
/// - Polls bridge API at a regular interval
/// - Converts messages to MsgContext
/// - Sends via inbound_tx
/// - Exponential backoff on errors
pub(crate) async fn run_whatsapp_polling(
    api: Arc<WhatsAppApi>,
    account_id: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    let mut last_timestamp: i64 = Utc::now().timestamp();
    let mut consecutive_failures: usize = 0;

    app_info!(
        "channel",
        "whatsapp::polling",
        "Started WhatsApp polling for account '{}'",
        account_id
    );

    loop {
        // Wait for the poll interval or cancellation
        if sleep_or_cancel(&cancel, POLL_INTERVAL).await {
            app_info!(
                "channel",
                "whatsapp::polling",
                "WhatsApp polling cancelled for account '{}'",
                account_id
            );
            break;
        }

        let result = api.poll_messages(last_timestamp).await;

        match result {
            Ok(messages) => {
                consecutive_failures = 0;

                for msg in messages {
                    // Skip messages from the bot itself
                    if msg.from_me {
                        continue;
                    }

                    if let Some(ts) = msg.timestamp {
                        let ts = normalize_unix_seconds(ts);
                        if ts > last_timestamp {
                            last_timestamp = ts;
                        }
                    }

                    if let Some(ctx) = convert_bridge_message(&account_id, msg) {
                        if let Err(err) = inbound_tx.send(ctx).await {
                            app_error!(
                                "channel",
                                "whatsapp::polling",
                                "Failed to forward WhatsApp inbound message: {}",
                                err
                            );
                            return;
                        }
                    }
                }
            }
            Err(err) => {
                consecutive_failures += 1;
                app_warn!(
                    "channel",
                    "whatsapp::polling",
                    "WhatsApp polling error for '{}': {}",
                    account_id,
                    err
                );

                let delay = if consecutive_failures >= 3 {
                    consecutive_failures = 0;
                    BACKOFF_DELAY
                } else {
                    RETRY_DELAY
                };
                if sleep_or_cancel(&cancel, delay).await {
                    break;
                }
            }
        }
    }
}

/// Convert a bridge message to a normalized MsgContext.
fn convert_bridge_message(account_id: &str, msg: BridgeMessage) -> Option<MsgContext> {
    // Capture raw JSON before moving fields out of msg
    let raw = serde_json::to_value(&msg).unwrap_or(serde_json::Value::Null);

    let BridgeMessage {
        id,
        chat_id,
        sender_id,
        sender_name,
        text,
        timestamp,
        was_mentioned,
        reply_to,
        chat_title,
        ..
    } = msg;

    let (chat_id, sender_id) = match (chat_id, sender_id) {
        (Some(chat_id), Some(sender_id)) => (chat_id, sender_id),
        _ => return None,
    };

    let message_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let text = text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let timestamp = timestamp
        .map(normalize_unix_seconds)
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now);

    // WhatsApp JID convention:
    // - DM: `<phone>@s.whatsapp.net`
    // - Group: `<groupid>@g.us`
    let chat_type = if chat_id.ends_with("@g.us") {
        ChatType::Group
    } else {
        ChatType::Dm
    };

    Some(MsgContext {
        channel_id: ChannelId::WhatsApp,
        account_id: account_id.to_string(),
        sender_id,
        sender_name,
        sender_username: None,
        chat_id,
        chat_type,
        chat_title,
        thread_id: None,
        message_id,
        text,
        media: Vec::new(),
        reply_to_message_id: reply_to,
        timestamp,
        was_mentioned,
        raw,
    })
}

/// Sleep for the given duration, returning true if cancelled.
async fn sleep_or_cancel(cancel: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::convert_bridge_message;
    use crate::channel::types::ChatType;
    use crate::channel::whatsapp::api::BridgeMessage;

    #[test]
    fn convert_bridge_message_missing_required_fields_returns_none() {
        let missing_chat = BridgeMessage {
            sender_id: Some("sender@s.whatsapp.net".to_string()),
            ..Default::default()
        };
        assert!(convert_bridge_message("acct", missing_chat).is_none());

        let missing_sender = BridgeMessage {
            chat_id: Some("chat@s.whatsapp.net".to_string()),
            ..Default::default()
        };
        assert!(convert_bridge_message("acct", missing_sender).is_none());
    }

    #[test]
    fn convert_bridge_message_accepts_group_chat_without_unwrap() {
        let msg = BridgeMessage {
            id: Some("m1".to_string()),
            chat_id: Some("123@g.us".to_string()),
            sender_id: Some("sender@s.whatsapp.net".to_string()),
            text: Some(" hello ".to_string()),
            ..Default::default()
        };

        let ctx = convert_bridge_message("acct", msg).expect("valid message");
        assert_eq!(ctx.chat_type, ChatType::Group);
        assert_eq!(ctx.message_id, "m1");
        assert_eq!(ctx.text.as_deref(), Some("hello"));
    }
}
