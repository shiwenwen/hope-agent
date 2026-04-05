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

                    // Update last_timestamp to the most recent message
                    if let Some(ts) = msg.timestamp {
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
    // Validate required fields exist before proceeding
    if msg.chat_id.is_none() || msg.sender_id.is_none() {
        return None;
    }

    // Capture raw JSON before moving fields out of msg
    let raw = serde_json::to_value(&msg).unwrap_or(serde_json::Value::Null);

    let chat_id = msg.chat_id.unwrap();
    let sender_id = msg.sender_id.unwrap();
    let message_id = msg.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let text = msg
        .text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let timestamp = msg
        .timestamp
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
        sender_name: msg.sender_name,
        sender_username: None,
        chat_id,
        chat_type,
        chat_title: msg.chat_title,
        thread_id: None,
        message_id,
        text,
        media: Vec::new(),
        reply_to_message_id: msg.reply_to,
        timestamp,
        was_mentioned: msg.was_mentioned,
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
