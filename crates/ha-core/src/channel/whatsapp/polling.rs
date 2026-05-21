use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::channel::types::{ChannelId, ChatType, InboundEvent, MsgContext};
use crate::failover::retry_delay_ms;

use super::api::{BridgeMessage, WhatsAppApi};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_DELAY: Duration = Duration::from_secs(2);
const QUICK_RETRY_FAILURES: usize = 2;
const PERSISTENT_FAILURE_ALERT_THRESHOLD: usize = 3;

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
    account_label: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
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
                if consecutive_failures >= PERSISTENT_FAILURE_ALERT_THRESHOLD {
                    app_info!(
                        "channel",
                        "whatsapp::polling",
                        "WhatsApp polling for '{}' [{}] recovered after {} consecutive failure(s)",
                        account_label,
                        account_id,
                        consecutive_failures
                    );
                }
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
                        if let Err(err) = inbound_tx.send(InboundEvent::Message(ctx)).await {
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
                let delay = polling_retry_delay(consecutive_failures);
                if consecutive_failures >= PERSISTENT_FAILURE_ALERT_THRESHOLD {
                    app_error!(
                        "channel",
                        "whatsapp::polling",
                        "WhatsApp polling has failed {} consecutive time(s) for '{}' [{}]: {} | next retry in {}s",
                        consecutive_failures,
                        account_label,
                        account_id,
                        err,
                        delay.as_secs()
                    );
                } else {
                    app_warn!(
                        "channel",
                        "whatsapp::polling",
                        "WhatsApp polling error for '{}' [{}]: {} | retry in {}s",
                        account_label,
                        account_id,
                        err,
                        delay.as_secs()
                    );
                }
                if sleep_or_cancel(&cancel, delay).await {
                    break;
                }
            }
        }
    }
}

fn polling_retry_delay(consecutive_failures: usize) -> Duration {
    if consecutive_failures <= QUICK_RETRY_FAILURES {
        return RETRY_DELAY;
    }

    let attempt = consecutive_failures
        .saturating_sub(QUICK_RETRY_FAILURES + 1)
        .try_into()
        .unwrap_or(u32::MAX);
    Duration::from_millis(retry_delay_ms(attempt, 30_000, 300_000))
}

/// Convert a bridge message to a normalized MsgContext.
fn convert_bridge_message(account_id: &str, msg: BridgeMessage) -> Option<MsgContext> {
    // Capture raw JSON before moving fields out of msg
    let mut raw = serde_json::to_value(&msg).unwrap_or(serde_json::Value::Null);

    // Parse inbound attachments to deferred refs (download happens in
    // WhatsAppPlugin::materialize_pending_media after gating). Older
    // bridges that don't emit `attachments` produce an empty vec, so
    // this is a safe no-op for backward compat.
    let pending = super::inbound_media::parse_attachments(&msg.attachments);
    crate::channel::inbound_media_common::embed_pending_refs(&mut raw, pending);

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
    use super::{convert_bridge_message, polling_retry_delay};
    use crate::channel::types::ChatType;
    use crate::channel::whatsapp::api::BridgeMessage;
    use std::time::Duration;

    fn assert_delay_between(actual: Duration, min: Duration, max: Duration) {
        assert!(actual >= min, "{actual:?} < {min:?}");
        assert!(actual <= max, "{actual:?} > {max:?}");
    }

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

    #[test]
    fn polling_retry_delay_escalates_after_quick_retries() {
        assert_eq!(polling_retry_delay(1), Duration::from_secs(2));
        assert_eq!(polling_retry_delay(2), Duration::from_secs(2));
        assert_delay_between(
            polling_retry_delay(3),
            Duration::from_secs(27),
            Duration::from_secs(33),
        );
        assert_delay_between(
            polling_retry_delay(4),
            Duration::from_secs(54),
            Duration::from_secs(66),
        );
        assert_delay_between(
            polling_retry_delay(8),
            Duration::from_secs(270),
            Duration::from_secs(330),
        );
    }
}
